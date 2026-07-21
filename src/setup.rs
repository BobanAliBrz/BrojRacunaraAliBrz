#![windows_subsystem = "windows"]

//! TaskbarIP Setup — Single-file installer that auto-detects 32/64-bit Windows.
//!
//! This binary embeds both the x86 and x64 versions of taskbar-ip.exe and uninstall.exe.
//! On run, it detects the OS architecture, extracts the appropriate binaries,
//! and launches taskbar-ip.exe (which handles its own autostart registration).
//!
//! Build with build.ps1 — do NOT build directly with `cargo build --bin setup`
//! unless dist/ contains the real binaries.

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::ptr;

// Embedded binaries — populated by build.ps1, placeholders otherwise
const TASKBAR_X86: &[u8] = include_bytes!("../dist/taskbar-ip-x86.exe");
const TASKBAR_X64: &[u8] = include_bytes!("../dist/taskbar-ip-x64.exe");
const UNINSTALL_X86: &[u8] = include_bytes!("../dist/uninstall-x86.exe");
const UNINSTALL_X64: &[u8] = include_bytes!("../dist/uninstall-x64.exe");

const CREATE_NO_WINDOW: u32 = 0x08000000;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn message_box(text: &str, title: &str, flags: u32) {
    let text_w = to_wide(text);
    let title_w = to_wide(title);
    unsafe {
        winapi::um::winuser::MessageBoxW(
            ptr::null_mut(),
            text_w.as_ptr(),
            title_w.as_ptr(),
            flags,
        );
    }
}

/// Detect if the OS is 64-bit.
/// Since setup.exe is compiled as 32-bit, on 64-bit Windows it runs under WOW64.
/// PROCESSOR_ARCHITEW6432 is only set when a 32-bit process runs on 64-bit Windows.
fn is_64bit_os() -> bool {
    if std::env::var("PROCESSOR_ARCHITEW6432").is_ok() {
        return true;
    }
    if let Ok(arch) = std::env::var("PROCESSOR_ARCHITECTURE") {
        let arch_upper = arch.to_uppercase();
        return arch_upper == "AMD64" || arch_upper == "IA64" || arch_upper == "ARM64";
    }
    false
}

fn main() {
    // Sanity check: make sure we have real binaries, not placeholders
    if TASKBAR_X86.len() < 4096 || TASKBAR_X64.len() < 4096 {
        message_box(
            "This setup.exe was not built correctly.\n\nPlease use build.ps1 to build the installer.",
            "TaskbarIP Setup Error",
            0x10, // MB_ICONERROR
        );
        return;
    }

    let is_64 = is_64bit_os();
    let taskbar_bytes = if is_64 { TASKBAR_X64 } else { TASKBAR_X86 };
    let uninstall_bytes = if is_64 { UNINSTALL_X64 } else { UNINSTALL_X86 };

    // Kill any existing taskbar-ip.exe instances (silently, no console window)
    let _ = std::process::Command::new("taskkill")
        .args(&["/f", "/im", "taskbar-ip.exe"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    // Brief wait for process handles to release
    std::thread::sleep(std::time::Duration::from_millis(500));

    // Determine install directory
    let install_dir = "C:\\ProgramData\\TaskbarIP";
    let dir_ok = std::fs::create_dir_all(install_dir).is_ok();

    let (exe_path, uninst_path) = if dir_ok {
        (
            format!("{}\\taskbar-ip.exe", install_dir),
            format!("{}\\uninstall.exe", install_dir),
        )
    } else {
        // Fallback to user-level location
        match std::env::var("LOCALAPPDATA") {
            Ok(local) => {
                let user_dir = format!("{}\\TaskbarIP", local);
                let _ = std::fs::create_dir_all(&user_dir);
                (
                    format!("{}\\taskbar-ip.exe", user_dir),
                    format!("{}\\uninstall.exe", user_dir),
                )
            }
            Err(_) => {
                message_box(
                    "Failed to create install directory.\nTry running as Administrator.",
                    "TaskbarIP Setup Error",
                    0x10,
                );
                return;
            }
        }
    };

    // Write the binaries
    if let Err(e) = std::fs::write(&exe_path, taskbar_bytes) {
        message_box(
            &format!("Failed to write taskbar-ip.exe:\n{}\n\nTry running as Administrator.", e),
            "TaskbarIP Setup Error",
            0x10,
        );
        return;
    }

    if let Err(_) = std::fs::write(&uninst_path, uninstall_bytes) {
        // Non-fatal — uninstaller is optional
    }

    // Launch the installed binary (it will handle autostart registration)
    match std::process::Command::new(&exe_path).spawn() {
        Ok(_) => {
            let arch_str = if is_64 { "64-bit" } else { "32-bit" };
            message_box(
                &format!(
                    "TaskbarIP installed successfully ({}).\n\nThe IP address will now appear in your taskbar.\nIt will start automatically on login.",
                    arch_str
                ),
                "TaskbarIP Setup",
                0x40, // MB_ICONINFORMATION
            );
        }
        Err(e) => {
            message_box(
                &format!("Installed but failed to launch:\n{}", e),
                "TaskbarIP Setup Warning",
                0x30, // MB_ICONWARNING
            );
        }
    }
}
