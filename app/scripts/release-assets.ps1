param(
    [string]$Repository = 'greeeenie/clipago'
)

$ErrorActionPreference = 'Stop'
$root = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$config = Get-Content -Raw -LiteralPath (Join-Path $root 'tauri.conf.json') | ConvertFrom-Json
$version = $config.version
$tag = "v$version"
$filename = "Clipago_${version}_x64-setup.exe"
$folder = Join-Path $root 'target/release/bundle/nsis'
$installer = Join-Path $folder $filename
$signaturePath = "$installer.sig"
$signature = (Get-Content -Raw -LiteralPath $signaturePath).Trim()
if (!(Test-Path -LiteralPath $installer -PathType Leaf) -or !$signature) {
    throw 'Expected a signed Windows x64 installer'
}
$uiFilename = "Clipago_$($version)_ui.json"
$ui = Join-Path $folder $uiFilename
node (Join-Path $PSScriptRoot 'ui-assets.mjs') pack $ui
if ($LASTEXITCODE -ne 0) { throw 'Could not package the interface' }
$signingKey = $env:TAURI_SIGNING_PRIVATE_KEY
try {
    if ([IO.File]::Exists($signingKey)) {
        $env:TAURI_SIGNING_PRIVATE_KEY = (Get-Content -Raw -LiteralPath $signingKey).Trim()
    }
    npm.cmd --prefix $root run tauri -- signer sign --app-version $version $ui | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'Could not sign the interface package' }
} finally {
    $env:TAURI_SIGNING_PRIVATE_KEY = $signingKey
}
$uiSignature = (Get-Content -Raw -LiteralPath "$ui.sig").Trim()
$hostId = Get-Content -Raw -LiteralPath (Join-Path $root 'web-dist/clipago-ui-host.txt')

$manifest = [ordered]@{
    version = $version
    pub_date = [DateTimeOffset]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
    platforms = @{
        'windows-x86_64' = @{
            url = "https://github.com/$Repository/releases/download/$tag/$filename"
            signature = $signature
        }
    }
}
$manifest.platforms["clipago-ui-$hostId"] = @{
    url = "https://github.com/$Repository/releases/download/$tag/$uiFilename"
    signature = $uiSignature
}
$manifestPath = Join-Path $folder 'latest.json'
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
$installer
$signaturePath
$ui
$manifestPath
