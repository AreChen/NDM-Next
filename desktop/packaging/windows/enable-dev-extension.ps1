param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('(?-i)^[a-p]{32}$')]
    [string]$ExtensionId,

    # Optional for testing; normally read from the current user's Edge registration.
    [string]$ManifestPath
)

$ErrorActionPreference = 'Stop'
$hostNames = @('com.onesdm.host', 'com.ndmnext.host')
$manifests = @(if ($ManifestPath) {
    [pscustomobject]@{ Name = $null; Path = $ManifestPath }
} else {
    foreach ($name in $hostNames) {
        $key = "Registry::HKEY_CURRENT_USER\Software\Microsoft\Edge\NativeMessagingHosts\$name"
        $registration = Get-Item -LiteralPath $key -ErrorAction SilentlyContinue
        if ($registration) {
            $path = [string]$registration.GetValue('')
            if ($path -and (Test-Path -LiteralPath $path -PathType Leaf)) {
                [pscustomobject]@{ Name = $name; Path = $path }
            }
        }
    }
})
if ($manifests.Count -eq 0) {
    throw 'The OnesDM native host manifest was not found. Install the desktop app first.'
}

$origin = "chrome-extension://$ExtensionId/"
foreach ($registered in $manifests) {
    $manifest = Get-Content -LiteralPath $registered.Path -Raw | ConvertFrom-Json
    if ($manifest.name -cnotin $hostNames -or ($registered.Name -and $manifest.name -cne $registered.Name) -or $manifest.type -ne 'stdio' -or -not $manifest.allowed_origins) {
        throw 'The selected file is not an OnesDM native messaging manifest.'
    }
    if (@($manifest.allowed_origins) -notcontains $origin) {
        $manifest.allowed_origins = @($manifest.allowed_origins) + $origin
        $json = $manifest | ConvertTo-Json -Depth 8
        [IO.File]::WriteAllText($registered.Path, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    }
}
Write-Output "Allowed local extension $ExtensionId. Reload it in the browser."
