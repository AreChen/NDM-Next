param(
    [string]$Nsis = 'makensis.exe',
    [string]$CargoAbout = 'cargo-about.exe',
    [Parameter(Mandatory = $true)]
    [string]$MediaToolDir
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$distribution = Join-Path $workspace 'dist'
New-Item -ItemType Directory -Force $distribution | Out-Null
$mediaRoot = (Resolve-Path -LiteralPath $MediaToolDir).Path
$mediaFiles = @(
    'bin/avcodec-63.dll', 'bin/avdevice-63.dll', 'bin/avfilter-12.dll',
    'bin/avformat-63.dll', 'bin/avutil-61.dll', 'bin/swresample-7.dll',
    'bin/swscale-10.dll', 'bin/ffmpeg.exe', 'bin/ffprobe.exe',
    'source/BUILD-INFO.txt', 'source/build.sh', 'source/COPYING.LGPLv2.1',
    'source/COPYING.GCC-RUNTIME', 'source/COPYING.GPL-3', 'source/LICENSE.md',
    'source/ffmpeg-9.0.2.tar.xz', 'source/ffmpeg-9.0.2.tar.xz.asc', 'source/ffmpeg-devel.asc'
)
$manifest = Join-Path $mediaRoot 'SHA256SUMS'
$seen = @{}
foreach ($line in Get-Content -LiteralPath $manifest) {
    if ($line -notmatch '^([a-f0-9]{64})  ((?:bin|source)/[a-zA-Z0-9._-]+)$') {
        throw 'Invalid media-tool checksum manifest.'
    }
    $expectedHash = $Matches[1]
    $name = $Matches[2]
    if ($name -cnotin $mediaFiles -or $seen.ContainsKey($name)) {
        throw "Unexpected media-tool file: $name"
    }
    $seen[$name] = $true
    $actualHash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $mediaRoot $name)).Hash
    if ($actualHash -ine $expectedHash) { throw "Media-tool checksum mismatch: $name" }
}
if ($seen.Count -ne $mediaFiles.Count) { throw 'Media-tool files are missing.' }
foreach ($directory in @('bin', 'source')) {
    $actual = Get-ChildItem -Force -LiteralPath (Join-Path $mediaRoot $directory)
    if ($actual.Count -ne @($mediaFiles | Where-Object { $_.StartsWith("$directory/") }).Count) {
        throw "Unexpected content in media-tool directory: $directory"
    }
}
$ffmpeg = Join-Path $mediaRoot 'bin/ffmpeg.exe'
$configuration = (& rtk proxy $ffmpeg -hide_banner -buildconf 2>&1 | Out-String)
if ($LASTEXITCODE -ne 0) { throw 'Cannot run the bundled media tool.' }
foreach ($flag in @('--disable-gpl', '--disable-version3', '--disable-nonfree', '--disable-network', '--disable-autodetect', '--enable-shared')) {
    if (-not $configuration.Contains($flag)) { throw "Media-tool build is missing $flag." }
}

Push-Location $workspace
try {
    $metadataText = & rtk proxy cargo +1.100.0 metadata --no-deps --format-version 1 --locked
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataText | ConvertFrom-Json
    $package = $metadata.packages | Where-Object { $_.name -eq 'ndm-desktop' }
    $version = $package.version
    if ($version -notmatch '^([0-9]+)\.([0-9]+)\.([0-9]+)$') {
        throw "Unsupported installer version: $version"
    }
    $versionQuad = "$version.0"
    $binaryDirectory = Join-Path $metadata.target_directory 'release'
    $notices = Join-Path $distribution 'THIRD-PARTY-LICENSES.html'
    $output = Join-Path $distribution "NDM-Next-Setup-$version-x64.exe"

    $compiler = & rtk proxy rustc +1.100.0 -vV
    if ($LASTEXITCODE -ne 0 -or ($compiler -notcontains 'host: x86_64-pc-windows-msvc')) {
        throw 'This installer requires the x86_64-pc-windows-msvc Rust host toolchain.'
    }
    & rtk proxy cargo +1.100.0 build --locked --release
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }

    & rtk proxy $CargoAbout generate --frozen --workspace --target x86_64-pc-windows-msvc --fail --output-file $notices about.hbs
    if ($LASTEXITCODE -ne 0) { throw 'Third-party license generation failed.' }

    & rtk proxy $Nsis "/DNDM_BIN_DIR=$binaryDirectory" "/DNDM_OUTPUT=$output" "/DNDM_NOTICES=$notices" "/DNDM_FFMPEG_DIR=$mediaRoot" "/DNDM_VERSION=$version" "/DNDM_VERSION_QUAD=$versionQuad" (Join-Path $PSScriptRoot 'ndm-next.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed.' }
    Write-Output "Installer: $output"
} finally {
    Pop-Location
}
