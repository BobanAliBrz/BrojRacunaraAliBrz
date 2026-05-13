# TaskbarIP — Project Memory

## What It Is

A minimal Rust Windows application that displays "Broj Racunara: [IP]" in the Windows 11 taskbar area, immediately to the left of the system notification tray (clock, volume, network icons). It auto-starts on login and requires zero configuration.

## How It Works

### Core Architecture

Pure Win32 API (no frameworks like eframe/egui). A single popup window owned by the taskbar's `Shell_TrayWnd`, with a child `STATIC` control for text rendering. Every 1 second a timer fires to:

1. Read the current IPv4 address via `GetAdaptersAddresses`
2. Update the static label text if the IP changed
3. Reposition the window to stay left of `TrayNotifyWnd` (the notification area)
4. No z-order management needed (ownership handles this)

### Component Breakdown

#### IP Detection (`get_first_ipv4`)
- Calls `GetAdaptersAddresses` with `AF_INET` for IPv4 addresses
- Iterates all adapters, collects non-loopback (127.x.x.x), non-link-local (169.254.x.x) addresses
- Priority system: `select_ip_by_priority()` picks **10.0.x.x first**, then **192.168.x.x**, then the first available
- Uses manual byte extraction from `sockaddr` structure (bytes at offset 4-7 from `sa_data`)

#### Window Creation & Ownership
- Window class registered via `WNDCLASSW`
- Window created with `WS_EX_TOOLWINDOW` (hides from Alt+Tab) 
- **CRITICAL: The window's OWNER is `Shell_TrayWnd`** — set via the `hwndParent` parameter of `CreateWindowExW`
- This is the key trick that eliminates z-order flicker (see below)

#### Text Rendering
- A child `STATIC` control (`WS_CHILD | WS_VISIBLE | SS_LEFT`) handles text display
- `WM_CTLCOLORSTATIC` handler returns a white brush (white background) and black text
- Label is sized dynamically to fit the text measurement

#### Text Measurement
- `CreateFontW` with Segoe UI, 15px height, weight 600 (bold)
- `GetDC(NULL)` for screen device context
- `DrawTextW` with `DT_CALCRECT | DT_SINGLELINE` to measure exact pixel width
- Window width = text_width + 16px padding, height = 30px
- Height is fixed (30px) because the text height is consistent for the font size

#### Positioning (`find_tray_pos`)
- Finds `Shell_TrayWnd` via `FindWindowW`
- Finds `TrayNotifyWnd` (notification area) via `FindWindowExW`
- Gets the tray's screen rect via `GetWindowRect`
- `x = tray_rect.left - window_width` (positions window just left of the tray)
- `y = tray_rect.top + (tray_height - 30) / 2` (vertically centers in the tray area)
- Falls back to `SPI_GETWORKAREA` if taskbar windows aren't found

