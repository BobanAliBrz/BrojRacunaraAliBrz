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
   - Extracts binaries to `C:\ProgramData\TaskbarIP\` (or `%LOCALAPPDATA%\TaskbarIP\` fallback).
   - Spawns the main executable, which self-registers for autostart.
4. **User Feedback**: Native Win32 GUI message boxes report success or errors without requiring a CLI.

---

## Persistence & Autostart Mechanism

To prevent "random uninstallation" (which occurred when running directly off SMB shares with flaky network paths or when re-copying over locked binaries), `set_autostart()` in [main.rs](file:///c:/Coding/Skibidi/taskbar-ip/src/main.rs) implements a robust persistence layer:

1. **Already Installed Guard**: If `get_module_path()` is already inside `C:\ProgramData\TaskbarIP` or Startup directories, copy loops are skipped.
2. **Local ProgramData Staging**: When run from an external location (e.g. SMB share, USB stick), it first copies the executable to local disk (`C:\ProgramData\TaskbarIP\taskbar-ip.exe`) before registering autostart entries.
3. **Dual Persistence Layers**:
   - **Startup Folder**: All-Users (`C:\ProgramData\...\StartUp\`) when elevated; Current-User (`%APPDATA%\...\Startup\`) always.
   - **Registry `Run` Key**: `HKLM\Software\Microsoft\Windows\CurrentVersion\Run` when elevated; `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` always.
4. **Single-Instance Enforcement**: Named Windows Mutex (`Global\TaskbarIP_SingleInstance`) prevents duplicate taskbar windows if both registry and startup folder launch the app simultaneously.

---

## Project Structure & Build

```
taskbar-ip/
├── .cargo/config.toml  # Static CRT flags for msvc targets
├── build.rs            # Manifest embedding & dist/ placeholder generator
├── build.ps1           # Multi-architecture build pipeline script
├── Cargo.toml          # Package definitions & dependencies
├── src/
│   ├── main.rs         # Taskbar IP app (~270 lines)
│   ├── setup.rs        # Auto-detecting installer (~140 lines)
│   └── uninstall.rs    # Complete uninstaller (~90 lines)
└── dist/               # Build output directory
    ├── setup.exe       # Complete single-file installer (~1.3 MB)
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
