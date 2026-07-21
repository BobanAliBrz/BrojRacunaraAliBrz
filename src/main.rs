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
use winapi::um::winnt::{KEY_SET_VALUE, KEY_WRITE, REG_SZ};
use winapi::um::winreg::*;
use winapi::um::winuser::*;
use winapi::um::wingdi::*;

static mut HWND_LABEL: HWND = ptr::null_mut();
const PADDING_X: i32 = 16;
const WINDOW_H: i32 = 30;
const MUTEX_NAME: &str = "Global\\TaskbarIP_SingleInstance";

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

/// Get the all-users Startup folder path
fn all_users_startup_path() -> String {
    "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\taskbar-ip.exe".to_string()
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
    let programdata = normalize_path("C:\\ProgramData\\TaskbarIP\\");
    let startup_all = normalize_path("C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\");

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
        let mut hkey: HKEY = ptr::null_mut();

        let ret = RegCreateKeyExW(
            hive,
            subkey.as_ptr(),
            0,
            ptr::null_mut(),
            0,
            KEY_SET_VALUE | KEY_WRITE,
            ptr::null_mut(),
            &mut hkey,
            ptr::null_mut(),
        );
        if ret != 0 { return; }

        let name = to_wide("TaskbarIP");
        let value_wide = to_wide(exe_path);
        let byte_len = (value_wide.len() * 2) as DWORD;

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

fn set_autostart() {
    let exe = get_module_path();
    if exe.is_empty() { return; }

    // If we're already running from an installed location, don't re-install.
    // Just make sure our persistence mechanisms are intact.
    if is_running_from_install_location(&exe) {
        // Verify registry key exists pointing to ProgramData copy
        let shared_path = "C:\\ProgramData\\TaskbarIP\\taskbar-ip.exe";
        if file_exists_and_valid(shared_path) {
            let admin = is_elevated();
            set_registry_autostart(shared_path, admin);
            if !admin {
                set_registry_autostart(shared_path, false);
            }
        }
        return;
    }

    // --- First-time install (running from SMB share, USB, desktop, etc.) ---

    let admin = is_elevated();

    // Derive companion uninstaller path (same dir as source exe)
    let exe_path = std::path::Path::new(&exe);
    let uninstaller_src = exe_path.with_file_name("uninstall.exe");

    // Step 1: Copy to ProgramData (shared, accessible by all users)
    let shared_dir = "C:\\ProgramData\\TaskbarIP";
    let shared_exe = format!("{}\\taskbar-ip.exe", shared_dir);
    let shared_uninstaller = format!("{}\\uninstall.exe", shared_dir);

    if let Err(_) = std::fs::create_dir_all(shared_dir) {
        // If we can't create the dir, we can't install — bail
        // Still try user-level install below
    }

    // Copy main exe
    let shared_copy_ok = match std::fs::copy(&exe, &shared_exe) {
        Ok(bytes) => bytes > 0,
        Err(_) => false,
    };

    // Copy uninstaller if available (best-effort)
    if uninstaller_src.exists() {
        let _ = std::fs::copy(&uninstaller_src, &shared_uninstaller);
    }

    // Step 2: Copy to Startup folders
    if shared_copy_ok && file_exists_and_valid(&shared_exe) {
        // Use the ProgramData copy as the source for Startup folders
        // (more reliable than the potentially-remote SMB source)

        if admin {
            // All-users Startup folder
            let all_startup = all_users_startup_path();
            let _ = std::fs::copy(&shared_exe, &all_startup);
        }

        // Current-user Startup folder (always attempt)
        if let Some(user_startup) = user_startup_path() {
            let _ = std::fs::copy(&shared_exe, &user_startup);
        }

        // Step 3: Registry Run key (backup persistence)
        set_registry_autostart(&shared_exe, admin);
        if !admin {
            set_registry_autostart(&shared_exe, false);
        }
    } else {
        // ProgramData copy failed — try direct user-level install
        if let Some(user_startup) = user_startup_path() {
            if std::fs::copy(&exe, &user_startup).is_ok() {
                set_registry_autostart(&user_startup, false);
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

fn find_tray_pos(window_w: i32) -> (i32, i32) {
    unsafe {
        let mut work_area: RECT = mem::zeroed();
        SystemParametersInfoW(0x0030, 0, &mut work_area as *mut _ as *mut winapi::ctypes::c_void, 0);
        let taskbar = FindWindowW(to_wide("Shell_TrayWnd").as_ptr(), ptr::null_mut());
        if taskbar.is_null() { return (work_area.right - window_w, work_area.bottom - 30); }
        let tray = FindWindowExW(taskbar, ptr::null_mut(), to_wide("TrayNotifyWnd").as_ptr(), ptr::null_mut());
        if tray.is_null() { return (work_area.right - window_w, work_area.bottom - 30); }
        let mut tray_rc: RECT = mem::zeroed();
        if GetWindowRect(tray, &mut tray_rc) != 0 {
            let y = tray_rc.top + (tray_rc.bottom - tray_rc.top - 30) / 2;
            return (tray_rc.left - window_w, y);
        }
        (work_area.right - window_w, work_area.bottom - 30)
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
            SetWindowPos(hwnd, ptr::null_mut(), x, y, w, WINDOW_H,
                SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOREDRAW);
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
        let _hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            name.as_ptr(), to_wide("Broj Racunara").as_ptr(),
            WS_POPUP | WS_VISIBLE,
            x, y, w, WINDOW_H,
            taskbar,
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
