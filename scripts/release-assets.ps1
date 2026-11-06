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
$manifestPath = Join-Path $folder 'latest.json'
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 4), [Text.UTF8Encoding]::new($false))
$installer
$signaturePath
$manifestPath
