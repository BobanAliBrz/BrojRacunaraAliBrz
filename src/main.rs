#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use sha2::{Digest, Sha256};
use winapi::shared::minwindef::{BOOL, DWORD, HKEY, LPARAM, LRESULT, TRUE, UINT, WPARAM};
use winapi::shared::windef::{HFONT, HWND, POINT, RECT};
use winapi::shared::ws2def::AF_INET;
use winapi::um::iphlpapi::GetAdaptersAddresses;
use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winhttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_FLAG_SECURE,
};
use winapi::um::winnt::{KEY_SET_VALUE, KEY_WRITE, REG_DWORD, REG_SZ};
use winapi::um::winreg::*;
use winapi::um::winuser::*;
use winapi::um::wingdi::*;

static mut HWND_LABEL: HWND = ptr::null_mut();
static mut LABEL_FONT: HFONT = ptr::null_mut();
static mut LAYOUT_INITIALIZED: bool = false;
static mut LAST_X: i32 = 0;
static mut LAST_Y: i32 = 0;
static mut LAST_W: i32 = 0;
static mut LAST_TEXT: Option<String> = None;
const PADDING_X: i32 = 6;
const WINDOW_H: i32 = 30;
const MUTEX_NAME: &str = "Global\\TaskbarIP_SingleInstance";
const KEY_WOW64_64KEY: DWORD = 0x0100;
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const UPDATE_API_HOST: &str = "api.github.com";
const UPDATE_API_PATH: &str = "/repos/BobanAliBrz/BrojRacunaraAliBrz/releases/latest";
const UPDATE_DOWNLOAD_HOST: &str = "github.com";
const IP_REFRESH_TIMER_ID: usize = 1;
const Z_ORDER_CHECK_TIMER_ID: usize = 2;
const Z_ORDER_CHECK_INTERVAL_MS: UINT = 100;

/// Preview builds must not install themselves or change the machine's startup settings.
fn is_preview_mode() -> bool {
    std::env::var("TASKBAR_IP_PREVIEW").as_deref() == Ok("1")
}

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

