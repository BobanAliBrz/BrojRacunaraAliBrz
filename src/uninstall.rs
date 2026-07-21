#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use winapi::shared::minwindef::{HKEY, DWORD};
use winapi::shared::ntdef::HANDLE;
use winapi::um::winreg::*;
use winapi::um::handleapi::CloseHandle;
use winapi::um::tlhelp32::*;
use winapi::um::processthreadsapi::*;
use winapi::um::fileapi::*;
use winapi::um::winnt::PROCESS_TERMINATE;

const INVALID_HANDLE_VALUE: HANDLE = !0 as HANDLE;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn kill_processes_by_name(name: &str) {
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE { return; }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as DWORD;

        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let exe_name = String::from_utf16_lossy(
                    &entry.szExeFile[..entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(0)]
                );
                if exe_name.eq_ignore_ascii_case(name) {
                    let handle = OpenProcess(PROCESS_TERMINATE, 0, entry.th32ProcessID);
                    if !handle.is_null() {
                        TerminateProcess(handle, 0);
                        CloseHandle(handle);
                    }
                }
                if Process32NextW(snapshot, &mut entry) == 0 { break; }
            }
        }
        CloseHandle(snapshot);
    }
}

fn delete_file_wide(path: &str) {
    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { DeleteFileW(wide.as_ptr()); }
}

fn main() {
    // 1. Kill any running taskbar-ip.exe
    kill_processes_by_name("taskbar-ip.exe");

    // Give processes a moment to terminate
    unsafe {
        winapi::um::synchapi::Sleep(500);
    }

    // 2. Remove registry keys from both HKLM (all users) and HKCU (current user)
    unsafe {
        for &hive in &[HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let mut hkey: HKEY = ptr::null_mut();
            let path = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            // KEY_SET_VALUE = 0x0002 — need write access to delete values
            if RegOpenKeyExW(hive, path.as_ptr(), 0, 0x0002, &mut hkey) == 0 {
                let name = to_wide("TaskbarIP");
                RegDeleteValueW(hkey, name.as_ptr());
                RegCloseKey(hkey);
            }
        }
    }

    // 3. Delete from ProgramData shared location
    delete_file_wide("C:\\ProgramData\\TaskbarIP\\taskbar-ip.exe");
    delete_file_wide("C:\\ProgramData\\TaskbarIP\\uninstall.exe");
    // Try to remove the directory (will only succeed if empty)
    let _ = std::fs::remove_dir("C:\\ProgramData\\TaskbarIP");

    // 4. Delete from all-users Startup folder
    delete_file_wide("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\taskbar-ip.exe");

    // 5. Delete from current-user Startup folder
    if let Ok(appdata) = std::env::var("APPDATA") {
        let user_startup = format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\taskbar-ip.exe", appdata);
        delete_file_wide(&user_startup);
    }
}