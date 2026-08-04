#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use winapi::shared::minwindef::{DWORD, HKEY, LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::shared::ws2def::AF_INET;
use winapi::um::iphlpapi::GetAdaptersAddresses;
use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winnt::{KEY_SET_VALUE, KEY_WRITE, REG_DWORD, REG_SZ};
use winapi::um::winreg::*;
use winapi::um::winuser::*;
use winapi::um::wingdi::*;

static mut HWND_LABEL: HWND = ptr::null_mut();
const PADDING_X: i32 = 16;
const WINDOW_H: i32 = 30;
const MUTEX_NAME: &str = "Global\\TaskbarIP_SingleInstance";
const KEY_WOW64_64KEY: DWORD = 0x0100;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn get_module_path() -> String {
    unsafe {
        let mut buf = [0u16; 1024];
        let len = GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), buf.len() as u32);
        if len == 0 { return String::new(); }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

/// Check if we're running with elevated (admin) privileges
fn is_elevated() -> bool {
    unsafe {
        use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
        use winapi::um::securitybaseapi::GetTokenInformation;
        use winapi::um::winnt::{TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

        let mut token: winapi::shared::ntdef::HANDLE = ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut elevation: TOKEN_ELEVATION = mem::zeroed();
        let mut size: DWORD = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            mem::size_of::<TOKEN_ELEVATION>() as DWORD,
            &mut size,
        );
        winapi::um::handleapi::CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// Normalizes a path to lowercase for comparison
fn normalize_path(p: &str) -> String {
    p.to_lowercase().replace('/', "\\")
}

/// Check if a file exists and has a non-zero size
fn file_exists_and_valid(path: &str) -> bool {
    match std::fs::metadata(path) {
        Ok(m) => m.len() > 0,
        Err(_) => false,
    }
}

/// Get the system ProgramData directory (e.g. C:\ProgramData)
fn program_data_dir() -> String {
    std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string())
}

/// Get the all-users Startup folder path
fn all_users_startup_path() -> String {
    format!(
        "{}\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\taskbar-ip.exe",
        program_data_dir()
    )
}

/// Get the current-user Startup folder path
fn user_startup_path() -> Option<String> {
    std::env::var("APPDATA").ok().map(|appdata| {
        format!(
            "{}\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\taskbar-ip.exe",
            appdata
        )
    })
}

/// Check if we're already running from an installed location
fn is_running_from_install_location(exe_path: &str) -> bool {
    let norm = normalize_path(exe_path);
    let pd = program_data_dir();
    let programdata = normalize_path(&format!("{}\\TaskbarIP\\", pd));
    let startup_all = normalize_path(&format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\", pd));

    if norm.starts_with(&programdata) || norm.starts_with(&startup_all) {
        return true;
    }

    // Check current-user startup
    if let Some(user_startup) = user_startup_path() {
        let user_startup_dir = normalize_path(
            &user_startup[..user_startup.rfind('\\').unwrap_or(0)]
        );
        if norm.starts_with(&user_startup_dir) {
            return true;
        }
    }

    false
}

/// Write a registry Run key for autostart persistence
fn set_registry_autostart(exe_path: &str, admin: bool) {
    unsafe {
        let hive = if admin { HKEY_LOCAL_MACHINE } else { HKEY_CURRENT_USER };
        let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        let name = to_wide("TaskbarIP");
        let value_wide = to_wide(&format!("\"{}\"", exe_path));
        let byte_len = (value_wide.len() * 2) as DWORD;

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
                RegSetValueExW(
                    hkey,
                    name.as_ptr(),
                    0,
                    REG_SZ,
                    value_wide.as_ptr() as *const u8,
                    byte_len,
                );
                RegCloseKey(hkey);
            }
        }
    }
}

