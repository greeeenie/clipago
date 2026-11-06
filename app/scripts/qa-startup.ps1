param([string]$Executable = 'target/release/rewind.exe')
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$exe = (Resolve-Path -LiteralPath (Join-Path $projectRoot $Executable)).Path
$profile = Join-Path $projectRoot 'target/qa-startup/profile'
$clips = Join-Path $profile 'clips'
New-Item -ItemType Directory -Path $clips -Force | Out-Null
$settings = @{ buffer_seconds=30; fps=30; monitor=0; encoder='Auto'; output_dir=$clips; ffmpeg='ffmpeg'; ffprobe='ffprobe'; shortcut='CtrlAltS' }
[IO.File]::WriteAllText((Join-Path $profile 'settings.json'), ($settings | ConvertTo-Json), (New-Object Text.UTF8Encoding $false))
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$before = (Get-ItemProperty -LiteralPath $runKey -Name Rewind -ErrorAction SilentlyContinue).Rewind
$oldProfile = $env:REWIND_CONFIG_DIR
$appProcess = $null
Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class RewindStartupWindows {
    private delegate bool Callback(IntPtr window, IntPtr data);
    [DllImport("user32.dll")] private static extern bool EnumWindows(Callback callback, IntPtr data);
    [DllImport("user32.dll")] private static extern uint GetWindowThreadProcessId(IntPtr window, out uint process);
    [DllImport("user32.dll")] private static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] private static extern int GetWindowText(IntPtr window, StringBuilder text, int count);
    public static bool EditorVisible(int owner) {
        bool found = false;
        EnumWindows((window, data) => {
            uint process;
            GetWindowThreadProcessId(window, out process);
            if (process == owner && IsWindowVisible(window)) {
                var title = new StringBuilder(128);
                GetWindowText(window, title, title.Capacity);
                if (title.ToString() == "Clipago") found = true;
            }
            return true;
        }, IntPtr.Zero);
        return found;
    }
}
'@
function Find-Capture([int]$ownerId) {
    Get-CimInstance Win32_Process -Filter "ParentProcessId=$ownerId" | Where-Object { $_.Name -eq 'ffmpeg.exe' -and $_.CommandLine -like '*pipe:1*' -and $_.CommandLine -like '*mpegts*' } | Select-Object -First 1
}
function Wait-Capture([int]$ownerId, [int]$excludeId = 0) {
    $deadline = [DateTime]::UtcNow.AddSeconds(35)
    do {
        $capture = Find-Capture $ownerId
        if ($capture -and $capture.ProcessId -ne $excludeId) {
            Start-Sleep -Seconds 2
            $confirmed = Find-Capture $ownerId
            if ($confirmed -and $confirmed.ProcessId -eq $capture.ProcessId) { return $confirmed }
        }
        Start-Sleep -Milliseconds 500
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'No stable background capture process'
}
try {
    $env:REWIND_CONFIG_DIR = $profile
    $appProcess = Start-Process -FilePath $exe -ArgumentList '--background' -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru
    $capture = Wait-Capture $appProcess.Id
    $appProcess.Refresh()
    if ([RewindStartupWindows]::EditorVisible($appProcess.Id)) { throw 'Startup unexpectedly opened the editor' }
    Write-Output "PASS: hidden startup automatically recording; app=$($appProcess.Id), capture=$($capture.ProcessId)"
    $duplicate = Start-Process -FilePath $exe -ArgumentList '--background' -WorkingDirectory $projectRoot -WindowStyle Hidden -PassThru
    if (-not $duplicate.WaitForExit(5000)) { Stop-Process -Id $duplicate.Id; throw 'Duplicate instance did not exit' }
    if ($duplicate.ExitCode -ne 0) { throw 'Duplicate instance exited with an error' }
    Write-Output 'PASS: duplicate startup exited successfully'
    Stop-Process -Id $capture.ProcessId
    $restarted = Wait-Capture $appProcess.Id $capture.ProcessId
    Write-Output "PASS: capture automatically recovered after FFmpeg was killed; capture=$($restarted.ProcessId)"
    if (@(Get-ChildItem -LiteralPath $clips -File).Count -ne 0) { throw 'Capture wrote a file without save command' }
    Write-Output 'PASS: no video files written without a save command'
    $after = (Get-ItemProperty -LiteralPath $runKey -Name Rewind -ErrorAction SilentlyContinue).Rewind
    if ($before -ne $after) { throw 'Isolated QA modified Windows autostart' }
    Write-Output 'PASS: isolated QA left user autostart unchanged'
} finally {
    if ($appProcess -and -not $appProcess.HasExited) {
        Get-CimInstance Win32_Process -Filter "ParentProcessId=$($appProcess.Id)" | Where-Object { $_.Name -eq 'ffmpeg.exe' } | ForEach-Object { Stop-Process -Id $_.ProcessId -ErrorAction SilentlyContinue }
        Stop-Process -Id $appProcess.Id -ErrorAction SilentlyContinue
    }
    $env:REWIND_CONFIG_DIR = $oldProfile
}
