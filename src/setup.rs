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
use winapi::shared::minwindef::{DWORD, HKEY};
use winapi::um::winnt::{KEY_SET_VALUE, KEY_WRITE, REG_DWORD, REG_SZ};
use winapi::um::winreg::*;

// Embedded binaries — populated by build.ps1, placeholders otherwise
const TASKBAR_X86: &[u8] = include_bytes!("../dist/taskbar-ip-x86.exe");
const TASKBAR_X64: &[u8] = include_bytes!("../dist/taskbar-ip-x64.exe");
const UNINSTALL_X86: &[u8] = include_bytes!("../dist/uninstall-x86.exe");
const UNINSTALL_X64: &[u8] = include_bytes!("../dist/uninstall-x64.exe");

const CREATE_NO_WINDOW: u32 = 0x08000000;
const KEY_WOW64_64KEY: DWORD = 0x0100;

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

fn program_data_dir() -> String {
    std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string())
}

/// Check if running elevated (admin)
fn is_elevated() -> bool {
    unsafe {
        use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
        use winapi::um::securitybaseapi::GetTokenInformation;
        use winapi::um::winnt::{TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

        let mut token: winapi::shared::ntdef::HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = std::mem::zeroed();
        let mut size: DWORD = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as DWORD,
            &mut size,
        );
        winapi::um::handleapi::CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// Register Uninstall entry in Windows Control Panel / Apps & Features
fn register_uninstall_entry(exe_path: &str, uninst_path: &str, admin: bool) {
    let uninstaller_str = format!("\"{}\"", uninst_path);
    let icon_str = format!("\"{}\"", exe_path);
    let install_dir = std::path::Path::new(exe_path)
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    unsafe {
        let hive = if admin { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER };
        let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\TaskbarIP");

        let flags_list = [
            KEY_SET_VALUE | KEY_WRITE | KEY_WOW64_64KEY,
            KEY_SET_VALUE | KEY_WRITE,
        ];

        for &flags in &flags_list {
            let mut hkey: HKEY = ptr::null_mut();
            let ret = RegCreateKeyExW(
                hive,
                subkey.as_ptr(),
                0,
                ptr::null_mut(),
                0,
                flags,
                ptr::null_mut(),
                &mut hkey,
                ptr::null_mut(),
            );
            if ret == 0 && !hkey.is_null() {
                let set_sz = |name: &str, val: &str| {
                    let name_w = to_wide(name);
                    let val_w = to_wide(val);
                    let len = (val_w.len() * 2) as DWORD;
                    RegSetValueExW(hkey, name_w.as_ptr(), 0, REG_SZ, val_w.as_ptr() as *const u8, len);
                };

                let set_dword = |name: &str, val: u32| {
                    let name_w = to_wide(name);
                    let val_bytes = val.to_ne_bytes();
                    RegSetValueExW(hkey, name_w.as_ptr(), 0, REG_DWORD, val_bytes.as_ptr(), 4);
                };

                set_sz("DisplayName", "TaskbarIP - Broj Racunara");
                set_sz("DisplayVersion", "1.1.0");
                set_sz("Publisher", "TaskbarIP");
                set_sz("UninstallString", &uninstaller_str);
                set_sz("QuietUninstallString", &uninstaller_str);
                set_sz("DisplayIcon", &icon_str);
                set_sz("InstallLocation", &install_dir);
                set_dword("NoModify", 1);
                set_dword("NoRepair", 1);
                set_dword("EstimatedSize", 1024);

                RegCloseKey(hkey);
            }
        }
    }
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
    let admin = is_elevated();
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
    let pd = program_data_dir();
    let install_dir = format!("{}\\TaskbarIP", pd);
    let dir_ok = std::fs::create_dir_all(&install_dir).is_ok();

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

    // Register Control Panel / Apps & Features entry
    register_uninstall_entry(&exe_path, &uninst_path, admin);

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
