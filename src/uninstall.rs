#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use winapi::shared::minwindef::{HKEY, LPBYTE, DWORD};
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

fn get_exe_path_from_registry() -> Option<String> {
    unsafe {
        let mut hkey: HKEY = ptr::null_mut();
        let path = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let ret = RegOpenKeyExW(HKEY_CURRENT_USER, path.as_ptr(), 0, 0x00020000, &mut hkey);
        if ret != 0 { return None; }

        let name = to_wide("TaskbarIP");
        let mut buf = [0u16; 520];
        let mut size: DWORD = (buf.len() * 2) as DWORD;
        let query_ret = RegQueryValueExW(
            hkey,
            name.as_ptr(),
            ptr::null_mut(),
            ptr::null_mut(),
            buf.as_mut_ptr() as LPBYTE,
            &mut size,
        );
        RegCloseKey(hkey);

        if query_ret == 0 && size > 0 {
            let len = (size / 2) as usize - 1;
            Some(String::from_utf16_lossy(&buf[..len]))
        } else {
            None
        }
    }
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

fn main() {
    // 1. Kill any running taskbar-ip.exe
    kill_processes_by_name("taskbar-ip.exe");

    // 2. Read the exe path from registry
    let exe_path = get_exe_path_from_registry();

    // 3. Remove registry key from both HKLM (all users) and HKCU (current user)
    unsafe {
        for &hive in &[HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
            let mut hkey: HKEY = ptr::null_mut();
            let path = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
            if RegOpenKeyExW(hive, path.as_ptr(), 0, 0x00020000, &mut hkey) == 0 {
                let name = to_wide("TaskbarIP");
                RegDeleteValueW(hkey, name.as_ptr());
                RegCloseKey(hkey);
            }
        }
    }

    // 4. Delete the original exe
    if let Some(ref path) = exe_path {
        let wide_path: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            DeleteFileW(wide_path.as_ptr());
        }
    }

    // 5. Also delete shared ProgramData copy
    let shared = "C:\\ProgramData\\TaskbarIP\\taskbar-ip.exe";
    let wide_shared: Vec<u16> = shared.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { DeleteFileW(wide_shared.as_ptr()); }

    // 6. Delete from startup folders
    let all_users = "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\taskbar-ip.exe";
    let wide_all: Vec<u16> = all_users.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe { DeleteFileW(wide_all.as_ptr()); }

    if let Ok(appdata) = std::env::var("APPDATA") {
        let user_startup = format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\taskbar-ip.exe", appdata);
        let wide_user: Vec<u16> = user_startup.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { DeleteFileW(wide_user.as_ptr()); }
    }
}