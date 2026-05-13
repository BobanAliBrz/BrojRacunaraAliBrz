# Broj Racunara Ali Brz

> **⚠️ DISCLAIMER: This is a personal project built for my own work use.**
> It is not a commercial product, not polished for public distribution, and comes with no guarantees, support, or warranties. Use at your own risk.

A minimal Windows utility that displays **"Broj Racunara: [IP]"** directly in the Windows 11 taskbar, left of the system notification tray (clock area). Shows your current IPv4 address at a glance — no clicking, no menus, no GUI.

[taskbar-ip.exe]: Screenshot placeholder

## Features

- **Live IP display** — Shows your active IPv4 in the taskbar, updates every second
- **Smart adapter priority** — Prefers 10.0.x.x over 192.168.x.x over anything else
- **Zero configuration** — Copy and run, that's it
- **Statically linked** — No runtime, no DLLs, no dependencies, 128 KB
- **Flicker-free** — Uses Win32 ownership to stay above the taskbar without z-order fights
- **Auto-start** — Copies itself to the Startup folder on first run
- **Uninstaller included** — Clean removal with `uninstall.exe`

## Requirements

- Windows 10 or 11
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
