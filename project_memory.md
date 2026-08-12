# TaskbarIP Project Memory

## Purpose

TaskbarIP is a small Rust/Win32 application that shows `Broj Racunara: <IPv4>`
immediately left of the Windows notification area. It updates every second,
autostarts after installation, and has no .NET or Visual C++ Redistributable
requirement.

For the record of user-visible and maintenance changes, see [changelog.md](changelog.md).

## Compatibility contract

- Support Windows 7 SP1 through Windows 11, on both 32-bit and 64-bit systems.
- Build with Rust `1.77.2` for legacy Windows compatibility.
- Produce `i686-pc-windows-msvc` and `x86_64-pc-windows-msvc` binaries.
- Statically link the MSVC runtime (`-Ctarget-feature=+crt-static`) so no VC++
  runtime installation is required.
- `build.rs` embeds the application manifest for Common Controls v6, DPI
  awareness, and the UTF-8 code page.

## Overlay behavior

- `src/main.rs` obtains the preferred active IPv4 address and refreshes the
  label once per second.
- The overlay is a non-activating, topmost popup. Its `STATIC` child has a
  white background, centered text, and uses the same Segoe UI font for drawing
  and width measurement. Keep this pairing intact so the text remains centered
  and does not wrap.
- The window is positioned immediately left of `TrayNotifyWnd`.
- A global mutex (`Global\\TaskbarIP_SingleInstance`) prevents duplicates.

### Windows 7 z-order rule

Windows 7 treats a taskbar-owned popup differently from newer Windows
versions: it can render the popup behind `Shell_TrayWnd`. When
`is_windows_7_or_lower()` is true, create the popup with no owner (`NULL`) and
use both `WS_EX_TOPMOST` and `SetWindowPos(..., HWND_TOPMOST, ...)`. Do not
change this to taskbar ownership without testing Windows 7.

The same code reserves an additional 75 px to the left on Windows 7 to avoid
the language selector. It also detects a visible `CiceroUIWndFrame` language
bar for positioning on other supported versions.

## Installation and persistence

`setup.exe` is a self-extracting installer, not merely a launcher:

1. It embeds the x86/x64 app and uninstaller binaries.
2. It detects the host architecture and extracts the correct pair to
   `%ProgramData%\\TaskbarIP` (with a local-app-data fallback).
3. It starts the app; the app configures autostart and the uninstall entry.

The app prefers `%ProgramData%` paths to remain valid if a user profile is
renamed. It writes Run and uninstall registry entries to HKLM when elevated or
HKCU otherwise. `KEY_WOW64_64KEY` ensures a 32-bit build writes to the native
64-bit registry view on 64-bit Windows. The uninstaller removes the installed
files, startup registrations, and uninstall registration.

## Development workflow

| Task | Command or file |
| --- | --- |
| Fast visual preview | `preview-overlay.bat` or `./test-overlay.ps1` |
| Stop preview | `stop-preview-overlay.bat` or `./test-overlay.ps1 -Stop` |
| Full x86/x64 build and installer | `powershell -ExecutionPolicy Bypass -File ./build.ps1` |
| Check the 32-bit target | `cargo +1.77.2 check --target i686-pc-windows-msvc --bin taskbar-ip` |

Preview mode sets `TASKBAR_IP_PREVIEW=1`; this intentionally skips autostart
and uninstall registration. The preview launcher stops any currently running
TaskbarIP process first, including an installed copy.

## Key files

```text
.cargo/config.toml       Static CRT settings for both MSVC targets
build.ps1                Full multi-architecture build and packaging script
test-overlay.ps1         Build/run or stop a non-installing visual preview
preview-overlay.bat      Double-click preview launcher
stop-preview-overlay.bat Double-click preview stop launcher
src/main.rs              Overlay, positioning, autostart, and registration
src/setup.rs             Architecture-detecting self-extracting installer
src/uninstall.rs         Uninstall and cleanup logic
changelog.md             Maintained change history
```
