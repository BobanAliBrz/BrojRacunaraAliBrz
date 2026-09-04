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
- The window position is determined dynamically by detecting all occupied
  taskbar elements (notification area, ReBar bands/deskbands, Language Bar, Help
  buttons, OEM toolbars, and overlapping docked windows) and placing the overlay
  snug in available free space to the left with a clean 6 px gap.
- A global mutex (`Global\\TaskbarIP_SingleInstance`) prevents duplicates.

### Windows 7 z-order and space detection rules

Windows 7 treats a taskbar-owned popup differently from newer Windows
versions: it can render the popup behind `Shell_TrayWnd`. When
`is_windows_7_or_lower()` is true, create the popup with no owner (`NULL`) and
use both `WS_EX_TOPMOST` and `SetWindowPos(..., HWND_TOPMOST, ...)`. Do not
change this to taskbar ownership without testing Windows 7.

The code dynamically inspects `TrayNotifyWnd`, `ReBarWindow32` bands via
`RB_GETBANDCOUNT`/`RB_GETRECT`/`RB_GETBANDINFO` (identifying deskbands and
toolbars such as the Language Bar and Help buttons), taskbar child controls, and
top-level docked windows (`CiceroUIWndFrame`), placing the overlay in the
available free space without guessing or fixed offsets.

Windows 11 also uses an unowned topmost popup. Its Start menu can suppress a
taskbar-owned popup; an independent `WS_EX_TOPMOST` tool window keeps the
overlay visible while Start is open.

Windows 8/10 use a taskbar-owned popup without `WS_EX_TOPMOST`. It stays above
its owner automatically, so no topmost polling runs there and taskbar/Start
focus changes cause no z-order contention.

The overlay refreshes its text and layout only when values change. Paint
flicker is suppressed with `WS_EX_COMPOSITED`, `WS_CLIPCHILDREN`, a `NULL`
background brush, and `WM_ERASEBKGND` returning 1. On Windows 11 a
`SetWinEventHook` foreground/reorder hook (out-of-context, coalesced via
`RECHECK_PENDING` so `WM_TIMER` never starves) plus a 100 ms threshold-1 poll
restores topmost status with a single async `SetWindowPos` as soon as
`Shell_TrayWnd` covers the overlay after a taskbar click or Start close.
Windows 7 keeps a 500 ms 2-hit debounced poll and no hook (untested here).
`WM_WINDOWPOSCHANGING` keeps unowned popups in the topmost band without an
extra `SetWindowPos` round-trip, and `TaskbarCreated` re-asserts topmost after
Explorer restarts.

Measured Win11 limits (composed screenshots, `C:\Users\xanix\AppData\Local\Temp\overlay_*.ps1`
probes): while the Start/Search panel is open its DWM layer covers classic
topmost popups and no restore can win — the overlay returns on close, now
within ~100 ms instead of ~1 s. `taskbar_is_above_overlay()` walks the classic
z-order, which does not include DWM-composited shell panels, so Start-open
coverage is undetectable there by design. Hosting the overlay as a `WS_CHILD`
of `Shell_TrayWnd` was tried and reverted: the fullscreen
`DesktopWindowContentBridge` XAML layer paints over foreign classic children
even at sibling-top (hiding it makes the overlay instantly bright, and kills
tray visuals with it).

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

## Automatic updates

The installed app checks the repository's latest published GitHub release at
startup and every 24 hours. When a newer semantic version is available, it
downloads only the `setup.exe` asset from
`/<owner>/<repo>/releases/download/<tag>/setup.exe` on `github.com` (WinHTTP
follows the CDN redirect by default), validates its GitHub-provided SHA-256
digest and size, and starts it with `--silent-update`. `https_get` requests
TLS 1.2/1.3 explicitly: stock Windows 7 negotiates only TLS 1.0, which GitHub
rejects, so Win7 updates additionally need KB3140245 (and current root certs);
this is untestable here, no Win7 machine available.

Silent updates display no message boxes and never request elevation. They
prefer `%LOCALAPPDATA%\\TaskbarIP`, where the current user can replace files
without administrator rights, then restart the overlay. A standard user cannot
silently overwrite an administrator-owned all-users installation in
`%ProgramData%`; the per-user location is the secure fallback. Because HKLM
Run entries launch before HKCU ones, a stale shared copy would otherwise win
every login and re-download each time — so at startup (before the mutex and
autostart) the app yields to a newer per-user copy recorded in the HKCU
uninstall entry when its exe exists. Preview mode never runs update checks
and never yields.

When GitHub is unreachable or its download fails verification, the updater
falls back to the worker-only LAN share `\\10.0.135.252\taskbar ip auto update`
(read-only) via `WNetAddConnection2W` as `auto_update_worker` (temporary, no
drive letter; embedded read-only LAN credential, same tradeoff as the Print
Spooler Guardian updater). The share is enumeration-hidden from other users
(probing without worker creds yields 1223, never 67). On credential conflict
with the user's own mapping (1219) the existing session is reused. It installs
the newest `TaskbarIP_Setup_vX.Y.Z(.W).exe` newer than the running build;
share bytes verify against the GitHub digest whenever metadata was fetched,
otherwise version comparison plus the 16 MB cap apply (LAN trust). A `Current`
GitHub answer never triggers a blind share install, so a compromised share
cannot push code while GitHub is healthy.

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

## Release deployment rule

Whenever a new GitHub release is created:
1. Build the full installer bundle using `build.ps1` to produce `dist\setup.exe`.
2. Publish the release with `dist\setup.exe` attached to GitHub.
3. Copy the built setup files (`dist\*`) to the network share at:
   `\\10.0.135.252\Ono_Kad\Setup novog racunara\Taskbar IP`
   replacing existing files.
4. Copy `dist\setup.exe` to the worker share
   `\\10.0.135.252\taskbar ip auto update` as
   `TaskbarIP_Setup_v<version>.exe` (PSG-style versioned name) so the SMB
   updater can version-compare it.

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
