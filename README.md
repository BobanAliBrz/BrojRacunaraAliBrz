# Broj Racunara Ali Brz

> **⚠️ DISCLAIMER: This is a personal project built for my own work use.**
> It is not a commercial product, not polished for public distribution, and comes with no guarantees, support, or warranties. Use at your own risk.

A minimal Windows utility that displays **"Broj Racunara: [IP]"** directly in the Windows 11 taskbar, left of the system notification tray (clock area). Shows your current IPv4 address at a glance — no clicking, no menus, no GUI.

<img width="569" height="47" alt="Screenshot 2026-05-13 140148" src="https://github.com/user-attachments/assets/bb8e8ca5-5558-4828-bbe7-af340dcfaaeb" />


## Features

- **Live IP display** — Shows your active IPv4 in the taskbar, updates every second
- **Smart adapter priority** — Prefers 10.0.x.x over 192.168.x.x over anything else
- **Zero configuration** — Copy and run, that's it
- **Statically linked** — No runtime, no DLLs, no dependencies, 128 KB
- **Flicker-free** — Uses Win32 ownership to stay above the taskbar without z-order fights
- **Auto-start** — Copies itself to the Startup folder on first run
- **Uninstaller included** — Clean removal with `uninstall.exe`

## Requirements

- Windows 7, 10, or 11
- That's it. No .NET, no VC++ redist, nothing.

## Usage

### Install

1. Download `taskbar-ip.exe` (and `uninstall.exe` alongside it)
2. **Run `taskbar-ip.exe` once**
   - Normal user → auto-starts for current user only
   - **Run as administrator** → auto-starts for **all users** on the machine

That's it. The IP shows up in the taskbar immediately and will re-appear after every login.

### Uninstall

Run `uninstall.exe` (run as admin for full cleanup across all users), then delete `uninstall.exe`.

## How It Works

Pure Win32 API in Rust:

1. Creates a popup window **owned by** `Shell_TrayWnd` (the taskbar) — this keeps it above the taskbar without any z-order hacks
2. Renders IP text via a child `STATIC` control with white background
3. Every 1s, polls `GetAdaptersAddresses` for IPv4 and updates if changed
4. Positions itself left of `TrayNotifyWnd` (notification area) using `GetWindowRect`
5. On first run, copies itself to `%APPDATA%\...\Startup` (or `C:\ProgramData\...\StartUp` if run as admin)

The ownership trick (`CreateWindowExW` with `hwndParent = Shell_TrayWnd`) is the key to avoiding flicker — owned popups are always above their owner in Windows window manager, no `SetWindowPos` fighting needed.

## Build

```bash
cargo build --release
```

Outputs `target/release/taskbar-ip.exe` (128 KB) and `target/release/uninstall.exe` (119 KB).

### Preview the overlay during development

To rebuild and launch only the overlay (without building or running `setup.exe`):

```powershell
.\test-overlay.ps1
```

The preview does not install itself, configure autostart, or modify uninstall entries. It stops any
existing TaskbarIP overlay first, so the freshly built version is visible immediately. Close the
preview with:

```powershell
.\test-overlay.ps1 -Stop
```

For double-click testing, use `preview-overlay.bat` to rebuild and start the preview, and
`stop-preview-overlay.bat` to close it.

## Project Structure

```
taskbar-ip/
├── Cargo.toml          # Dependencies (winapi only)
├── src/
│   ├── main.rs         # Main application (~210 lines)
│   └── uninstall.rs    # Uninstaller (~90 lines)
├── project_memory.md   # Dev notes, architecture, problem-solving history
└── .gitignore
```