#### Autostart (`set_autostart`)
- **Run as admin:** Copies both `taskbar-ip.exe` and `uninstall.exe` to `C:\ProgramData\TaskbarIP\`, then copies `taskbar-ip.exe` to `C:\ProgramData\Microsoft\Windows\Start Menu\Programs\StartUp\` (all users startup folder)
- **Run as normal user:** Copies exe to `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Startup\taskbar-ip.exe` (current user startup folder)
- No registry hacks — just file operations into Startup folders

### Uninstaller (`uninstall.exe`)
- Kills running `taskbar-ip.exe` process via `CreateToolhelp32Snapshot` + `TerminateProcess`
- Removes registry entries from both `HKLM` and `HKCU` under `...\CurrentVersion\Run\TaskbarIP`
- Deletes `taskbar-ip.exe` from its registered location (read from registry)
- Deletes `C:\ProgramData\TaskbarIP\taskbar-ip.exe`
- Deletes from both Startup folders (all-users and current-user)

### Key Files

| File | Purpose |
|---|---|
| `Cargo.toml` | Project config, winapi dependencies |
| `src/main.rs` | Main application (~210 lines) |
| `src/uninstall.rs` | Uninstaller (~90 lines) |
| `target/release/taskbar-ip.exe` | Compiled app (128 KB) |
| `target/release/uninstall.exe` | Compiled uninstaller (119 KB) |

## Difficulties Overcome

### 1. Z-Order Flicker (The Hardest Problem)

**Problem:** The window sits on top of the taskbar, but clicking the taskbar causes a z-order fight. The taskbar uses a higher-level z-band than `WS_EX_TOPMOST`. Any `SetWindowPos` to re-assert topmost causes DWM re-compositing flicker.

**Attempted solutions that failed:**
- `WS_EX_TOPMOST` alone — window hides behind taskbar when clicked
- Periodic `SetWindowPos(hwnd, HWND_TOPMOST, ...)` — causes flicker
- Two-step `NOTOPMOST` → `TOPMOST` trick — works for z-order but the transition causes visible flicker
- `SWP_NOREDRAW` — prevents WM_PAINT but DWM still re-composites
- `SWP_NOSENDCHANGING` — prevents messages but DWM still re-composites
- `WS_EX_LAYERED` + `SetLayeredWindowAttributes` — window invisible (needs `UpdateLayeredWindow` path)
- Double-buffered WM_PAINT — doesn't address the z-order problem
- Child of taskbar via `SetParent` / `WS_CHILD` — invisible (taskbar clips child windows)
- `WS_EX_LAYERED` removed — then flicker returns

**Solution: Taskbar Ownership — `CreateWindowExW(..., hwndParent=Shell_TrayWnd)`**

In Windows, when you create a `WS_POPUP` window and pass `Shell_TrayWnd` as the `hwndParent` parameter, it establishes an **ownership** relationship (not parenting). Owned popup windows are:
- Always displayed **above** their owner in z-order (this is a fundamental Windows window manager invariant)
- NOT clipped by the owner (unlike child windows)
- Cleaned up when the owner is destroyed

This eliminates the z-order fight entirely. No `SetWindowPos` calls for z-order, no flicker, no periodic kicking. The taskbar can be clicked without causing any visual disruption because the window manager naturally keeps owned windows above their owners.

### 2. Text Measurement with DT_CALCRECT

**Problem:** Initial text measurement used `DT_CENTER | DT_VCENTER | DT_SINGLELINE` which didn't expand the RECT properly for measurement.

**Fix:** Use `DT_CALCRECT | DT_SINGLELINE`. `DT_CALCRECT` tells `DrawTextW` to measure the text without drawing it, and `DT_SINGLELINE` ensures a single line (no wrapping). The function modifies the RECT's `right` and `bottom` fields to match the text dimensions.

### 3. STATIC Control Wrapping

**Problem:** When the window was correctly trimmed to text width, the STATIC control wrapped the text to multiple lines because `SS_LEFT` includes word-wrapping behavior.

**Root cause:** The STATIC control's `SS_LEFT` style wraps text. If the control is narrower than the text, it wraps. The fix was ensuring the control width (from `measure_text`) is always sufficient.

### 4. ProgramData vs Program Files

**Problem:** Writing to `C:\Program Files\TaskbarIP\taskbar-ip.exe` failed silently even when running as admin on some systems due to permission quirks.

**Fix:** Use `C:\ProgramData\TaskbarIP\` instead. `ProgramData` is designed for shared application data, has fewer permission restrictions, and is accessible by all users.

### 5. `std::ffi::c_void` vs `winapi::ctypes::c_void`

**Problem:** These are distinct types in Rust. `SystemParametersInfoW` expects `*mut winapi::ctypes::c_void` but casting via `*mut std::ffi::c_void` causes compile errors.

**Fix:** Use `*mut winapi::ctypes::c_void` explicitly in casts, or import `LPVOID` from `winapi::shared::minwindef` which is correctly typed.

### 6. Separate Uninstaller Binary

**Problem:** The uninstaller needed to kill the main process and delete its own exe — which is impossible for a single binary to do cleanly (can't delete yourself while running).

**Fix:** Two separate binaries (`taskbar-ip.exe` and `uninstall.exe`) defined via `[[bin]]` sections in `Cargo.toml`. The uninstaller uses `CreateToolhelp32Snapshot` to enumerate processes, finds `taskbar-ip.exe` by name, terminates it, removes registry entries, and deletes the file.

### 7. Missing `HDC` / `HBITMAP` / `HGDIOBJ` Types

**Problem:** These types are defined in `winapi::um::wingdi` but marked private. They're re-exported from `winapi::shared::windef`.

**Fix:** Import directly from `winapi::shared::windef::{HDC, HBITMAP, HGDIOBJ}`.

## Building

```bash
cd taskbar-ip
cargo build --release
```

Output: `target/release/taskbar-ip.exe` (128 KB) and `target/release/uninstall.exe` (119 KB)

## Deployment

1. Copy **both** `taskbar-ip.exe` AND `uninstall.exe` to the target PC
2. Run `taskbar-ip.exe` once to register autostart
3. (Optional) Run as admin for all-user autostart

## Uninstall

Run `uninstall.exe` (as admin for full cleanup), then delete `uninstall.exe`.

## Dependencies

- **winapi 0.3** — features: `winuser`, `wingdi`, `libloaderapi`, `winreg`, `iphlpapi`, `ws2def`, `tlhelp32`, `handleapi`, `processthreadsapi`, `fileapi`, `errhandlingapi`, `winnt`, `ntdef`
- No runtime, no DLLs, no external libraries — completely statically linked
