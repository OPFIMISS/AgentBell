$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$dist = Join-Path $root 'dist'
New-Item -ItemType Directory -Path $dist -Force | Out-Null
Push-Location $root
try {
    cargo test --all
    cargo build --release
    Copy-Item -LiteralPath (Join-Path $root 'target\release\agentbell.exe') -Destination (Join-Path $dist 'AgentBell.exe') -Force
    $env:ANDROID_HOME = 'D:\Android'
    $env:JAVA_HOME = 'C:\Program Files\Microsoft\jdk-17.0.19.10-hotspot'
    $env:GRADLE_USER_HOME = 'D:\Lunote 2\.toolchains\gradle-home'
    $env:PUB_HOSTED_URL = 'https://pub.flutter-io.cn'
    $flutter = 'D:\Lunote 2\.toolchains\flutter\flutter\bin\flutter.bat'
    & $flutter analyze (Join-Path $root 'mobile')
    Push-Location (Join-Path $root 'mobile')
    try { & $flutter test; & $flutter build apk --release } finally { Pop-Location }
    Copy-Item -LiteralPath (Join-Path $root 'mobile\build\app\outputs\flutter-apk\app-release.apk') -Destination (Join-Path $dist 'AgentBell.apk') -Force
    Get-FileHash -Algorithm SHA256 (Join-Path $dist 'AgentBell.exe'),(Join-Path $dist 'AgentBell.apk') | Format-Table
} finally {
    Pop-Location
}