fn local_install_dir() -> Option<String> {
    std::env::var("LOCALAPPDATA").ok().map(|local| format!("{}\\TaskbarIP", local))
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

    if let Some(local_install) = local_install_dir() {
        if norm.starts_with(&normalize_path(&format!("{}\\", local_install))) {
            return true;
        }
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
                set_sz("DisplayVersion", CURRENT_VERSION);
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
        // Keep a per-user install in its writable location. This is necessary
        // for silent updates without requiring elevation.
        let installed_as_admin = admin && normalize_path(&exe).starts_with(&normalize_path(&shared_dir));
        set_registry_autostart(&exe, installed_as_admin);
        if !installed_as_admin {
            set_registry_autostart(&exe, false);
        }
        register_uninstall_entry(&exe, installed_as_admin);
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

/// Fetch an HTTPS resource without blocking the overlay's UI thread.
/// WinHTTP is present on every supported Windows version and uses the system
/// proxy and certificate store.
fn https_get(host: &str, path: &str, max_bytes: usize) -> Option<Vec<u8>> {
    unsafe {
        let agent = to_wide(&format!("TaskbarIP/{}", CURRENT_VERSION));
        let session = WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            ptr::null(),
            ptr::null(),
            0,
        );
        if session.is_null() {
            return None;
        }
        let _ = WinHttpSetTimeouts(session, 5_000, 5_000, 10_000, 20_000);

        let host_w = to_wide(host);
        let connection = WinHttpConnect(session, host_w.as_ptr(), INTERNET_DEFAULT_HTTPS_PORT, 0);
        if connection.is_null() {
            WinHttpCloseHandle(session);
            return None;
        }

        let method = to_wide("GET");
        let path_w = to_wide(path);
        let request = WinHttpOpenRequest(
            connection,
            method.as_ptr(),
            path_w.as_ptr(),
            ptr::null(),
            ptr::null(),
            ptr::null_mut(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            WinHttpCloseHandle(connection);
            WinHttpCloseHandle(session);
            return None;
        }

        let sent = WinHttpSendRequest(request, ptr::null(), 0, ptr::null_mut(), 0, 0, 0);
        let received = sent != 0 && WinHttpReceiveResponse(request, ptr::null_mut()) != 0;
        if !received {
            WinHttpCloseHandle(request);
            WinHttpCloseHandle(connection);
            WinHttpCloseHandle(session);
            return None;
        }

        let mut body = Vec::new();
        loop {
            let mut available: DWORD = 0;
            if WinHttpQueryDataAvailable(request, &mut available) == 0 {
                WinHttpCloseHandle(request);
                WinHttpCloseHandle(connection);
                WinHttpCloseHandle(session);
                return None;
            }
            if available == 0 {
                break;
            }
            if body.len().saturating_add(available as usize) > max_bytes {
                WinHttpCloseHandle(request);
                WinHttpCloseHandle(connection);
                WinHttpCloseHandle(session);
                return None;
            }

            let start = body.len();
            body.resize(start + available as usize, 0);
            let mut read: DWORD = 0;
            if WinHttpReadData(
                request,
                body[start..].as_mut_ptr() as *mut _,
                available,
                &mut read,
            ) == 0 {
                WinHttpCloseHandle(request);
                WinHttpCloseHandle(connection);
                WinHttpCloseHandle(session);
                return None;
            }
            body.truncate(start + read as usize);
        }

        WinHttpCloseHandle(request);
        WinHttpCloseHandle(connection);
        WinHttpCloseHandle(session);
        Some(body)
    }
}

fn parse_version(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.trim_start_matches('v').split('.');
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some() { None } else { Some(parsed) }
}

fn update_is_newer(latest: &str) -> bool {
    match (parse_version(latest), parse_version(CURRENT_VERSION)) {
        (Some(latest), Some(current)) => latest > current,
        _ => false,
    }
}

/// Download and execute a verified newer installer, if one is published.
/// The updater accepts only the public GitHub release API response for this
/// repository and verifies the release's SHA-256 digest before execution.
fn install_latest_release() {
    let metadata = match https_get(UPDATE_API_HOST, UPDATE_API_PATH, 1_024 * 1_024) {
        Some(data) => data,
        None => return,
    };
    let release: serde_json::Value = match serde_json::from_slice(&metadata) {
        Ok(value) => value,
        Err(_) => return,
    };
    let tag = match release.get("tag_name").and_then(|value| value.as_str()) {
        Some(tag) if update_is_newer(tag) => tag,
        _ => return,
    };
    let asset = match release.get("assets").and_then(|value| value.as_array()).and_then(|assets| {
        assets.iter().find(|asset| {
            asset.get("name").and_then(|value| value.as_str()) == Some("setup.exe")
                && asset.get("state").and_then(|value| value.as_str()) == Some("uploaded")
        })
    }) {
        Some(asset) => asset,
        None => return,
    };
    let url = match asset.get("browser_download_url").and_then(|value| value.as_str()) {
        Some(url) => url,
        None => return,
    };
    let expected_digest = match asset.get("digest").and_then(|value| value.as_str()) {
        Some(digest) if digest.len() == 71 && digest.starts_with("sha256:") => digest,
        _ => return,
    };
    let expected_size = match asset.get("size").and_then(|value| value.as_u64()) {
        Some(size) if size > 0 && size <= 16 * 1_024 * 1_024 => size as usize,
        _ => return,
    };
    let prefix = "https://github.com/BobanAliBrz/BrojRacunaraAliBrz/";
    let path = match url.strip_prefix(prefix) {
        Some(path) if path.starts_with("releases/download/") => format!("/{}", path),
        _ => return,
    };

    let installer = match https_get(UPDATE_DOWNLOAD_HOST, &path, expected_size) {
        Some(data) if data.len() == expected_size => data,
        _ => return,
    };
    let actual_digest = format!("sha256:{:x}", Sha256::digest(&installer));
    if actual_digest != expected_digest {
        return;
    }

    let installer_path = std::env::temp_dir().join(format!(
        "TaskbarIP-{}-setup.exe",
        tag.trim_start_matches('v')
    ));
    if std::fs::write(&installer_path, installer).is_ok() {
        let _ = std::process::Command::new(installer_path)
            .arg("--silent-update")
            .spawn();
    }
}

fn start_automatic_updates() {
    std::thread::spawn(|| loop {
        install_latest_release();
        std::thread::sleep(std::time::Duration::from_secs(24 * 60 * 60));
    });
}

#[cfg(test)]
mod tests {
    use super::{parse_version, update_is_newer, CURRENT_VERSION};

    #[test]
    fn parses_release_versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("v1.2"), None);
        assert_eq!(parse_version("v1.2.3-beta"), None);
    }

    #[test]
    fn accepts_only_newer_versions() {
        let (major, minor, patch) = parse_version(CURRENT_VERSION).unwrap();
        assert!(update_is_newer(&format!("v{}.{}.{}", major, minor, patch + 1)));
        assert!(update_is_newer(&format!("v{}.{}.0", major, minor + 1)));
        assert!(!update_is_newer(CURRENT_VERSION));
        assert!(!update_is_newer(&format!("v{}.{}.{}", major, minor, patch.saturating_sub(1))));
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

fn windows_version() -> Option<(DWORD, DWORD, DWORD)> {
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
                    return Some((osvi.dwMajorVersion, osvi.dwMinorVersion, osvi.dwBuildNumber));
                }
            }
        }
        None
    }
}

