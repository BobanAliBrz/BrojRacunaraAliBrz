# TaskbarIP — Project Memory

## What It Is

A minimal Rust Windows application that displays "Broj Racunara: [IP]" in the taskbar area, immediately to the left of the system notification tray (clock, volume, network icons). It auto-starts on login, requires zero configuration, and has zero external dependencies (no .NET, no VC++ Redistributable).

---

## Architecture & Cross-Windows Compatibility

### Target Windows Versions
- **Windows 7 (32-bit & 64-bit)** (SP1 + all updates)
- **Windows 8 / 8.1 (32-bit & 64-bit)**
- **Windows 10 / 11 (32-bit & 64-bit)**

### Toolchain & Runtime Strategy
- **Compiler**: Rust `1.77.2` (the final official release with robust legacy Windows support prior to API requirement bumps).
- **Targets**: `x86_64-pc-windows-msvc` (64-bit) and `i686-pc-windows-msvc` (32-bit).
- **C Runtime**: Statically linked (`-Ctarget-feature=+crt-static`), avoiding VC++ Redistributable dependency.
- **Manifest**: Embedded via `embed-manifest` (`1.4.0`) for DPI awareness, UTF-8 code page, and Common Controls v6.

---

## Single-File Auto-Detecting Installer (`setup.exe`)

Instead of copying raw `.exe` files manually across SMB shares, `dist\setup.exe` acts as a self-extracting, single-file installer:

1. **Embedded Binaries**: Uses `include_bytes!` to embed:
   - `taskbar-ip-x86.exe` & `taskbar-ip-x64.exe`
   - `uninstall-x86.exe` & `uninstall-x64.exe`
2. **Architecture Detection**: Detects if the host OS is 32-bit or 64-bit (via `PROCESSOR_ARCHITEW6432` / `PROCESSOR_ARCHITECTURE` environment checks).
3. **Extraction & Launch**:
   - Stops existing instances via `taskkill`.
   - Extracts binaries to `%ProgramData%\TaskbarIP\` (or `%LOCALAPPDATA%\TaskbarIP\` fallback).
   - Spawns the main executable, which self-registers for autostart and Control Panel uninstallation.
4. **User Feedback**: Native Win32 GUI message boxes report success or errors without requiring a CLI.

---

## Persistence, Control Panel Uninstall, & Win7 Specifics

### 1. Robust Autostart & Profile Independence
- **Dynamic ProgramData Pathing**: Uses `%ProgramData%` to insulate installation from user profile paths (`C:\Users\Username`). Renaming user profile folders will not affect autostart.
- **64-bit Registry Access**: Uses `KEY_WOW64_64KEY` (0x0100) when accessing `HKLM` & `HKCU` `Software\Microsoft\Windows\CurrentVersion\Run` so 32-bit builds on 64-bit OS write directly to native 64-bit registry run keys.
- **User Startup Cleanup**: Removes duplicate startup entries in `%APPDATA%\...\Startup\` when `%ProgramData%` autostart is active.
- **Single-Instance Enforcement**: Named Windows Mutex (`Global\TaskbarIP_SingleInstance`) prevents duplicate taskbar windows if launched multiple times.

### 2. Control Panel / Apps & Features Uninstallation
- Registers in `Software\Microsoft\Windows\CurrentVersion\Uninstall\TaskbarIP` (`HKLM` when admin, `HKCU` when standard user).
- Sets `DisplayName` ("TaskbarIP - Broj Racunara"), `UninstallString`, `DisplayIcon`, `Publisher`, `NoModify`, `NoRepair`, etc.
- Clicking "Uninstall" in Windows Settings ("Apps & Features") or Control Panel ("Programs and Features") launches `uninstall.exe`.

### 3. Windows 7 Z-Order & Language Selector Positioning
- **Z-Order Topmost**: Created with `WS_EX_TOPMOST` and updated via `SetWindowPos(hwnd, HWND_TOPMOST, ...)`, staying above `Shell_TrayWnd` on Windows 7 DWM composition.
- **Language Bar Avoidance**: Detects `CiceroUIWndFrame` (Windows Language Bar). On Windows 7 (`RtlGetVersion` major 6, minor 1), applies an extra left margin offset so the IP text never covers or overlaps the Windows 7 Language Selector ("EN", "SR", etc.).

---

## Project Structure & Build

```
taskbar-ip/
├── .cargo/config.toml  # Static CRT flags for msvc targets
├── build.rs            # Manifest embedding & dist/ placeholder generator
├── build.ps1           # Multi-architecture build pipeline script
├── Cargo.toml          # Package definitions & dependencies
├── src/
│   ├── main.rs         # Taskbar IP app (~450 lines)
│   ├── setup.rs        # Auto-detecting installer (~220 lines)
│   └── uninstall.rs    # Complete uninstaller (~100 lines)
└── dist/               # Build output directory
    ├── setup.exe       # Complete single-file installer (~1.35 MB)
    ├── taskbar-ip-x64.exe
    ├── taskbar-ip-x86.exe
    ├── uninstall-x64.exe
    └── uninstall-x86.exe
```

### Build Commands
To generate the release binaries and installer:
```powershell
powershell -ExecutionPolicy Bypass -File .\build.ps1
```
