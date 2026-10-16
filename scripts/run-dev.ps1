# Build Glyph and (re)start it from a copy, so the next build can always
# replace glyph.exe while the app is running.
#
# Usage (from the repo root):  & .\scripts\run-dev.ps1

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$HOME\.cargo\bin;$env:PATH"
$env:CARGO_TARGET_DIR = Join-Path $root 'target\ui'
$exe = Join-Path $env:CARGO_TARGET_DIR 'debug\glyph.exe'
$run = Join-Path $env:CARGO_TARGET_DIR 'debug\glyph-run.exe'

# An instance started straight from glyph.exe would lock it.
Get-Process glyph -ErrorAction SilentlyContinue | Stop-Process -Force
cargo build -p glyph
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$old = Get-Process glyph-run, glyph, glyph-preview -ErrorAction SilentlyContinue
if ($old) {
    $old | Stop-Process -Force
    $old | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
}
# Windows can hold the image file a moment after the process is gone.
for ($i = 0; $i -lt 50; $i++) {
    try { Copy-Item $exe $run -Force -ErrorAction Stop; break }
    catch { Start-Sleep -Milliseconds 200 }
}
$log = Join-Path (Split-Path -Parent $root) 'scratch-glyph.log'
$err = Join-Path (Split-Path -Parent $root) 'scratch-glyph.err'
Start-Process -FilePath $run -WorkingDirectory $root -RedirectStandardOutput $log -RedirectStandardError $err | Out-Null
Write-Host "glyph started"