/// Windows 7 is 6.1; Vista and earlier use lower version numbers.
fn is_windows_7_or_lower() -> bool {
    match windows_version() {
        Some((major, minor, _)) => major < 6 || (major == 6 && minor <= 1),
        None => false,
    }
}

/// Windows 11 reports 10.0 with build 22000 or later.
fn is_windows_11_or_higher() -> bool {
    matches!(windows_version(), Some((major, _, build)) if major > 10 || (major == 10 && build >= 22_000))
}

const RB_GETBANDCOUNT: UINT = WM_USER + 6;
const RB_GETRECT: UINT = WM_USER + 9;
const RB_GETBANDINFOW: UINT = WM_USER + 29;
const RBBIM_CHILD: UINT = 0x00000010;
const RBBIM_STYLE: UINT = 0x00000001;
const RBBS_HIDDEN: UINT = 0x00000008;

#[repr(C)]
#[allow(non_snake_case)]
struct REBARBANDINFO_MIN {
    cbSize: UINT,
    fMask: UINT,
    fStyle: UINT,
    clrFore: DWORD,
    clrBack: DWORD,
    lpText: *mut u16,
    cch: UINT,
    iImage: i32,
    hwndChild: HWND,
}

struct TaskbarDetectionContext {
    taskbar_rect: RECT,
    is_horizontal: bool,
    start_right: i32,
    right_boundary: i32,
}

unsafe fn get_window_class(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let len = GetClassNameW(hwnd, buf.as_mut_ptr(), 256);
    if len == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..len as usize])
}

unsafe extern "system" fn enum_taskbar_children(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam as *mut TaskbarDetectionContext);
    if IsWindowVisible(hwnd) == 0 {
        return TRUE;
    }

    let class_name = get_window_class(hwnd);
    if class_name == "TaskbarIPC" {
        return TRUE;
    }

    let mut rc: RECT = mem::zeroed();
    if GetWindowRect(hwnd, &mut rc) == 0 {
        return TRUE;
    }

    if rc.right <= rc.left || rc.bottom <= rc.top {
        return TRUE;
    }

    let tb = &ctx.taskbar_rect;
    if ctx.is_horizontal {
        // Window must overlap vertically with the taskbar
        if rc.bottom <= tb.top || rc.top >= tb.bottom {
            return TRUE;
        }

        // Left-side controls (Start button, Search box, Task list)
        if (class_name == "Button" && rc.left < tb.left + 80)
            || class_name == "Start"
            || class_name == "TrayDummySearchControl"
        {
            if rc.right > ctx.start_right {
                ctx.start_right = rc.right;
            }
            return TRUE;
        }
        if class_name == "MSTaskSwWClass" || class_name == "MSTaskListWClass" {
            return TRUE;
        }

        // Right-side controls (TrayNotifyWnd, Clock, ShowDesktop, Toolbars/Deskbands, Language Bar, TrayButtons)
        let is_right_element = class_name == "TrayNotifyWnd"
            || class_name == "TrayClockWClass"
            || class_name == "TrayShowDesktopButtonWClass"
            || class_name == "ToolbarWindow32"
            || class_name == "CiceroUIWndFrame"
            || class_name == "TrayButton"
            || class_name == "InputIndicatorFlyout"
            || class_name.contains("DeskBand")
            || class_name.contains("Deskband")
            || (rc.right <= tb.right + 4 && rc.left > tb.left + 80);

        if is_right_element {
            if rc.left < ctx.right_boundary && rc.left > ctx.start_right {
                ctx.right_boundary = rc.left;
            }
        }
    } else {
        // Vertical taskbar
        if rc.right <= tb.left || rc.left >= tb.right {
            return TRUE;
        }
        let is_bottom_element = class_name == "TrayNotifyWnd"
            || class_name == "TrayClockWClass"
            || class_name == "ToolbarWindow32"
            || (rc.bottom <= tb.bottom + 4 && rc.top > tb.top + 60);

        if is_bottom_element {
            if rc.top < ctx.right_boundary {
                ctx.right_boundary = rc.top;
            }
        }
    }

    TRUE
}

