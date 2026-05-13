#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use winapi::shared::minwindef::{LPARAM, LRESULT, UINT, WPARAM};
use winapi::shared::windef::{HWND, RECT};
use winapi::shared::ws2def::AF_INET;
use winapi::um::iphlpapi::GetAdaptersAddresses;
use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};
use winapi::um::winuser::*;
use winapi::um::wingdi::*;

static mut HWND_LABEL: HWND = ptr::null_mut();
const PADDING_X: i32 = 16;
const WINDOW_H: i32 = 30;

fn to_wide(s: &str) -> Vec<u16> {
    OsStr::new(s).encode_wide().chain(std::iter::once(0)).collect()
}

fn get_module_path() -> String {
    unsafe {
        let mut buf = [0u16; 260];
        let len = GetModuleFileNameW(ptr::null_mut(), buf.as_mut_ptr(), 260);
        if len == 0 { return String::new(); }
        String::from_utf16_lossy(&buf[..len as usize])
    }
}

fn set_autostart() {
    let exe = get_module_path();
    if exe.is_empty() { return; }

    // Derive companion uninstaller path (same dir, different name)
    let exe_path = std::path::Path::new(&exe);
    let uninstaller_src = exe_path.with_file_name("uninstall.exe");

    // Copy to shared ProgramData location (accessible by all users)
    let shared_dir = "C:\\ProgramData\\TaskbarIP";
    let shared_path = format!("{}\\taskbar-ip.exe", shared_dir);
    let shared_uninstaller = format!("{}\\uninstall.exe", shared_dir);
    if exe != shared_path {
        let _ = std::fs::create_dir_all(shared_dir);
        let _ = std::fs::copy(&exe, &shared_path);
        let _ = std::fs::copy(&uninstaller_src, &shared_uninstaller);
    }

    // Place in all-users Startup folder (only works as admin)
    let all_users_startup = "C:\\ProgramData\\Microsoft\\Windows\\Start Menu\\Programs\\StartUp\\taskbar-ip.exe";
    let _ = std::fs::copy(&shared_path, all_users_startup);

    // Place in current user's Startup folder (always works)
    if let Ok(appdata) = std::env::var("APPDATA") {
        let user_startup = format!("{}\\Microsoft\\Windows\\Start Menu\\Programs\\Startup\\taskbar-ip.exe", appdata);
        let _ = std::fs::copy(&shared_path, &user_startup);
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
        let _ = GetAdaptersAddresses(AF_INET.try_into().unwrap(), 0, ptr::null_mut(), ptr::null_mut(), &mut size);
        if size == 0 { return "N/A".to_string(); }
        let layout = std::alloc::Layout::from_size_align_unchecked(size as usize, 8);
        let buf = std::alloc::alloc(layout);
        if buf.is_null() { return "N/A".to_string(); }
        let ret = GetAdaptersAddresses(AF_INET.try_into().unwrap(), 0, ptr::null_mut(), buf as *mut _, &mut size);
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
