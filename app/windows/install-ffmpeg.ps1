param(
    [Parameter(Mandatory = $true)]
    [string]$InstallDir
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
$managedBin = Join-Path $InstallDir 'bin/ffmpeg'
$configFile = Join-Path $env:APPDATA 'Rewind/Rewind/config/settings.json'
$settings = $null
if (Test-Path -LiteralPath $configFile -PathType Leaf) {
    try {
        $settings = Get-Content -LiteralPath $configFile -Raw -Encoding UTF8 | ConvertFrom-Json
    } catch {
        Write-Host 'Could not read the previous Clipago settings.'
    }
}

$missing = @(
    foreach ($name in 'ffmpeg', 'ffprobe') {
        $configured = $settings.$name
        if ($configured -and (Test-Path -LiteralPath $configured -PathType Leaf)) { continue }

        $exe = "$name.exe"
        $found = @($InstallDir, (Join-Path $InstallDir 'bin'), $managedBin) | Where-Object {
            Test-Path -LiteralPath (Join-Path $_ $exe) -PathType Leaf
        }
        if (!$found -and !(Get-Command -Name $exe -CommandType Application -ErrorAction SilentlyContinue)) {
            $exe
        }
    }
)

if ($missing.Count -eq 0) { exit 0 }

# Versioned release and SHA-256 from Gyan's FFmpeg builds.
$archiveName = 'ffmpeg-9.0.2-essentials_build'
$url = "https://github.com/GyanD/codexffmpeg/releases/download/9.0.2/$archiveName.zip"
$sha256 = '60f467265b1e312373dbcd92200c2618a74850f98d3d078e94296bb3fa2047ba'
$archive = Join-Path $PSScriptRoot "$archiveName.zip"

try {
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    Invoke-WebRequest -Uri $url -UseBasicParsing -TimeoutSec 120 -OutFile $archive
    $hash = [Security.Cryptography.SHA256]::Create()
    $stream = [IO.File]::OpenRead($archive)
    try {
        $actual = [BitConverter]::ToString($hash.ComputeHash($stream)).Replace('-', '')
    } finally {
        $stream.Dispose()
        $hash.Dispose()
    }
    if ($actual -ne $sha256) {
        throw 'The FFmpeg download did not match the expected SHA-256.'
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    try {
        foreach ($exe in $missing) {
            $entry = $zip.GetEntry("$archiveName/bin/$exe")
            if ($null -eq $entry) { throw "The FFmpeg archive is missing $exe." }
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entry, (Join-Path $PSScriptRoot $exe), $true)
        }
    } finally {
        $zip.Dispose()
    }

    New-Item -ItemType Directory -Path $managedBin -Force | Out-Null
    foreach ($exe in $missing) {
        Move-Item -LiteralPath (Join-Path $PSScriptRoot $exe) -Destination (Join-Path $managedBin $exe)
    }
} finally {
    Remove-Item -LiteralPath $archive -Force -ErrorAction SilentlyContinue
    foreach ($exe in $missing) {
        Remove-Item -LiteralPath (Join-Path $PSScriptRoot $exe) -Force -ErrorAction SilentlyContinue
    }
}