unsafe extern "system" fn enum_top_level_overlap(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam as *mut TaskbarDetectionContext);
    if IsWindowVisible(hwnd) == 0 {
        return TRUE;
    }

    let class_name = get_window_class(hwnd);
    if class_name == "TaskbarIPC" || class_name == "Shell_TrayWnd" || class_name == "Shell_SecondaryTrayWnd" {
        return TRUE;
    }

    let mut rc: RECT = mem::zeroed();
    if GetWindowRect(hwnd, &mut rc) == 0 {
        return TRUE;
    }

    if rc.right <= rc.left || rc.bottom <= rc.top {
        return TRUE;
    }

    let tb = &ctx.taskbar_rect;
    if ctx.is_horizontal {
        let vertically_inside = rc.top >= tb.top - 10 && rc.bottom <= tb.bottom + 10;
        let horizontally_inside = rc.left >= tb.left && rc.right <= tb.right + 10;

        if vertically_inside && horizontally_inside {
            let is_docked = class_name == "CiceroUIWndFrame"
                || class_name == "TF_FloatingLangBar_WndTitle"
                || (rc.left > ctx.start_right && rc.left < ctx.right_boundary);

            if is_docked {
                if rc.left < ctx.right_boundary && rc.left > ctx.start_right {
                    ctx.right_boundary = rc.left;
                }
            }
        }
    }

    TRUE
}

