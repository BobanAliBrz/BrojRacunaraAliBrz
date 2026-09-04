# Changelog

All notable changes to TaskbarIP are recorded here. Release tags and source
commits remain the authoritative implementation history.

## [1.2.4] - 2026-09-04

### Fixed

- Taskbar-click / Start-close blackout on Windows 11: measured with composed
  screenshots, every taskbar click and Start close puts `Shell_TrayWnd` above
  the overlay and it stayed hidden until the restore fired (~1 s with the old
  debounce). The overlay now restores `HWND_TOPMOST` immediately: a
  `SetWinEventHook` foreground/reorder hook posts a coalesced re-check (single
  async `SetWindowPos`, no move/size/repaint), with a 100 ms threshold-1 poll
  as fallback. Verified: bright overlay pixels at +60–100 ms after an empty
  taskbar click and after Start close.
- While the Start/Search panel itself is open its DWM layer paints above all
  classic topmost windows, so the overlay stays hidden until the panel really
  closes (clicking the taskbar/Start button may just swap panels; dismiss
  with Esc or an outside click). Nothing to fight there; the fix makes the
  return instant.
- Tried hosting the Win11 overlay as a `WS_CHILD` of `Shell_TrayWnd` so it
  would compose with the taskbar and stay visible during Start. Abandoned:
  the fullscreen `Windows.UI.Composition.DesktopWindowContentBridge` XAML
  layer paints over foreign classic children even at sibling-top (verified by
  hiding it: overlay instantly bright). Top-level topmost popup remains the
  only working host on Win11.
- Fixed silent auto-update downloads: the request path stripped the
  owner/repo prefix, so GitHub answered 404 "Not Found" and no update ever
  installed (verified live: full 1675264-byte download plus SHA-256 match
  after the fix).
- The updater now requests TLS 1.2/1.3 explicitly, so Windows 7 machines with
  KB3140245 can negotiate with GitHub (stock Win7 offers only TLS 1.0, which
  GitHub rejects).
- A stale shared install now hands off to a newer per-user copy at startup
  instead of re-downloading the release on every login.
- SMB update fallback: when GitHub is unreachable (or its download fails),
  the updater tries the worker-only share
  `\\10.0.135.252\taskbar ip auto update` with the `auto_update_worker`
  account and installs the newest `TaskbarIP_Setup_vX.Y.Z.exe` newer than the
  running build (PSG convention). Share bytes still verify against the GitHub
  digest when metadata is available; otherwise version comparison plus the
  size cap apply. Release process must now also drop a versioned setup copy
  into the worker share.
- Paint hardening (all versions): double-buffer the popup
  (`WS_EX_COMPOSITED`), clip child repaints (`WS_CLIPCHILDREN`), and skip the
  parent background erase (`WM_ERASEBKGND`).
- Keep per-OS z-order behavior intact: unowned `WS_EX_TOPMOST` on Windows 7
  and Windows 11, taskbar-owned without `TOPMOST` on Windows 8/10. The event
  hook and fast poll are Windows 11 only; Windows 7 keeps its 500 ms
  debounced poll, Windows 8/10 keep no polling at all.

## [1.2.3] - 2026-08-24

### Fixed

- On Windows 11, prevent internal Start/task-list layout windows from being
  mistaken for tray-side toolbars after an update. The overlay remains anchored
  immediately left of the notification area.

## [1.2.2] - 2026-08-17

### Fixed

- Dynamic taskbar space detection: replaced fixed offset guessing with active
  detection of all right-docked elements (ReBar bands, Language Bar, Help buttons,
  OEM toolbars, custom deskbands, and docked popups) across Windows 7 through
  Windows 11.
- Resolved overlay collision with the Language Bar and Help button on Windows 7.

## [1.2.1] - 2026-08-14

### Added

- Silent automatic updates from the latest GitHub release, with SHA-256 asset
  verification and a per-user fallback that does not require elevation.

### Fixed

- Keep the overlay visible when the Windows 11 Start menu is open.
- Tightened the Windows 7 tray spacing while retaining language-bar avoidance.
- Eliminated taskbar and Start-menu flicker by avoiding unnecessary overlay
  z-order changes and text redraws.

### Documentation

- Added this changelog and condensed the maintainer reference in
  `project_memory.md`.

## [1.1.0] - 2026-08-12

### Added

- Single-file, architecture-detecting `setup.exe` installer.
- Windows 7 SP1 and 32-bit build support.
- Control Panel / Apps & Features uninstall registration.

### Changed

- Centered overlay text horizontally and vertically.
- Matched rendering and text-measurement fonts, tightened horizontal padding,
  and prevented label wrapping.
- Added a fast, non-installing overlay preview command and double-clickable
  start/stop batch launchers.

### Fixed

- On Windows 7, create the overlay unowned and enforce its topmost z-order so
  it is not layered behind the taskbar.
- Reserved additional space for the Windows 7 language selector.
- SMB-installed autostart uninstall behavior.
- Reliable autostart after user profile directory renames.
- Windows 7 taskbar and language-bar positioning.

## [1.0.2] - 2026-05-13

### Added

- Windows 7 listed as a supported operating system.

## [1.0.1] - 2026-05-13

### Added

- Initial TaskbarIP overlay for displaying the active IPv4 address.

### Changed

- Improved the README description and added a screenshot.
