$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $projectRoot
try {
    rtk proxy npm run build
    if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed' }
    rtk cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Rust build failed' }
    $destination = Join-Path $projectRoot 'dist'
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $projectRoot 'target\release\rewind.exe') -Destination (Join-Path $destination 'Clipago.exe')
    Copy-Item -LiteralPath (Join-Path $projectRoot '../README.md') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $projectRoot '../LICENSE') -Destination $destination
    Copy-Item -LiteralPath (Join-Path $projectRoot 'assets/fonts/OFL.txt') -Destination (Join-Path $destination 'Inter-OFL.txt')
    Write-Output (Join-Path $destination 'Clipago.exe')
} finally {
    Pop-Location
}