/// Register TaskbarIP in Windows Uninstall Registry key for Control Panel / Apps & Features
fn register_uninstall_entry(exe_path: &str, admin: bool) {
    let exe_p = std::path::Path::new(exe_path);
    let uninstaller_path = exe_p.with_file_name("uninstall.exe");
    let uninstaller_str = format!("\"{}\"", uninstaller_path.to_string_lossy());
    let icon_str = format!("\"{}\"", exe_path);
    let install_dir = exe_p.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();

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

fn set_autostart() {
    let exe = get_module_path();
    if exe.is_empty() { return; }

    let admin = is_elevated();
    let pd = program_data_dir();
    let shared_dir = format!("{}\\TaskbarIP", pd);
    let shared_exe = format!("{}\\taskbar-ip.exe", shared_dir);
    let shared_uninstaller = format!("{}\\uninstall.exe", shared_dir);

    // If we're already running from an installed location, don't re-install.
    // Just make sure our persistence & uninstall mechanisms are intact.
    if is_running_from_install_location(&exe) {
        if file_exists_and_valid(&shared_exe) {
            set_registry_autostart(&shared_exe, admin);
            if !admin {
                set_registry_autostart(&shared_exe, false);
            }
            register_uninstall_entry(&shared_exe, admin);
        }
        return;
    }

    // --- First-time install (running from SMB share, USB, desktop, etc.) ---

    let exe_path = std::path::Path::new(&exe);
    let uninstaller_src = exe_path.with_file_name("uninstall.exe");

    let _ = std::fs::create_dir_all(&shared_dir);

    // Copy main exe
    let shared_copy_ok = match std::fs::copy(&exe, &shared_exe) {
        Ok(bytes) => bytes > 0,
        Err(_) => false,
    };

    // Copy uninstaller if available
    if uninstaller_src.exists() {
        let _ = std::fs::copy(&uninstaller_src, &shared_uninstaller);
    }

    if shared_copy_ok && file_exists_and_valid(&shared_exe) {
        if admin {
            let all_startup = all_users_startup_path();
            let _ = std::fs::copy(&shared_exe, &all_startup);
        }

        set_registry_autostart(&shared_exe, admin);
        if !admin {
            set_registry_autostart(&shared_exe, false);
        }
        register_uninstall_entry(&shared_exe, admin);

        // Remove duplicate user profile startup shortcut to avoid duplicate autostart
        // or broken shortcuts when the user folder is renamed.
        if let Some(user_startup) = user_startup_path() {
            if file_exists_and_valid(&user_startup) {
                let _ = std::fs::remove_file(&user_startup);
            }
        }
    } else {
        // Fallback to user-level location if ProgramData is not writable
        if let Some(user_startup) = user_startup_path() {
            if std::fs::copy(&exe, &user_startup).is_ok() {
                set_registry_autostart(&user_startup, false);
                register_uninstall_entry(&user_startup, false);
            }
        }
    }
}

fn select_ip_by_priority(ips: &[(String, u8)]) -> String {
    for (ip, _) in ips {
        let parts: Vec<&str> = ip.split('.').collect();
        if parts.len() == 4 && parts[0] == "10" { return ip.clone(); }
    }
    for (ip, _) in ips {
        let parts: Vec<&str> = ip.split('.').collect();
        if parts.len() == 4 && parts[0] == "192" && parts[1] == "168" { return ip.clone(); }
    }
    ips.first().map(|(ip, _)| ip.clone()).unwrap_or_else(|| "N/A".to_string())
}

fn get_first_ipv4() -> String {
    unsafe {
        let mut size: u32 = 0;
        let family: u32 = AF_INET as u32;
        let _ = GetAdaptersAddresses(family, 0, ptr::null_mut(), ptr::null_mut(), &mut size);
        if size == 0 { return "N/A".to_string(); }
        let layout = std::alloc::Layout::from_size_align_unchecked(size as usize, 8);
        let buf = std::alloc::alloc(layout);
        if buf.is_null() { return "N/A".to_string(); }
        let ret = GetAdaptersAddresses(family, 0, ptr::null_mut(), buf as *mut _, &mut size);
        if ret != 0 { std::alloc::dealloc(buf, layout); return "N/A".to_string(); }
        let mut candidates: Vec<(String, u8)> = Vec::new();
        let mut addr = buf as *mut winapi::um::iptypes::IP_ADAPTER_ADDRESSES;
        while !addr.is_null() {
            if (*addr).OperStatus == 1 {
                let mut unicast = (*addr).FirstUnicastAddress;
                while !unicast.is_null() {
                    let sockaddr_ptr = (*unicast).Address.lpSockaddr;
                    if !sockaddr_ptr.is_null() && (*sockaddr_ptr).sa_family == AF_INET as u16 {
                        let base = sockaddr_ptr as *const u8;
                        let a = *base.offset(4); let b = *base.offset(5);
                        let c = *base.offset(6); let d = *base.offset(7);
                        if a != 0 && a != 127 && a != 169 {
                            candidates.push((format!("{}.{}.{}.{}", a, b, c, d), a));
                        }
                    }
                    unicast = (*unicast).Next;
                }
            }
            addr = (*addr).Next;
        }
        std::alloc::dealloc(buf, layout);
        select_ip_by_priority(&candidates)
    }
}

/// Detect if running on Windows 7 or earlier
fn is_windows_7_or_lower() -> bool {
    unsafe {
        #[repr(C)]
        #[allow(non_snake_case)]
        struct OSVERSIONINFOEXW {
            dwOSVersionInfoSize: DWORD,
            dwMajorVersion: DWORD,
            dwMinorVersion: DWORD,
            dwBuildNumber: DWORD,
            dwPlatformId: DWORD,
            szCSDVersion: [u16; 128],
            wServicePackMajor: u16,
            wServicePackMinor: u16,
            wSuiteMask: u16,
            wProductType: u8,
            wReserved: u8,
        }
        type RtlGetVersionFn = unsafe extern "system" fn(*mut OSVERSIONINFOEXW) -> i32;

        let ntdll = GetModuleHandleW(to_wide("ntdll.dll").as_ptr());
        if !ntdll.is_null() {
            let proc_name = std::ffi::CString::new("RtlGetVersion").unwrap();
            let proc = winapi::um::libloaderapi::GetProcAddress(ntdll, proc_name.as_ptr());
            if !proc.is_null() {
                let rtl_get_version: RtlGetVersionFn = std::mem::transmute(proc);
                let mut osvi: OSVERSIONINFOEXW = std::mem::zeroed();
                osvi.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOEXW>() as DWORD;
                if rtl_get_version(&mut osvi) == 0 {
                    // Windows 7 is Major 6, Minor 1. Windows Vista is Major 6, Minor 0.
                    return osvi.dwMajorVersion < 6 || (osvi.dwMajorVersion == 6 && osvi.dwMinorVersion <= 1);
                }
            }
        }
        false
    }
}

fn find_tray_pos(window_w: i32) -> (i32, i32) {
    unsafe {
        let mut work_area: RECT = mem::zeroed();
        SystemParametersInfoW(0x0030, 0, &mut work_area as *mut _ as *mut winapi::ctypes::c_void, 0);

        let taskbar = FindWindowW(to_wide("Shell_TrayWnd").as_ptr(), ptr::null_mut());
        if taskbar.is_null() { return (work_area.right - window_w, work_area.bottom - 30); }

        let tray = FindWindowExW(taskbar, ptr::null_mut(), to_wide("TrayNotifyWnd").as_ptr(), ptr::null_mut());
        if tray.is_null() { return (work_area.right - window_w, work_area.bottom - 30); }

        let mut tray_rc: RECT = mem::zeroed();
        if GetWindowRect(tray, &mut tray_rc) == 0 {
            return (work_area.right - window_w, work_area.bottom - 30);
        }

        let mut min_left = tray_rc.left;

        // Check if Language Bar (CiceroUIWndFrame) is visible and docked next to the tray
        let lang_bar = FindWindowW(to_wide("CiceroUIWndFrame").as_ptr(), ptr::null_mut());
        if !lang_bar.is_null() && IsWindowVisible(lang_bar) != 0 {
            let mut lang_rc: RECT = mem::zeroed();
            if GetWindowRect(lang_bar, &mut lang_rc) != 0 {
                // Check if language bar is on the taskbar area vertically
                if lang_rc.top >= tray_rc.top - 10 && lang_rc.bottom <= tray_rc.bottom + 10 {
                    if lang_rc.left < min_left && lang_rc.left > 0 {
                        min_left = lang_rc.left;
                    }
                }
            }
        }

        // On Windows 7 or lower, add an extra 75px margin to the left
        // to avoid covering the Windows 7 language selector ("EN", "SR", etc.) or tray edge
        if is_windows_7_or_lower() {
            min_left -= 75;
        } else if !lang_bar.is_null() && IsWindowVisible(lang_bar) != 0 {
            min_left -= 12;
        }

        let y = tray_rc.top + (tray_rc.bottom - tray_rc.top - 30) / 2;
        (min_left - window_w, y)
    }
}

fn measure_text(text: &str) -> i32 {
    unsafe {
        let hdc = GetDC(ptr::null_mut());
        let font = CreateFontW(15, 0, 0, 0, 600, 0, 0, 0, 0, 0, 0, 0, 0, to_wide("Segoe UI").as_ptr());
        let old_font = SelectObject(hdc, font as *mut _);
        let mut rc: RECT = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        DrawTextW(hdc, wide.as_ptr(), wide.len() as i32 - 1, &mut rc, DT_CALCRECT | DT_SINGLELINE);
        SelectObject(hdc, old_font);
        DeleteObject(font as *mut _);
        ReleaseDC(ptr::null_mut(), hdc);
        rc.right - rc.left
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let label = CreateWindowExW(0, to_wide("STATIC").as_ptr(), to_wide("...").as_ptr(),
                WS_CHILD | WS_VISIBLE | SS_LEFT, 0, 0, 100, WINDOW_H, hwnd,
                ptr::null_mut(), GetModuleHandleW(ptr::null_mut()), ptr::null_mut());
            HWND_LABEL = label;
            SetTimer(hwnd, 1, 1000, None);
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wp as winapi::shared::windef::HDC;
            SetBkColor(hdc, 0x00FFFFFF);
            SetTextColor(hdc, 0);
            GetStockObject(WHITE_BRUSH as i32) as LRESULT
        }
        WM_TIMER => {
            let ip = get_first_ipv4();
            let text = format!("Broj Racunara: {}", ip);
            let tw = measure_text(&text);
            let w = tw + PADDING_X;
            let (x, y) = find_tray_pos(w);
            SetWindowPos(hwnd, HWND_TOPMOST, x, y, w, WINDOW_H,
                SWP_NOACTIVATE | SWP_NOSENDCHANGING);
            let wide = to_wide(&text);
            SetWindowTextW(HWND_LABEL, wide.as_ptr());
            SetWindowPos(HWND_LABEL, ptr::null_mut(), 0, 0, w, WINDOW_H, SWP_NOZORDER | SWP_NOREDRAW);
            InvalidateRect(hwnd, ptr::null_mut(), 0);
            0
        }
        WM_DESTROY => { PostQuitMessage(0); 0 }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    // Single-instance enforcement: if another copy is already running, exit silently
    unsafe {
        let mutex_name = to_wide(MUTEX_NAME);
        let handle = CreateMutexW(ptr::null_mut(), 0, mutex_name.as_ptr());
        if handle.is_null() || winapi::um::errhandlingapi::GetLastError() == winapi::shared::winerror::ERROR_ALREADY_EXISTS {
            // Another instance is running — just exit
            return;
        }
        // Don't close the handle — keep it alive for the process lifetime
    }

    set_autostart();

    unsafe {
        let name = to_wide("TaskbarIPC");
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wnd_proc),
            cbClsExtra: 0, cbWndExtra: 0,
            hInstance: GetModuleHandleW(ptr::null_mut()),
            hIcon: ptr::null_mut(),
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            hbrBackground: ptr::null_mut(),
            lpszMenuName: ptr::null_mut(),
            lpszClassName: name.as_ptr(),
        };
        RegisterClassW(&wc);
        let ip = get_first_ipv4();
        let text = format!("Broj Racunara: {}", ip);
        let tw = measure_text(&text);
        let w = tw + PADDING_X;
        let (x, y) = find_tray_pos(w);
        let taskbar = FindWindowW(to_wide("Shell_TrayWnd").as_ptr(), ptr::null_mut());
        
        // On Windows 7 or earlier, setting hwndParent to taskbar causes DWM to layer the window
        // UNDER Shell_TrayWnd. Setting parent to NULL creates an un-owned HWND_TOPMOST window
        // that sits cleanly ABOVE the taskbar on Windows 7.
        let parent_hwnd = if is_windows_7_or_lower() {
            ptr::null_mut()
        } else {
            taskbar
        };

        let _hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE,
            name.as_ptr(), to_wide("Broj Racunara").as_ptr(),
            WS_POPUP | WS_VISIBLE,
            x, y, w, WINDOW_H,
            parent_hwnd,
            ptr::null_mut(),
            GetModuleHandleW(ptr::null_mut()),
            ptr::null_mut(),
        );
        PostMessageW(_hwnd, WM_TIMER, 0, 0);
        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) != 0 {
            TranslateMessage(&msg); DispatchMessageW(&msg);
        }
    }
}
