# Quickly rebuild and preview the taskbar overlay without creating setup.exe.
# Usage:
#   .\test-overlay.ps1        # rebuild and launch the preview
#   .\test-overlay.ps1 -Stop  # close the active overlay

[CmdletBinding()]
param(
    [switch]$Stop
)

$ErrorActionPreference = "Stop"
$processName = "taskbar-ip"

# The application uses a single-instance mutex, so stop a prior preview or the
# installed copy before launching the freshly built executable.
$running = Get-Process -Name $processName -ErrorAction SilentlyContinue
if ($running) {
    $running | Stop-Process -Force
}

if ($Stop) {
    Write-Host "TaskbarIP overlay stopped." -ForegroundColor Yellow
    exit 0
}

Write-Host "Building TaskbarIP preview..." -ForegroundColor Cyan
cargo build --release --bin taskbar-ip
if ($LASTEXITCODE -ne 0) {
    throw "Preview build failed."
}

$previewExe = Join-Path $PSScriptRoot "target\release\taskbar-ip.exe"
$env:TASKBAR_IP_PREVIEW = "1"
try {
    Start-Process -FilePath $previewExe
} finally {
    Remove-Item Env:TASKBAR_IP_PREVIEW -ErrorAction SilentlyContinue
}

Write-Host "Preview started. Run .\test-overlay.ps1 -Stop to close it." -ForegroundColor Green
