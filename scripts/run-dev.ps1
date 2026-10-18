# Build Glyph and (re)start it from a copy, so the next build can always
# replace glyph.exe while the app is running.
#
# Usage: .\scripts\run-dev.ps1 (build + start); add -NoBuild for instant restart.

param([switch]$NoBuild, [switch]$Check, [switch]$Timings)
$ErrorActionPreference = 'Stop'
# Use up to half the logical CPUs, capped at four to bound compiler memory.
# An explicit caller setting takes precedence.
if (-not $env:CARGO_BUILD_JOBS) { $env:CARGO_BUILD_JOBS = [string][Math]::Max(1, [Math]::Min(4, [Environment]::ProcessorCount / 2)) }
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$HOME\.cargo\bin;$env:PATH"
$env:CARGO_TARGET_DIR = Join-Path $root 'target'
$exe = Join-Path $env:CARGO_TARGET_DIR 'debug\glyph.exe'
$run = Join-Path $env:CARGO_TARGET_DIR 'debug\glyph-run.exe'
function Invoke-BuildRetention {
    try { & (Join-Path $PSScriptRoot 'clean-builds.ps1') -KeepProfile debug }
    catch { Write-Warning "Build cleanup failed: $($_.Exception.Message)" }
}


# An instance started straight from glyph.exe would lock it.
if ($Check -and $NoBuild) { throw '-Check and -NoBuild cannot be combined' }
if (-not $NoBuild) {
    $cargoArgs = @($(if ($Check) { 'check' } else { 'build' }), '-p', 'glyph')
    if ($Timings) { $cargoArgs += '--timings' }
    $buildTimer = [Diagnostics.Stopwatch]::StartNew()
    & cargo @cargoArgs
    $buildExit = $LASTEXITCODE
    Write-Host ('Build/check: {0:N2}s' -f $buildTimer.Elapsed.TotalSeconds)
    if ($buildExit -ne 0) { Invoke-BuildRetention; exit $buildExit }
    if ($Check) { Invoke-BuildRetention; return }
}
if (-not (Test-Path -LiteralPath $exe)) { throw "Build first without -NoBuild" }

$old = Get-Process glyph-run, glyph-preview -ErrorAction SilentlyContinue | Where-Object { $_.Path -and $_.Path.StartsWith((Join-Path $root "target\debug\"), [StringComparison]::OrdinalIgnoreCase) }
if ($old) {
    $old | Stop-Process -Force
    $old | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}
# Windows can hold the image file a moment after the process is gone.
$copied = $false
for ($i = 0; $i -lt 50; $i++) {
    try { Copy-Item -LiteralPath $exe -Destination $run -Force -ErrorAction Stop; $copied = $true; break }
    catch { Start-Sleep -Milliseconds 200 }
}
if (-not $copied) { throw 'Could not replace the dev executable; refusing to launch an older copy' }
$log = Join-Path (Split-Path -Parent $root) 'scratch-glyph.log'
$err = Join-Path (Split-Path -Parent $root) 'scratch-glyph.err'
Start-Process -FilePath $run -WorkingDirectory $root -WindowStyle Normal -RedirectStandardOutput $log -RedirectStandardError $err | Out-Null
Write-Host "glyph started"

Invoke-BuildRetention