fn find_tray_pos(window_w: i32) -> (i32, i32) {
    unsafe {
        let mut work_area: RECT = mem::zeroed();
        SystemParametersInfoW(0x0030, 0, &mut work_area as *mut _ as *mut winapi::ctypes::c_void, 0);

        let taskbar = FindWindowW(to_wide("Shell_TrayWnd").as_ptr(), ptr::null_mut());
        if taskbar.is_null() {
            return (work_area.right - window_w, work_area.bottom - WINDOW_H);
        }

        let mut tb_rc: RECT = mem::zeroed();
        if GetWindowRect(taskbar, &mut tb_rc) == 0 {
            return (work_area.right - window_w, work_area.bottom - WINDOW_H);
        }

        let is_horizontal = (tb_rc.right - tb_rc.left) >= (tb_rc.bottom - tb_rc.top);

        let mut ctx = TaskbarDetectionContext {
            taskbar_rect: tb_rc,
            is_horizontal,
            start_right: tb_rc.left,
            right_boundary: if is_horizontal { tb_rc.right } else { tb_rc.bottom },
        };

        // 1. Check TrayNotifyWnd as baseline
        let tray = FindWindowExW(taskbar, ptr::null_mut(), to_wide("TrayNotifyWnd").as_ptr(), ptr::null_mut());
        if !tray.is_null() && IsWindowVisible(tray) != 0 {
            let mut tray_rc: RECT = mem::zeroed();
            if GetWindowRect(tray, &mut tray_rc) != 0 {
                if is_horizontal {
                    if tray_rc.left < ctx.right_boundary && tray_rc.left > tb_rc.left {
                        ctx.right_boundary = tray_rc.left;
                    }
                } else {
                    if tray_rc.top < ctx.right_boundary && tray_rc.top > tb_rc.top {
                        ctx.right_boundary = tray_rc.top;
                    }
                }
            }
        }

        // 2. Query ReBar bands directly (catches Language Bar, Help button, and deskbands on Win7/8/10)
        let rebar = FindWindowExW(taskbar, ptr::null_mut(), to_wide("ReBarWindow32").as_ptr(), ptr::null_mut());
        if !rebar.is_null() && IsWindowVisible(rebar) != 0 {
            let band_count = SendMessageW(rebar, RB_GETBANDCOUNT, 0, 0) as i32;
            for i in 0..band_count {
                let mut band_rc: RECT = mem::zeroed();
                if SendMessageW(rebar, RB_GETRECT, i as WPARAM, &mut band_rc as *mut _ as LPARAM) != 0 {
                    MapWindowPoints(rebar, ptr::null_mut(), &mut band_rc as *mut _ as *mut POINT, 2);

                    let mut info: REBARBANDINFO_MIN = mem::zeroed();
                    info.cbSize = mem::size_of::<REBARBANDINFO_MIN>() as UINT;
                    info.fMask = RBBIM_CHILD | RBBIM_STYLE;
                    let _ = SendMessageW(rebar, RB_GETBANDINFOW, i as WPARAM, &mut info as *mut _ as LPARAM);

                    if (info.fStyle & RBBS_HIDDEN) == 0 && band_rc.right > band_rc.left && band_rc.bottom > band_rc.top {
                        let child_class = if !info.hwndChild.is_null() {
                            get_window_class(info.hwndChild)
                        } else {
                            String::new()
                        };

                        if child_class == "MSTaskSwWClass" || child_class == "MSTaskListWClass" {
                            // Main task buttons band on left
                        } else {
                            // Right-docked deskband/toolbar (Language bar, Help button, custom deskbands)
                            if is_horizontal {
                                if band_rc.left < ctx.right_boundary && band_rc.left > tb_rc.left + 80 {
                                    ctx.right_boundary = band_rc.left;
                                }
                            } else {
                                if band_rc.top < ctx.right_boundary && band_rc.top > tb_rc.top + 60 {
                                    ctx.right_boundary = band_rc.top;
                                }
                            }
                        }
                    }
                }
            }
        }

        // 3. Enumerate all taskbar child windows to catch individual controls/toolbars
        EnumChildWindows(taskbar, Some(enum_taskbar_children), &mut ctx as *mut _ as LPARAM);

        // 4. Enumerate overlapping top-level windows to catch docked/popup Cicero language bars
        EnumWindows(Some(enum_top_level_overlap), &mut ctx as *mut _ as LPARAM);

        const GAP: i32 = 6;
        if is_horizontal {
            let y = tb_rc.top + (tb_rc.bottom - tb_rc.top - WINDOW_H) / 2;
            let x = ctx.right_boundary - GAP - window_w;
            (x, y)
        } else {
            let x = tb_rc.left + (tb_rc.right - tb_rc.left - window_w) / 2;
            let y = ctx.right_boundary - GAP - WINDOW_H;
            (x, y)
        }
    }
}

fn measure_text(text: &str) -> i32 {
    unsafe {
        let hdc = GetDC(ptr::null_mut());
        // Measure with the exact font assigned to the label. This keeps the
        // background snug and leaves enough room for the text to stay on one line.
        let font = if LABEL_FONT.is_null() {
            GetStockObject(DEFAULT_GUI_FONT as i32)
        } else {
            LABEL_FONT as *mut _
        };
        let old_font = SelectObject(hdc, font);
        let mut rc: RECT = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        DrawTextW(hdc, wide.as_ptr(), wide.len() as i32 - 1, &mut rc, DT_CALCRECT | DT_SINGLELINE);
        SelectObject(hdc, old_font);
        ReleaseDC(ptr::null_mut(), hdc);
        rc.right - rc.left
    }
}

