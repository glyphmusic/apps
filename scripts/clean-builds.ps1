# Retention for this checkout only. Never starts a build.
[CmdletBinding(SupportsShouldProcess=$true)]
param([ValidateSet('debug','fast','release')][string]$KeepProfile='debug', [switch]$Plan)
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
# Keep the selected profile and recently used profiles. Never clean active caches.
$cutoff=[DateTime]::UtcNow.AddHours(-24)
foreach ($profile in @('debug','fast','release')) {
    $dir=Join-Path $target $profile
    if ($profile -eq $KeepProfile -or -not (Test-Path -LiteralPath $dir)) { continue }
    $dir=Assert-OwnedPath $dir
    # Nested writes do not reliably change the directory timestamp on Windows.
    $newest=Get-ChildItem -LiteralPath $dir -Force -File -ErrorAction Stop | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    $stamp=(Get-Item -LiteralPath $dir).LastWriteTimeUtc
    if ($newest -and $newest.LastWriteTimeUtc -gt $stamp) { $stamp=$newest.LastWriteTimeUtc }
    # Cargo check/tests can update only nested artifacts. Inspect the small
    # fingerprint index rather than walking the entire dependency cache.
    $fingerprints=Join-Path $dir '.fingerprint'
    if (Test-Path -LiteralPath $fingerprints) {
        $fingerprints=Assert-OwnedPath $fingerprints
        foreach ($entry in @(Get-ChildItem -LiteralPath $fingerprints -Directory -Force)) {
            if ($entry.Attributes -band [IO.FileAttributes]::ReparsePoint) { continue }
            $invoked=Join-Path $entry.FullName 'invoked.timestamp'
            if (Test-Path -LiteralPath $invoked) {
                $time=(Get-Item -LiteralPath $invoked).LastWriteTimeUtc
                if ($time -gt $stamp) { $stamp=$time }
            }
        }
    }
    if ($stamp -lt $cutoff) { Remove-OldOutput $dir -Tree }
}
$active=Join-Path $target $KeepProfile
if ((Test-Path -LiteralPath $active) -and -not $Plan -and -not $WhatIfPreference) {
    $active=Assert-OwnedPath $active
    [IO.File]::WriteAllText((Join-Path $active '.glyph-last-used'),[DateTime]::UtcNow.ToString('o'))
}
# Stable runtime copy is overwritten in place; obsolete preview copies go away.
Remove-OldOutput (Join-Path $active 'glyph-preview.exe')
# Retain the newest executable for each Cargo hash family, plus recent outputs.
$deps=Join-Path $active 'deps'
if (Test-Path -LiteralPath $deps) {
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
