# Measures process creation -> native main window; not network/home-feed readiness.
[CmdletBinding()]
param(
    [string]$Executable = 'target/debug/glyph.exe',
    [ValidateRange(1,20)][int]$Runs = 3
)
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$candidate = if ([IO.Path]::IsPathRooted($Executable)) { $Executable } else { Join-Path $root $Executable }
$exe = (Resolve-Path -LiteralPath $candidate).Path
if (Get-Process -Name 'glyph*' -ErrorAction SilentlyContinue) {
    throw 'Close Glyph before measuring; single-instance forwarding invalidates startup results.'
}
$logs = Join-Path $root 'target/startup-timings'
$null = New-Item -ItemType Directory -Path $logs -Force
$results = for ($i = 1; $i -le $Runs; $i++) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $exe -WorkingDirectory $root -WindowStyle Normal -PassThru -RedirectStandardOutput (Join-Path $logs "run-$i.log") -RedirectStandardError (Join-Path $logs "run-$i.err")
    try {
        do {
            Start-Sleep -Milliseconds 20
            $process.Refresh()
        } while (-not $process.HasExited -and $process.MainWindowHandle -eq 0 -and $timer.Elapsed.TotalSeconds -lt 30)
        $timer.Stop()
        if ($process.HasExited -or $process.MainWindowHandle -eq 0) { throw 'Glyph failed to open a window within 30 seconds.' }
        [pscustomobject]@{ Run = $i; WindowMs = [math]::Round($timer.Elapsed.TotalMilliseconds, 1) }
        Start-Sleep -Seconds 4
    } finally {
        # Only close the process started by this measurement.
        if (-not $process.HasExited) {
            $null = $process.CloseMainWindow()
            if (-not $process.WaitForExit(5000)) { $process.Kill(); $process.WaitForExit() }
        }
        $process.Dispose()
        # Let Windows release the single-instance listener before restarting.
        Start-Sleep -Milliseconds 500
    }
}
$results
$ordered = @($results.WindowMs | Sort-Object)
$middle = [int][math]::Floor($Runs / 2)
$median = if ($Runs % 2) { $ordered[$middle] } else { ($ordered[$middle - 1] + $ordered[$middle]) / 2 }
Write-Host ('Median window time: {0:N1} ms' -f $median)
