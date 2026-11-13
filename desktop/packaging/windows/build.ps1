param(
    [string]$Nsis = 'makensis.exe',
    [string]$CargoAbout = 'cargo-about.exe'
)

$ErrorActionPreference = 'Stop'
$workspace = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$distribution = Join-Path $workspace 'dist'
New-Item -ItemType Directory -Force $distribution | Out-Null

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

    & rtk proxy $Nsis "/DNDM_BIN_DIR=$binaryDirectory" "/DNDM_OUTPUT=$output" "/DNDM_NOTICES=$notices" "/DNDM_VERSION=$version" "/DNDM_VERSION_QUAD=$versionQuad" (Join-Path $PSScriptRoot 'ndm-next.nsi')
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed.' }
    Write-Output "Installer: $output"
} finally {
    Pop-Location
}
