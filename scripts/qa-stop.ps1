param([string]$Executable = 'target/debug/rewind.exe', [int]$TestProcessId = 0)
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path -LiteralPath (Join-Path $PSScriptRoot '..')).Path
$testExecutable = Join-Path $projectRoot $Executable
$tests = Get-Process -Name rewind -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $testExecutable -and ($TestProcessId -eq 0 -or $_.Id -eq $TestProcessId) }
foreach ($test in $tests) {
    $children = Get-CimInstance Win32_Process -Filter "ParentProcessId=$($test.Id)" | Where-Object { $_.Name -eq 'ffmpeg.exe' }
    foreach ($child in $children) { Stop-Process -Id $child.ProcessId -ErrorAction SilentlyContinue }
    Stop-Process -Id $test.Id -ErrorAction SilentlyContinue
    Write-Output "Stopped isolated QA process $($test.Id)"
}
