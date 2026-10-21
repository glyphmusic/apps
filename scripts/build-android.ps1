# Builds the Rust side for Android, packages the APK and (with -Run) installs
# and starts it on the connected device or emulator.
#   ./scripts/build-android.ps1                 # x86_64 (emulator), debug
#   ./scripts/build-android.ps1 -Abi arm64-v8a  # phone
#   ./scripts/build-android.ps1 -Release -Run
#   ./scripts/build-android.ps1 -Abi arm64-v8a -Fast  # release APK, Rust without LTO (test builds)
param(
    [string[]]$Abi = @("x86_64"),
    [switch]$Release,
    [switch]$Fast,
    [switch]$Run
)
if ($Fast) { $Release = $true }
$ErrorActionPreference = "Stop"
$root = Split-Path -Parent $PSScriptRoot
$sdk = if ($env:ANDROID_HOME) { $env:ANDROID_HOME } else { Join-Path $env:LOCALAPPDATA "Android\Sdk" }
if (-not $env:ANDROID_NDK_HOME) {
    $ndk = Get-ChildItem (Join-Path $sdk "ndk") -Directory | Sort-Object Name -Descending | Select-Object -First 1
    $env:ANDROID_NDK_HOME = $ndk.FullName
}
# Gradle 8 runs on JDK <= 24; Android Studio's runtime fits.
$jbr = "C:\Program Files\Android\Android Studio\jbr"
if (Test-Path $jbr) { $env:JAVA_HOME = $jbr }

$android = Join-Path $root "mobile\android"
$jni = Join-Path $android "app\src\main\jniLibs"
$targets = $Abi | ForEach-Object { @("-t", $_) }
Push-Location $root
try {
    # Written out per branch: PowerShell mangles a splatted "--release".
    if ($Fast) {
        & cargo ndk @targets -P 26 -o $jni build -p glyph-android --profile fast
    } elseif ($Release) {
        & cargo ndk @targets -P 26 -o $jni build -p glyph-android --release
    } else {
        & cargo ndk @targets -P 26 -o $jni build -p glyph-android
    }
    if ($LASTEXITCODE -ne 0) { throw "cargo ndk failed" }
} finally { Pop-Location }

Push-Location $android
try {
    $task = if ($Release) { "assembleRelease" } else { "assembleDebug" }
    & .\gradlew.bat $task
    if ($LASTEXITCODE -ne 0) { throw "gradle failed" }
    if ($Run) {
        $adb = Join-Path $sdk "platform-tools\adb.exe"
        $variant = if ($Release) { "release" } else { "debug" }
        $apk = Get-ChildItem "app\build\outputs\apk\$variant\*.apk" | Select-Object -First 1
        & $adb install -r $apk.FullName
        & $adb shell am start -n app.glyph/.MainActivity
    }
} finally { Pop-Location }
