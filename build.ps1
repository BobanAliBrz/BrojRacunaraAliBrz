# build.ps1 — TaskbarIP multi-arch build script
# Builds 32-bit and 64-bit versions, then creates a single setup.exe installer.
#
# Prerequisites:
#   rustup toolchain install 1.77.2
#   rustup target add i686-pc-windows-msvc --toolchain 1.77.2
#   rustup target add x86_64-pc-windows-msvc --toolchain 1.77.2
#
# Usage:
#   .\build.ps1
#
# Output:
#   dist\setup.exe           — Single-file installer (auto-detects 32/64-bit)
#   dist\taskbar-ip-x86.exe  — 32-bit standalone
#   dist\taskbar-ip-x64.exe  — 64-bit standalone
#   dist\uninstall-x86.exe   — 32-bit uninstaller
#   dist\uninstall-x64.exe   — 64-bit uninstaller

$ErrorActionPreference = "Stop"
$toolchain = "1.77.2"

Write-Host "=== TaskbarIP Build ===" -ForegroundColor Cyan
Write-Host ""

# Check toolchain
Write-Host "[1/6] Checking Rust $toolchain toolchain..." -ForegroundColor Yellow
$installed = rustup toolchain list 2>$null | Select-String $toolchain
if (-not $installed) {
    Write-Host "  Installing toolchain $toolchain..."
    rustup toolchain install $toolchain
}

# Check targets
foreach ($target in @("i686-pc-windows-msvc", "x86_64-pc-windows-msvc")) {
    $hasTarget = rustup target list --toolchain $toolchain --installed 2>$null | Select-String $target
    if (-not $hasTarget) {
        Write-Host "  Adding target $target..."
        rustup target add $target --toolchain $toolchain
    }
}

# Create dist directory
$distDir = Join-Path $PSScriptRoot "dist"
if (-not (Test-Path $distDir)) {
    New-Item -ItemType Directory -Path $distDir -Force | Out-Null
}

# Build 64-bit
Write-Host "[2/6] Building 64-bit (x86_64)..." -ForegroundColor Yellow
cargo "+$toolchain" build --release --target x86_64-pc-windows-msvc --bin taskbar-ip --bin uninstall
if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: 64-bit build" -ForegroundColor Red; exit 1 }

# Build 32-bit
Write-Host "[3/6] Building 32-bit (i686)..." -ForegroundColor Yellow
cargo "+$toolchain" build --release --target i686-pc-windows-msvc --bin taskbar-ip --bin uninstall
if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: 32-bit build" -ForegroundColor Red; exit 1 }

# Copy to dist/ (these are used by setup.rs via include_bytes!)
Write-Host "[4/6] Staging binaries..." -ForegroundColor Yellow
Copy-Item "target\x86_64-pc-windows-msvc\release\taskbar-ip.exe" "$distDir\taskbar-ip-x64.exe" -Force
Copy-Item "target\x86_64-pc-windows-msvc\release\uninstall.exe"  "$distDir\uninstall-x64.exe"  -Force
Copy-Item "target\i686-pc-windows-msvc\release\taskbar-ip.exe"   "$distDir\taskbar-ip-x86.exe"  -Force
Copy-Item "target\i686-pc-windows-msvc\release\uninstall.exe"    "$distDir\uninstall-x86.exe"   -Force

Write-Host "  taskbar-ip-x86.exe: $((Get-Item "$distDir\taskbar-ip-x86.exe").Length / 1KB) KB"
Write-Host "  taskbar-ip-x64.exe: $((Get-Item "$distDir\taskbar-ip-x64.exe").Length / 1KB) KB"
Write-Host "  uninstall-x86.exe:  $((Get-Item "$distDir\uninstall-x86.exe").Length / 1KB) KB"
Write-Host "  uninstall-x64.exe:  $((Get-Item "$distDir\uninstall-x64.exe").Length / 1KB) KB"

# Build setup.exe (32-bit, embeds all binaries)
Write-Host "[5/6] Building setup.exe (32-bit installer with embedded binaries)..." -ForegroundColor Yellow
cargo "+$toolchain" build --release --target i686-pc-windows-msvc --bin setup
if ($LASTEXITCODE -ne 0) { Write-Host "FAILED: setup build" -ForegroundColor Red; exit 1 }

Copy-Item "target\i686-pc-windows-msvc\release\setup.exe" "$distDir\setup.exe" -Force

# Summary
Write-Host "[6/6] Build complete!" -ForegroundColor Green
Write-Host ""
Write-Host "Output files in dist\:" -ForegroundColor Cyan
Get-ChildItem $distDir | ForEach-Object {
    $sizeKB = [math]::Round($_.Length / 1KB, 1)
    Write-Host "  $($_.Name)  ($sizeKB KB)"
}
Write-Host ""
Write-Host "Deploy setup.exe to your SMB share. It auto-detects 32/64-bit." -ForegroundColor Green
