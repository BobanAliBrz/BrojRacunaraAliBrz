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
use winapi::um::winnt::{PROCESS_TERMINATE, KEY_SET_VALUE};

const INVALID_HANDLE_VALUE: HANDLE = !0 as HANDLE;
const KEY_WOW64_64KEY: DWORD = 0x0100;

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

fn program_data_dir() -> String {
    std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string())
}

fn main() {
    // 1. Kill any running taskbar-ip.exe
    kill_processes_by_name("taskbar-ip.exe");

    // Brief wait for process handles to release
    unsafe {
        winapi::um::synchapi::Sleep(500);
    }

    // 2. Remove autostart and uninstall registry keys from both HKLM and HKCU
    unsafe {
        let hives = [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER];
        let flags_list = [
            KEY_SET_VALUE | KEY_WOW64_64KEY,
            KEY_SET_VALUE,
        ];

        let run_path = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let uninstall_parent = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall");
        let app_name = to_wide("TaskbarIP");

        for &hive in &hives {
            for &flags in &flags_list {
                // Delete autostart value
                let mut hkey: HKEY = ptr::null_mut();
                if RegOpenKeyExW(hive, run_path.as_ptr(), 0, flags, &mut hkey) == 0 {
                    RegDeleteValueW(hkey, app_name.as_ptr());
                    RegCloseKey(hkey);
                }

                // Delete Control Panel Uninstall key
                let mut hkey_uninst: HKEY = ptr::null_mut();
                if RegOpenKeyExW(hive, uninstall_parent.as_ptr(), 0, flags, &mut hkey_uninst) == 0 {
                    RegDeleteKeyW(hkey_uninst, app_name.as_ptr());
                    RegCloseKey(hkey_uninst);
                }
            }
        }
    }

    // 3. Delete from ProgramData shared location
    let pd = program_data_dir();
    let shared_dir = format!("{}\\TaskbarIP", pd);
    delete_file_wide(&format!("{}\\taskbar-ip.exe", shared_dir));
    delete_file_wide(&format!("{}\\uninstall.exe", shared_dir));
    let _ = std::fs::remove_dir(&shared_dir);

    // 4. Delete from All Users Startup folder
    delete_file_wide(&format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\taskbar-ip.exe", pd));

    // 5. Delete from Current User Startup folder
    if let Ok(appdata) = std::env::var("APPDATA") {
        let user_startup = format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\taskbar-ip.exe", appdata);
        delete_file_wide(&user_startup);
    }
}