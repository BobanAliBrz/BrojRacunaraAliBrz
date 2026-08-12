# Changelog

All notable changes to TaskbarIP are recorded here. Release tags and source
commits remain the authoritative implementation history.

## Unreleased

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
