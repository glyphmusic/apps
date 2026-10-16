# Live preview for development: rebuilds Glyph when sources change and
# restarts the app, but only after a successful build (a broken build keeps
# the last good version running).
#
# Usage (from the repo root):  & .\scripts\dev-watch.ps1
# Stop with Ctrl+C.

$ErrorActionPreference = 'Continue'
$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$env:PATH = "$HOME\.cargo\bin;$env:PATH"
$env:CARGO_TARGET_DIR = Join-Path $root 'target\ui'
$exe = Join-Path $env:CARGO_TARGET_DIR 'debug\glyph.exe'
# The preview runs from a copy, so builds (ours and anyone else's) can always
# overwrite glyph.exe while the preview stays open.
$preview = Join-Path $env:CARGO_TARGET_DIR 'debug\glyph-preview.exe'
$watch = @('src', 'crates', 'assets', 'Cargo.toml')

function Get-Stamp {
    # Only look at files git reports as changed: a full directory walk is
    # slow on Windows while a build is running.
    $changed = git -C $root status --porcelain=v1 -uall -- $watch 2>$null
    $times = foreach ($line in $changed) {
        $path = Join-Path $root $line.Substring(3).Trim('"')
        if (Test-Path $path) { (Get-Item $path).LastWriteTimeUtc.Ticks }
    }
    "$(git -C $root rev-parse HEAD)|$(($times | Measure-Object -Maximum).Maximum)|$($changed.Count)"
}

function Start-Glyph {
    $old = Get-Process glyph-preview -ErrorAction SilentlyContinue
    if ($old) {
        $old | Stop-Process -Force
        $old | Wait-Process -Timeout 10 -ErrorAction SilentlyContinue
    }
    Copy-Item $exe $preview -Force
    Start-Process -FilePath $preview -WorkingDirectory $root | Out-Null
    Write-Host ("[{0:HH:mm:ss}] glyph started" -f (Get-Date)) -ForegroundColor Green
}

function Invoke-Build {
    Write-Host ("[{0:HH:mm:ss}] building..." -f (Get-Date)) -ForegroundColor DarkGray
    cargo build -p glyph 2>&1 | Select-String -Pattern '^(error|warning: unused)|-->|Finished' |
        ForEach-Object { Write-Host $_ }
    return ($LASTEXITCODE -eq 0)
}

$last = $null
while ($true) {
    $stamp = Get-Stamp
    if ($stamp -ne $last) {
        # Wait until edits settle (agents often write several files in a row).
        Start-Sleep -Seconds 3
        if ((Get-Stamp) -ne $stamp) { continue }
        $last = $stamp
        if (Invoke-Build) {
            Start-Glyph
        } else {
            Write-Host ("[{0:HH:mm:ss}] build failed, keeping the last good version" -f (Get-Date)) -ForegroundColor Yellow
            if (-not (Get-Process glyph-preview -ErrorAction SilentlyContinue) -and (Test-Path $preview)) {
                Start-Process -FilePath $preview -WorkingDirectory $root | Out-Null
            }
        }
    }
    Start-Sleep -Seconds 2
}
