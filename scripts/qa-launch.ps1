param([string]$Executable = 'target/debug/rewind.exe')
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$profile = Join-Path $projectRoot 'target/ui-qa-react/profile'
$clips = Join-Path $profile 'clips'
New-Item -ItemType Directory -Path $clips -Force | Out-Null
$config = Join-Path $profile 'settings.json'
if (-not (Test-Path -LiteralPath $config)) {
    @{ buffer_seconds=60; fps=30; monitor=0; encoder='Auto'; output_dir=$clips; ffmpeg='ffmpeg'; ffprobe='ffprobe'; shortcut='CtrlAltS' } |
        ConvertTo-Json | Set-Content -LiteralPath $config -Encoding UTF8
    # Windows PowerShell writes a UTF-8 BOM; serde_json expects plain UTF-8.
    $json = Get-Content -LiteralPath $config -Raw
    [IO.File]::WriteAllText($config, $json, (New-Object Text.UTF8Encoding $false))
    $sample = Get-ChildItem -LiteralPath (Join-Path $env:USERPROFILE 'Videos/Rewind') -Filter '*.mp4' -File -ErrorAction SilentlyContinue | Select-Object -First 1
    if ($sample) { Copy-Item -LiteralPath $sample.FullName -Destination $clips }
}
$env:REWIND_CONFIG_DIR = $profile
Start-Process -FilePath (Join-Path $projectRoot $Executable) -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru | Select-Object Id,ProcessName
