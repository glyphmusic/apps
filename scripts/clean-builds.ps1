# Retention for this checkout only. Never starts a build.
[CmdletBinding(SupportsShouldProcess=$true)]
param([ValidateSet('debug','fast','release')][string]$KeepProfile='debug', [switch]$Plan, [int]$UnusedDays=14, [int]$MaxSizeGB=50)
$ErrorActionPreference='Stop'
$repo=[IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
$target=[IO.Path]::GetFullPath((Join-Path $repo 'target'))
if (-not (Test-Path -LiteralPath $target)) { return }
if ((Get-Item -LiteralPath $target -Force).Attributes -band [IO.FileAttributes]::ReparsePoint) { throw 'Refusing a linked target directory' }
function Test-BuildActive {
    return @((Get-Process cargo,rustc,rust-lld,worker-build -ErrorAction SilentlyContinue)).Count -gt 0
}
if (Test-BuildActive) { Write-Host 'Cleanup deferred: compiler/build is running.'; return }
$running=@()
foreach ($proc in @(Get-Process -Name 'glyph*' -ErrorAction SilentlyContinue)) {
    try { $path=$proc.Path } catch { Write-Host 'Cleanup deferred: cannot inspect a Glyph process.'; return }
    if (-not $path) { Write-Host 'Cleanup deferred: unknown Glyph executable path.'; return }
    $running += [IO.Path]::GetFullPath($path)
}
function Assert-OwnedPath([string]$Path) {
    $full=[IO.Path]::GetFullPath($Path)
    if (-not $full.StartsWith($target+[IO.Path]::DirectorySeparatorChar,[StringComparison]::OrdinalIgnoreCase)) { throw "Outside target: $full" }
    $ancestor=$full
    while ($ancestor -ne $target) {
        $entry=Get-Item -LiteralPath $ancestor -Force
        if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { throw "Refusing linked path: $ancestor" }
        $ancestor=Split-Path -Parent $ancestor
    }
    return $full
}
function Remove-OldOutput {
    [CmdletBinding(SupportsShouldProcess=$true)]
    param([string]$Path,[switch]$Tree)
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $full=Assert-OwnedPath $Path
    $prefix=$full+[IO.Path]::DirectorySeparatorChar
    if (@($running | Where-Object { $_ -eq $full -or $_.StartsWith($prefix,[StringComparison]::OrdinalIgnoreCase) }).Count) { Write-Host "Keeping running output: $full"; return }
    if (Test-BuildActive) { Write-Host 'Cleanup deferred: build started.'; return }
    if ($Tree -and @(Get-ChildItem -LiteralPath $full -Recurse -Force -Attributes ReparsePoint -ErrorAction Stop).Count) { Write-Warning "Keeping tree containing links: $full"; return }
    if ($Plan) { Write-Host "Would remove: $full"; return }
    if ($PSCmdlet.ShouldProcess($full,'Delete obsolete build output')) {
        try {
            if ($Tree) { Remove-Item -LiteralPath $full -Recurse -Force -ErrorAction Stop }
            else { Remove-Item -LiteralPath $full -Force -ErrorAction Stop }
            Write-Host "Removed: $full"
        } catch { Write-Warning "Cleanup incomplete for ${full}: $($_.Exception.Message)" }
    }
}
# Abandoned second target tree: never used by the current scripts.
Remove-OldOutput (Join-Path $target 'ui') -Tree
# Every profile (debug, fast, release, Android targets) keeps its build cache,
# so a profile switch never forces a full rebuild. Only outputs nobody used for
# a while go away: old app executables here, stale artifacts via cargo-sweep.
# -KeepProfile is still accepted from older callers; no profile is deleted.
$cutoff=[DateTime]::UtcNow.AddHours(-24)
$profiles=@()
foreach ($dir in @(Get-ChildItem -LiteralPath $target -Directory -Force)) {
    if ($dir.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
    if (Test-Path -LiteralPath (Join-Path $dir.FullName '.fingerprint')) { $profiles += $dir.FullName; continue }
    # Cross-compiled targets: target\<triple>\<profile>
    foreach ($sub in @(Get-ChildItem -LiteralPath $dir.FullName -Directory -Force)) {
        if ($sub.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
        if (Test-Path -LiteralPath (Join-Path $sub.FullName '.fingerprint')) { $profiles += $sub.FullName }
    }
}
foreach ($profileDir in $profiles) {
    $profileDir=Assert-OwnedPath $profileDir
    # Stable runtime copy is overwritten in place; obsolete preview copies go away.
    Remove-OldOutput (Join-Path $profileDir 'glyph-preview.exe')
    # Retain the newest executable for each Cargo hash family, plus recent outputs.
    $deps=Join-Path $profileDir 'deps'
    if (-not (Test-Path -LiteralPath $deps)) { continue }
    $deps=Assert-OwnedPath $deps
    $executables=@(Get-ChildItem -LiteralPath $deps -File -Filter '*.exe' | Where-Object { $_.Name -match '^glyph(?:[_-][a-z_]+)?-[0-9a-f]+\.exe$' })
    foreach ($group in @($executables | Group-Object { $_.BaseName -replace '-[0-9a-f]+$','' })) {
        foreach ($old in @($group.Group | Sort-Object LastWriteTimeUtc -Descending | Select-Object -Skip 1)) {
            if ($old.LastWriteTimeUtc -lt $cutoff) {
                Remove-OldOutput $old.FullName
                if (-not (Test-Path -LiteralPath $old.FullName)) { Remove-OldOutput ([IO.Path]::ChangeExtension($old.FullName,'.pdb')) }
            }
        }
    }
}
$timings=Join-Path $target 'cargo-timings'
if (Test-Path -LiteralPath $timings) {
    $timings=Assert-OwnedPath $timings
    foreach ($old in @(Get-ChildItem -LiteralPath $timings -File -Filter 'cargo-timing-*.html' | Sort-Object LastWriteTimeUtc -Descending | Select-Object -Skip 5)) { Remove-OldOutput $old.FullName }
}
# Stale artifacts in all profiles, at most once a day (cargo install cargo-sweep):
# outputs of Rust toolchains no longer installed, artifacts Cargo has not read
# for $UnusedDays days (each build reads the fingerprint of every crate it
# uses, even unchanged ones), then the least recently used ones while target
# is above $MaxSizeGB. cargo-sweep only deletes inside target.
$sweepStamp=Join-Path $target '.glyph-sweep'
$dryRun=[bool]($Plan -or $WhatIfPreference)
$due=$dryRun -or -not (Test-Path -LiteralPath $sweepStamp) -or (Get-Item -LiteralPath $sweepStamp).LastWriteTimeUtc -lt [DateTime]::UtcNow.AddDays(-1)
if ($due) {
    $cargoBin=Join-Path $HOME '.cargo\bin'
    if (Test-Path -LiteralPath $cargoBin) { $env:PATH="$cargoBin;$env:PATH" }
    $sweepExe=(Get-Command cargo-sweep -ErrorAction SilentlyContinue | Select-Object -First 1).Source
    if (-not $sweepExe) {
        Write-Warning 'Stale artifact cleanup skipped: cargo-sweep is not installed (cargo install cargo-sweep).'
    } else {
        $dry=@(); if ($dryRun) { $dry=@('--dry-run') }
        $complete=$true
        foreach ($rule in @(@('--installed'), @('--time',"$UnusedDays"), @('--maxsize',"${MaxSizeGB}GB"))) {
            if (Test-BuildActive) { Write-Host 'Cleanup deferred: build started.'; $complete=$false; break }
            # cargo-sweep logs to stderr; Windows PowerShell would turn that into errors.
            $ErrorActionPreference='Continue'
            & $sweepExe sweep @dry @rule $repo 2>&1 | ForEach-Object { Write-Host "$_" }
            $code=$LASTEXITCODE
            $ErrorActionPreference='Stop'
            if ($code -ne 0) { Write-Warning "Cleanup incomplete: cargo sweep $($rule -join ' ') failed (exit $code)"; $complete=$false }
        }
        if ($complete -and -not $dryRun) { [IO.File]::WriteAllText($sweepStamp,[DateTime]::UtcNow.ToString('o')) }
    }
}
