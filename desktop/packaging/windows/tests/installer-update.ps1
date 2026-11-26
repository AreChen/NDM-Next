param(
    [string]$HostExecutable = "$PSScriptRoot\..\..\..\target\debug\ndm-host.exe",
    [string]$Nsis = "$env:LOCALAPPDATA\tauri\NSIS\makensis.exe",
    [switch]$LaunchOnly,
    [switch]$IncludeTimeout
)
$ErrorActionPreference = 'Stop'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ("ndm-installer-" + [guid]::NewGuid().ToString('N'))
$bin = Join-Path $testRoot 'bin'
$media = Join-Path $testRoot 'media'
$data = Join-Path $testRoot 'data'
$install = Join-Path $testRoot 'install'
$hostName = 'com.ndmnext.test' + [guid]::NewGuid().ToString('N')
$legacyHostName = "$hostName.legacy"
$nativeHosts = @{
    $hostName = 'onesdm-native-messaging-host.json'
    $legacyHostName = 'native-messaging-host.json'
}
$nativeHostRoots = @(
    'HKCU:\Software\Google\Chrome\NativeMessagingHosts',
    'HKCU:\Software\Chromium\NativeMessagingHosts',
    'HKCU:\Software\Microsoft\Edge\NativeMessagingHosts'
)
$uninstallKey = 'Software\Microsoft\Windows\CurrentVersion\Uninstall\NDM-Installer-Test-' + [guid]::NewGuid().ToString('N')
New-Item -ItemType Directory -Path $bin,"$media\bin","$media\source",$data -Force | Out-Null
Copy-Item -LiteralPath $HostExecutable -Destination "$bin\ndm-host.exe"
& rtk proxy rustc +1.100.0 "$PSScriptRoot\installer-desktop.rs" -o "$bin\ndm-desktop.exe"
if ($LASTEXITCODE -ne 0) { throw 'Fixture compilation failed' }
Set-Content -LiteralPath "$media\bin\avcodec-63.dll" -Value 'fixture'
Copy-Item -LiteralPath "$bin\ndm-desktop.exe" -Destination "$media\bin\ffmpeg.exe"
Copy-Item -LiteralPath "$bin\ndm-desktop.exe" -Destination "$media\bin\ffprobe.exe"
Set-Content -LiteralPath "$media\SHA256SUMS" -Value 'fixture'
Set-Content -LiteralPath "$media\source\BUILD-INFO.txt" -Value 'fixture'
Set-Content -LiteralPath "$testRoot\notices.html" -Value 'fixture'
Set-Content -LiteralPath "$data\preferences.json" -Value '{"keep":true}'
Set-Content -LiteralPath "$data\tasks.db" -Value 'preserve existing task database'
$setup = Join-Path $testRoot 'NDM-Next-Setup-test-x64.exe'
$defines = @("/DNDM_BIN_DIR=$bin", "/DNDM_OUTPUT=$setup", "/DNDM_NOTICES=$testRoot\notices.html", "/DNDM_FFMPEG_DIR=$media", '/DNDM_TEST_INSTALL', "/DNDM_TEST_DATA_DIR=$data", "/DNDM_HOST_NAME=$hostName", "/DNDM_UNINSTALL_KEY=$uninstallKey")
& rtk proxy $Nsis /V2 @defines "$PSScriptRoot\..\ndm-next.nsi"
if ($LASTEXITCODE -ne 0) { throw 'NSIS compilation failed' }
$namespace = (& rtk proxy "$bin\ndm-host.exe" --installation-namespace --data-dir $data) -join ''
if ($LASTEXITCODE -ne 0 -or -not $namespace.EndsWith('-installing')) { throw 'Invalid fixture namespace' }
$previousNamespace = $env:NDM_TEST_INSTALLATION_NAMESPACE
$previousLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path $testRoot 'profile'
$env:NDM_TEST_INSTALLATION_NAMESPACE = $namespace
$fixtureProcesses = [Collections.Generic.List[Diagnostics.Process]]::new()
function Wait-File([string]$Path) {
    $deadline = [DateTime]::UtcNow.AddSeconds(5)
    while (-not (Test-Path -LiteralPath $Path) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 20 }
    if (-not (Test-Path -LiteralPath $Path)) { throw "Missing fixture marker: $Path" }
}
try {
    & rtk proxy $setup /S /UPDATE "/D=$install"
    if ($LASTEXITCODE -ne 0) { throw 'Silent update failed' }
    Wait-File "$install\launches.log"
    if ((Get-Content -LiteralPath "$install\launches.log") -contains 'guard-held') { throw 'App launched before replacement guard was released' }
    if ((Get-Content -LiteralPath "$install\launches.log") -notcontains 'launched') { throw 'Silent update did not launch app' }
    if ($LaunchOnly) { Write-Output "PASS: silent update launches after releasing guard; artifacts $testRoot"; return }
    $created = $false
    $guard = [Threading.Mutex]::new($false, $namespace, [ref]$created)
    try {
        $before = (Get-FileHash -LiteralPath "$install\ndm-desktop.exe").Hash
        & rtk proxy $setup /S /UPDATE "/D=$install"
        if ($LASTEXITCODE -eq 0) { throw 'Overlapping installer was accepted' }
        if ((Get-FileHash -LiteralPath "$install\ndm-desktop.exe").Hash -ne $before) { throw 'Rejected installer changed desktop executable' }
    } finally { $guard.Dispose() }
    New-Item -Path "HKCU:\$uninstallKey" -Force | Out-Null
    New-ItemProperty -LiteralPath "HKCU:\$uninstallKey" -Name DisplayIcon -Value "$testRoot\unexpected.exe" -PropertyType String | Out-Null
    & rtk proxy $setup /S /UPDATE "/D=$install"
    if ($LASTEXITCODE -eq 0) { throw 'Installer ignored a registered shutdown failure' }
    $old = Join-Path $testRoot 'legacy-custom-install'
    New-Item -ItemType Directory -Path $old -Force | Out-Null
    Copy-Item -LiteralPath "$bin\ndm-desktop.exe" -Destination "$old\ndm-desktop.exe"
    Copy-Item -LiteralPath "$bin\ndm-host.exe" -Destination "$old\ndm-host.exe"
    $unpackedId = 'abcdefghijklmnopabcdefghijklmnop'
    & rtk proxy "$old\ndm-host.exe" --write-manifest $legacyHostName $unpackedId
    if ($LASTEXITCODE -ne 0) { throw 'Could not create previous integration fixture' }
    Set-ItemProperty -LiteralPath "HKCU:\$uninstallKey" -Name DisplayIcon -Value ('"' + $old + '\ndm-desktop.exe",0')
    $env:NDM_TEST_DESKTOP_WAIT = '1'
    $legacyProcess = Start-Process -FilePath (Get-Command rtk).Source -ArgumentList @('proxy', "$old\ndm-desktop.exe") -PassThru -WindowStyle Hidden
    Remove-Item Env:\NDM_TEST_DESKTOP_WAIT
    Wait-File "$old\fixture.ready"
    $fixtureProcess = [Diagnostics.Process]::GetProcessById([int](Get-Content -LiteralPath "$old\fixture.pid"))
    $null = $fixtureProcess.Handle # Retain its handle so cleanup cannot target a reused PID.
    $fixtureProcesses.Add($fixtureProcess)
    & rtk proxy $setup /S /UPDATE "/D=$install"
    if ($LASTEXITCODE -ne 0) { throw 'Update could not close registered custom installation' }
    if (-not $legacyProcess.WaitForExit(5000)) { throw 'Registered desktop did not exit' }
    if (-not (Test-Path -LiteralPath "$old\fixture.shutdown")) { throw 'Registered custom installation did not receive graceful shutdown' }
    foreach ($name in $nativeHosts.Keys) {
        $path = Join-Path $install $nativeHosts[$name]
        $relocatedManifest = Get-Content -LiteralPath $path -Raw | ConvertFrom-Json
        if ($relocatedManifest.name -ne $name -or $relocatedManifest.path -ne "$install\ndm-host.exe") { throw "Incorrect manifest for $name" }
        if ($relocatedManifest.allowed_origins -notcontains "chrome-extension://$unpackedId/") { throw "Relocation lost the previous unpacked extension ID for $name" }
        if ($relocatedManifest.allowed_origins -notcontains 'chrome-extension://caegebfdojcdlkkiaiokeoadefdooamd/') { throw "Relocation lost the default extension ID for $name" }
        foreach ($root in $nativeHostRoots) {
            $registration = Get-Item -LiteralPath "$root\$name"
            if ($registration.GetValue('') -ne $path) { throw "Incorrect native host registration for $name" }
        }
    }
    if ((Get-Content -LiteralPath "$install\launches.log") -contains 'guard-held') { throw 'Relaunch saw active installer guard' }
    if ((Get-Content -LiteralPath "$data\preferences.json") -ne '{"keep":true}') { throw 'Installer changed preferences' }
    if ((Get-Content -LiteralPath "$data\tasks.db") -ne 'preserve existing task database') { throw 'Installer changed task database' }
    if ($IncludeTimeout) {
        $hanging = Join-Path $testRoot 'hanging-custom-install'
        New-Item -ItemType Directory -Path $hanging -Force | Out-Null
        Copy-Item -LiteralPath "$bin\ndm-desktop.exe" -Destination "$hanging\ndm-desktop.exe"
        Set-ItemProperty -LiteralPath "HKCU:\$uninstallKey" -Name DisplayIcon -Value "$hanging\ndm-desktop.exe"
        $env:NDM_TEST_DESKTOP_WAIT = '1'
        $env:NDM_TEST_DESKTOP_HANG = '1'
        $hangingWrapper = Start-Process -FilePath (Get-Command rtk).Source -ArgumentList @('proxy', "$hanging\ndm-desktop.exe") -PassThru -WindowStyle Hidden
        Remove-Item Env:\NDM_TEST_DESKTOP_WAIT
        Remove-Item Env:\NDM_TEST_DESKTOP_HANG
        Wait-File "$hanging\fixture.ready"
        $hangingProcess = [Diagnostics.Process]::GetProcessById([int](Get-Content -LiteralPath "$hanging\fixture.pid"))
        $null = $hangingProcess.Handle
        $fixtureProcesses.Add($hangingProcess)
        $launches = (Get-Content -LiteralPath "$install\launches.log").Count
        & rtk proxy $setup /S /UPDATE "/D=$install"
        if ($LASTEXITCODE -ne 4) { throw 'Graceful timeout did not abort installation distinctly' }
        if ($hangingProcess.HasExited) { throw 'Installer force-killed a desktop after graceful timeout' }
        if ((Get-Content -LiteralPath "$install\launches.log").Count -ne $launches) { throw 'Timed out installation launched another app' }
        if (-not (Test-Path -LiteralPath "$hanging\fixture.shutdown")) { throw 'Timeout fixture did not receive graceful event' }
        $hangingProcess.Kill()
        $hangingProcess.WaitForExit()
        if (-not $hangingWrapper.WaitForExit(5000)) { throw 'Timeout fixture wrapper did not exit' }
    }
    & rtk proxy "$install\Uninstall.exe" /S "_?=$install"
    if ($LASTEXITCODE -ne 0) { throw 'Isolated uninstall failed' }
    if (Test-Path -LiteralPath "$install\ndm-desktop.exe") { throw 'Uninstaller left installed executable' }
    foreach ($name in $nativeHosts.Keys) {
        if (Test-Path -LiteralPath (Join-Path $install $nativeHosts[$name])) { throw "Uninstaller left the manifest for $name" }
        foreach ($root in $nativeHostRoots) {
            if (Test-Path -LiteralPath "$root\$name") { throw "Uninstaller left the registration for $name" }
        }
    }
    if (-not (Test-Path -LiteralPath "$data\preferences.json")) { throw 'Uninstaller removed preferences by default' }
    if (-not (Test-Path -LiteralPath "$data\tasks.db")) { throw 'Uninstaller removed history by default' }
    Write-Output "PASS: launch, guard, registered custom-path shutdown, replacement, data preservation, uninstall; artifacts $testRoot"
} finally {
    $env:NDM_TEST_INSTALLATION_NAMESPACE = $previousNamespace
    $env:LOCALAPPDATA = $previousLocalAppData
    Remove-Item Env:\NDM_TEST_DESKTOP_WAIT -ErrorAction SilentlyContinue
    Remove-Item Env:\NDM_TEST_DESKTOP_HANG -ErrorAction SilentlyContinue
    foreach ($fixtureProcess in $fixtureProcesses) {
        if (-not $fixtureProcess.HasExited) { $fixtureProcess.Kill(); $fixtureProcess.WaitForExit() }
        $fixtureProcess.Dispose()
    }
    foreach ($root in $nativeHostRoots) {
        foreach ($name in $nativeHosts.Keys) {
            $key = "$root\$name"
            if (Test-Path -LiteralPath $key) { Remove-Item -LiteralPath $key -Recurse -Force }
        }
    }
    if (Test-Path -LiteralPath "HKCU:\$uninstallKey") { Remove-Item -LiteralPath "HKCU:\$uninstallKey" -Recurse -Force }
}