fn taskbar_is_above_overlay(overlay: HWND) -> bool {
    unsafe {
        let taskbar = FindWindowW(to_wide("Shell_TrayWnd").as_ptr(), ptr::null_mut());
        if taskbar.is_null() {
            return false;
        }

        // Walk the desktop z-order from front to back. If the taskbar appears
        // before our popup, Explorer has covered the overlay and it needs one
        // topmost restore. Start-menu windows do not trigger this path.
        let mut window = GetTopWindow(ptr::null_mut());
        for _ in 0..1024 {
            if window.is_null() || window == overlay {
                return false;
            }
            if window == taskbar {
                return true;
            }
            window = GetWindow(window, GW_HWNDNEXT);
        }
        false
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            let label = CreateWindowExW(0, to_wide("STATIC").as_ptr(), to_wide("...").as_ptr(),
                WS_CHILD | WS_VISIBLE | SS_CENTER | SS_CENTERIMAGE | SS_NOPREFIX,
                0, 0, 100, WINDOW_H, hwnd,
                ptr::null_mut(), GetModuleHandleW(ptr::null_mut()), ptr::null_mut());
            HWND_LABEL = label;
            if !LABEL_FONT.is_null() {
                SendMessageW(label, WM_SETFONT, LABEL_FONT as WPARAM, 1);
            }
            SetTimer(hwnd, IP_REFRESH_TIMER_ID, 1000, None);
            SetTimer(hwnd, Z_ORDER_CHECK_TIMER_ID, Z_ORDER_CHECK_INTERVAL_MS, None);
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wp as winapi::shared::windef::HDC;
            SetBkColor(hdc, 0x00FFFFFF);
            SetTextColor(hdc, 0);
            GetStockObject(WHITE_BRUSH as i32) as LRESULT
        }
        WM_TIMER => {
            if wp == Z_ORDER_CHECK_TIMER_ID {
                if taskbar_is_above_overlay(hwnd) {
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOSENDCHANGING,
                    );
                }
                return 0;
            }
            if wp != IP_REFRESH_TIMER_ID {
                return 0;
            }
            let ip = get_first_ipv4();
            let text = format!("Broj Racunara: {}", ip);
            let tw = measure_text(&text);
            let w = tw + PADDING_X * 2;
            let (x, y) = find_tray_pos(w);

            let layout_changed = !LAYOUT_INITIALIZED || x != LAST_X || y != LAST_Y || w != LAST_W;
            let width_changed = !LAYOUT_INITIALIZED || w != LAST_W;
            let text_changed = match &*ptr::addr_of!(LAST_TEXT) {
                Some(last) => last != &text,
                None => true,
            };

            if layout_changed {
                // Make the popup topmost once, then keep its existing z-order. Reasserting
                // HWND_TOPMOST every second causes visible flashing when Explorer changes
                // taskbar or Start-menu focus.
                let flags = if LAYOUT_INITIALIZED {
                    SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOZORDER
                } else {
                    SWP_NOACTIVATE | SWP_NOSENDCHANGING
                };
                let insert_after = if LAYOUT_INITIALIZED { ptr::null_mut() } else { HWND_TOPMOST };
                if SetWindowPos(hwnd, insert_after, x, y, w, WINDOW_H, flags) != 0 {
                    LAYOUT_INITIALIZED = true;
                    LAST_X = x;
                    LAST_Y = y;
                    LAST_W = w;
                }
            }
            if width_changed {
                SetWindowPos(HWND_LABEL, ptr::null_mut(), 0, 0, w, WINDOW_H, SWP_NOZORDER | SWP_NOREDRAW);
            }
            if text_changed {
                let wide = to_wide(&text);
                SetWindowTextW(HWND_LABEL, wide.as_ptr());
                LAST_TEXT = Some(text);
            }
            0
        }
        WM_DESTROY => {
            if !LABEL_FONT.is_null() {
                DeleteObject(LABEL_FONT as *mut _);
                LABEL_FONT = ptr::null_mut();
            }
            LAYOUT_INITIALIZED = false;
            LAST_TEXT = None;
            PostQuitMessage(0);
            0
        }
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

    if !is_preview_mode() {
        set_autostart();
        start_automatic_updates();
    }

    unsafe {
        LABEL_FONT = CreateFontW(
            15, 0, 0, 0, 600, 0, 0, 0, 0, 0, 0, 0, 0,
            to_wide("Segoe UI").as_ptr(),
        );
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
        let w = tw + PADDING_X * 2;
        let (x, y) = find_tray_pos(w);
        let taskbar = FindWindowW(to_wide("Shell_TrayWnd").as_ptr(), ptr::null_mut());
        
        // Windows 7 can layer taskbar-owned popups under the taskbar. On Windows 11,
        // Start menu activation can hide a taskbar-owned popup. In both cases an
        // independent topmost tool window remains visible without joining Alt+Tab.
        let parent_hwnd = if is_windows_7_or_lower() || is_windows_11_or_higher() {
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
