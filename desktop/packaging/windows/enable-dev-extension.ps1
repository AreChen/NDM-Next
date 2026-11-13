param(
    [Parameter(Mandatory = $true)]
    [ValidatePattern('(?-i)^[a-p]{32}$')]
    [string]$ExtensionId,

    # Optional for testing; normally read from the current user's Edge registration.
    [string]$ManifestPath
)

$ErrorActionPreference = 'Stop'
if (-not $ManifestPath) {
    $registration = Get-Item -LiteralPath 'Registry::HKEY_CURRENT_USER\Software\Microsoft\Edge\NativeMessagingHosts\com.ndmnext.host'
    $ManifestPath = [string]$registration.GetValue('')
}
if (-not $ManifestPath -or -not (Test-Path -LiteralPath $ManifestPath -PathType Leaf)) {
    throw 'The NDM Next native host manifest was not found. Install the desktop app first.'
}

$manifest = Get-Content -LiteralPath $ManifestPath -Raw | ConvertFrom-Json
if ($manifest.name -ne 'com.ndmnext.host' -or $manifest.type -ne 'stdio' -or -not $manifest.allowed_origins) {
    throw 'The selected file is not an NDM Next native messaging manifest.'
}
$origin = "chrome-extension://$ExtensionId/"
if (@($manifest.allowed_origins) -notcontains $origin) {
    $manifest.allowed_origins = @($manifest.allowed_origins) + $origin
    $json = $manifest | ConvertTo-Json -Depth 8
    [IO.File]::WriteAllText($ManifestPath, "$json`n", [Text.UTF8Encoding]::new($false))
}
Write-Output "Allowed local extension $ExtensionId. Reload it in the browser."
