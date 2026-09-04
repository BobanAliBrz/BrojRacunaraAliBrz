#![windows_subsystem = "windows"]

use std::ffi::OsStr;
use std::mem;
use std::os::windows::ffi::OsStrExt;
use std::ptr;
use sha2::{Digest, Sha256};
use winapi::shared::minwindef::{BOOL, DWORD, HKEY, LPARAM, LRESULT, TRUE, UINT, WPARAM};
use winapi::shared::windef::{HFONT, HWINEVENTHOOK, HWND, POINT, RECT};
use winapi::shared::ws2def::AF_INET;
use winapi::um::iphlpapi::GetAdaptersAddresses;
use winapi::um::libloaderapi::{GetModuleFileNameW, GetModuleHandleW};
use winapi::um::synchapi::CreateMutexW;
use winapi::um::winhttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryDataAvailable, WinHttpReadData, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT,
    WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_SECURE_PROTOCOLS,
};
// Secure-protocol flag values (WinHTTP SDK; not defined by winapi 0.3).
const WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2: DWORD = 0x0000_0800;
const WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3: DWORD = 0x0000_2000;

// ---- SMB (LAN) update fallback -------------------------------------------
// Flow: GitHub first; when unreachable (proxy down, Win7 TLS, offline) or its
// download fails verification, try the LAN update share with a dedicated
// low-privilege worker account. Same convention as Print Spooler Guardian:
// PSG-style versioned setup files in the per-program dir, newest-newer wins.
// NOTE: worker password is embedded (read-only LAN account, same tradeoff as
// the other updater here); rotating it means rebuilding.
const SMB_UPDATE_USER: &str = "auto_update_worker";
const SMB_UPDATE_PASSWORD: &str = "autoupdate12716";
// Dedicated worker-visible share (read-only). Probing it without worker
// creds yields 1223 (credential prompt, i.e. it exists but denies us),
// never 67 — confirmed present, contents visible to the worker only.
const SMB_UPDATE_DIR: &str = "\\\\10.0.135.252\\taskbar ip auto update";
const SMB_MAX_BYTES: usize = 16 * 1_024 * 1_024;

const RESOURCETYPE_DISK: DWORD = 0x0000_0001;
const CONNECT_TEMPORARY: DWORD = 0x0000_0004;

#[repr(C)]
#[allow(non_snake_case)]
struct NETRESOURCEW {
    dwScope: DWORD,
    dwType: DWORD,
    dwDisplayType: DWORD,
    dwUsage: DWORD,
    lpLocalName: *mut u16,
    lpRemoteName: *mut u16,
    lpComment: *mut u16,
    lpProvider: *mut u16,
}

#[link(name = "mpr")]
extern "system" {
    fn WNetAddConnection2W(
        lpNetResource: *const NETRESOURCEW,
        lpPassword: *const u16,
        lpUserName: *const u16,
        dwFlags: DWORD,
    ) -> DWORD;
    fn WNetCancelConnection2W(lpName: *const u16, dwFlags: DWORD, fForce: BOOL) -> DWORD;
}
use winapi::um::winnt::{KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WRITE, REG_DWORD, REG_SZ};
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
static mut TASKBAR_CREATED_MSG: UINT = 0;
static mut Z_ORDER_MISSES: u32 = 0;
static mut OVERLAY_HWND: HWND = ptr::null_mut();
static mut FOREGROUND_HOOK: HWINEVENTHOOK = ptr::null_mut();
static mut REORDER_HOOK: HWINEVENTHOOK = ptr::null_mut();
// Coalescing flag: reorder events fire in bursts (Start animation, tooltips,
// IME). Without this the hook would flood our queue and starve WM_TIMER,
// which is only delivered when the queue is empty.
static RECHECK_PENDING: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
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
const Z_ORDER_CHECK_INTERVAL_MS: UINT = 500;
const Z_ORDER_CHECK_INTERVAL_MS_WIN11: UINT = 100;
const Z_ORDER_RESTORE_THRESHOLD: u32 = 2;
// Win11 restores on the first hit: screenshots proved the visible blackout is
// Shell_TrayWnd sitting above the overlay after a taskbar click or Start
// close, so waiting only lengthens the outage. Win7 keeps its gentler
// debounce (untested here).
const Z_ORDER_RESTORE_THRESHOLD_WIN11: u32 = 1;
// Posted by the WinEvent hook the moment foreground or top-level z-order
// changes anywhere: re-check immediately instead of waiting for the poll.
const RECHECK_Z_ORDER_MSG: UINT = WM_APP + 1;
const EVENT_SYSTEM_FOREGROUND: DWORD = 0x0003;
const EVENT_OBJECT_REORDER: DWORD = 0x8004;
const WINEVENT_OUTOFCONTEXT: UINT = 0x0000;
const WINEVENT_SKIPOWNPROCESS: UINT = 0x0002;

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

/// Hand off to a newer per-user copy, if one exists.
///
/// Silent updates install to `%LOCALAPPDATA%\TaskbarIP` when the shared
/// `%ProgramData%` copy is not writable, leaving an admin-installed (HKLM)
/// entry behind that points at the older binary. HKLM Run entries launch
/// before HKCU ones, so without this the stale copy would win every login,
/// re-download the release, and blink the overlay. If the HKCU uninstall
/// entry records a newer version than this binary and the local exe exists,
/// spawn it and exit before taking the single-instance mutex. Must run before
/// set_autostart(), which would otherwise overwrite the newer HKCU
/// registration with this binary's older version.
fn yield_to_newer_local_install() {
    let local_dir = match local_install_dir() {
        Some(dir) => dir,
        None => return,
    };
    let local_exe = format!("{}\\taskbar-ip.exe", local_dir);
    if !file_exists_and_valid(&local_exe) {
        return;
    }
    let subkey = to_wide("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\TaskbarIP");
    let value_name = to_wide("DisplayVersion");
    let mut registered = String::new();
    unsafe {
        // Mirror the write order (native 64-bit view first).
        let views = [KEY_QUERY_VALUE | KEY_WOW64_64KEY, KEY_QUERY_VALUE];
        for &access in &views {
            let mut hkey: HKEY = ptr::null_mut();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                subkey.as_ptr(),
                0,
                access,
                &mut hkey,
            ) != 0
                || hkey.is_null()
            {
                continue;
            }
            let mut kind: DWORD = 0;
            let mut bytes: DWORD = 0;
            if RegQueryValueExW(
                hkey,
                value_name.as_ptr(),
                ptr::null_mut(),
                &mut kind,
                ptr::null_mut(),
                &mut bytes,
            ) == 0
                && kind == REG_SZ
                && bytes >= 2
                && bytes <= 512
            {
                let mut buf = vec![0u16; (bytes / 2) as usize];
                let mut got = bytes;
                if RegQueryValueExW(
                    hkey,
                    value_name.as_ptr(),
                    ptr::null_mut(),
                    &mut kind,
                    buf.as_mut_ptr() as *mut u8,
                    &mut got,
                ) == 0
                {
                    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
                    registered = String::from_utf16_lossy(&buf[..len]);
                }
            }
            RegCloseKey(hkey);
            if !registered.is_empty() {
                break;
            }
        }
    }
    if registered.is_empty() {
        return;
    }
    let newer = match (parse_version(&registered), parse_version(CURRENT_VERSION)) {
        (Some(r), Some(c)) => r > c,
        _ => false,
    };
    if newer {
        let _ = std::process::Command::new(&local_exe).spawn();
        std::process::exit(0);
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

        // GitHub requires TLS 1.2+. Stock Windows 7 negotiates only TLS 1.0
        // by default (TLS 1.1/1.2 need KB3140245 and are opt-in), so request
        // modern protocols explicitly. Where already default this is a no-op;
        // where a flag is unknown the call fails and we fall back to TLS 1.2
        // alone, then to system defaults — never worse than today.
        let opt_len = std::mem::size_of::<DWORD>() as DWORD;
        let mut modern = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2 | WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_3;
        if WinHttpSetOption(
            request,
            WINHTTP_OPTION_SECURE_PROTOCOLS,
            &mut modern as *mut DWORD as *mut _,
            opt_len,
        ) == 0
        {
            let mut tls12 = WINHTTP_FLAG_SECURE_PROTOCOL_TLS1_2;
            let _ = WinHttpSetOption(
                request,
                WINHTTP_OPTION_SECURE_PROTOCOLS,
                &mut tls12 as *mut DWORD as *mut _,
                opt_len,
            );
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

struct ReleaseInfo {
    tag: String,
    path: String,
    digest: String,
    size: usize,
}

enum ReleaseCheck {
    /// GitHub reachable and this binary is current: nothing to do anywhere.
    Current,
    /// GitHub has a newer verified release.
    Available(ReleaseInfo),
    /// GitHub unreachable or unusable: LAN fallback may still know better.
    Unknown,
}

/// Read the latest published GitHub release without downloading anything.
fn fetch_release_info() -> ReleaseCheck {
    let metadata = match https_get(UPDATE_API_HOST, UPDATE_API_PATH, 1_024 * 1_024) {
        Some(data) => data,
        None => return ReleaseCheck::Unknown,
    };
    let release: serde_json::Value = match serde_json::from_slice(&metadata) {
        Ok(value) => value,
        Err(_) => return ReleaseCheck::Unknown,
    };
    let tag = match release.get("tag_name").and_then(|value| value.as_str()) {
        Some(tag) if update_is_newer(tag) => tag.to_string(),
        Some(_) => return ReleaseCheck::Current,
        None => return ReleaseCheck::Unknown,
    };
    let asset = match release.get("assets").and_then(|value| value.as_array()).and_then(|assets| {
        assets.iter().find(|asset| {
            asset.get("name").and_then(|value| value.as_str()) == Some("setup.exe")
                && asset.get("state").and_then(|value| value.as_str()) == Some("uploaded")
        })
    }) {
        Some(asset) => asset,
        None => return ReleaseCheck::Unknown,
    };
    let url = match asset.get("browser_download_url").and_then(|value| value.as_str()) {
        Some(url) => url,
        None => return ReleaseCheck::Unknown,
    };
    let digest = match asset.get("digest").and_then(|value| value.as_str()) {
        Some(digest) if digest.len() == 71 && digest.starts_with("sha256:") => digest.to_string(),
        _ => return ReleaseCheck::Unknown,
    };
    let size = match asset.get("size").and_then(|value| value.as_u64()) {
        Some(size) if size > 0 && size <= 16 * 1_024 * 1_024 => size as usize,
        _ => return ReleaseCheck::Unknown,
    };
    // Keep the owner/repo in the request path: the download lives at
    // /<owner>/<repo>/releases/download/<tag>/setup.exe on github.com.
    // (Stripping the repo prefix produced /releases/download/..., which
    // GitHub answers with 404 "Not Found", silently disabling updates.)
    let prefix = "https://github.com/";
    let path = match url.strip_prefix(prefix) {
        Some(path)
            if path.starts_with("BobanAliBrz/BrojRacunaraAliBrz/releases/download/") =>
        {
            format!("/{}", path)
        }
        _ => return ReleaseCheck::Unknown,
    };
    ReleaseCheck::Available(ReleaseInfo { tag, path, digest, size })
}

/// Download the GitHub asset and install it iff size and digest verify.
fn install_verified_github_bytes(info: &ReleaseInfo) -> bool {
    let installer = match https_get(UPDATE_DOWNLOAD_HOST, &info.path, info.size) {
        Some(data) if data.len() == info.size => data,
        _ => return false,
    };
    if format!("sha256:{:x}", Sha256::digest(&installer)) != info.digest {
        return false;
    }
    stage_and_launch(&info.tag, &installer);
    true
}

/// Write verified installer bytes to temp and start them silently.
fn stage_and_launch(tag: &str, bytes: &[u8]) {
    let installer_path = std::env::temp_dir().join(format!(
        "TaskbarIP-{}-setup.exe",
        tag.trim_start_matches('v')
    ));
    if std::fs::write(&installer_path, bytes).is_ok() {
        let _ = std::process::Command::new(installer_path)
            .arg("--silent-update")
            .spawn();
    }
}

/// Parse `TaskbarIP_Setup_v1.2.4(.0).exe` style names into
/// (major, minor, patch, build). Comparison uses the full tuple with the
/// running binary treated as `x.y.z.0`.
fn parse_smb_filename_version(name: &str) -> Option<(u32, u32, u32, u32)> {
    let lower = name.to_lowercase();
    let bytes = lower.as_bytes();
    let mut start = None;
    let mut i = 0;
    while i + 2 < bytes.len() {
        if (bytes[i] == b'_' || bytes[i] == b'-')
            && bytes[i + 1] == b'v'
            && bytes[i + 2].is_ascii_digit()
        {
            start = Some(i + 2);
        }
        i += 1;
    }
    let mut parts = lower[start?..]
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty());
    let parsed = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next().and_then(|p| p.parse().ok()).unwrap_or(0),
    );
    Some(parsed)
}

fn current_version_tuple() -> (u32, u32, u32, u32) {
    match parse_version(CURRENT_VERSION) {
        Some((major, minor, patch)) => (major, minor, patch, 0),
        None => (0, 0, 0, 0),
    }
}

/// Connect with the worker account. Returns `Some(we_connected)` when the dir
/// is listable afterwards. On conflict (1219: another session already covers
/// the server, e.g. the user's own mapping; 85: already assigned) the
/// existing session is reused, so office PCs with mapped drives keep working.
fn smb_connect(dir: &str) -> Option<bool> {
    unsafe {
        let remote_w = to_wide(dir);
        let user_w = to_wide(SMB_UPDATE_USER);
        let pass_w = to_wide(SMB_UPDATE_PASSWORD);
        let resource = NETRESOURCEW {
            dwScope: 0,
            dwType: RESOURCETYPE_DISK,
            dwDisplayType: 0,
            dwUsage: 0,
            lpLocalName: ptr::null_mut(),
            lpRemoteName: remote_w.as_ptr() as *mut u16,
            lpComment: ptr::null_mut(),
            lpProvider: ptr::null_mut(),
        };
        let rc = WNetAddConnection2W(
            &resource,
            pass_w.as_ptr(),
            user_w.as_ptr(),
            CONNECT_TEMPORARY,
        );
        let explicit = rc == 0;
        if std::fs::read_dir(dir).is_ok() {
            Some(explicit)
        } else {
            if explicit {
                WNetCancelConnection2W(remote_w.as_ptr(), 0, 0);
            }
            None
        }
    }
}

/// Newest versioned setup in the share dir that is newer than this binary.
fn scan_smb_update_dir(dir: &str) -> Vec<(String, (u32, u32, u32, u32))> {
    let mut found = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return found,
    };
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };
        if !name.to_lowercase().ends_with(".exe") {
            continue;
        }
        if let Some(version) = parse_smb_filename_version(name) {
            if version > current_version_tuple() {
                if let Some(path_str) = path.to_str() {
                    found.push((path_str.to_string(), version));
                }
            }
        }
    }
    found.sort_by(|a, b| b.1.cmp(&a.1));
    found
}

/// SMB fallback for updates. `known` carries trusted (digest, size) from the
/// GitHub API when metadata was reachable but the CDN download failed; then
/// share bytes must verify exactly. Without metadata (e.g. Win7 TLS failure)
/// version comparison plus the size cap apply (LAN trust).
fn install_from_smb(known: Option<(&str, usize)>) -> bool {
    let explicit = match smb_connect(SMB_UPDATE_DIR) {
        Some(explicit) => explicit,
        None => return false,
    };
    let mut installed = false;
    if let Some((path, version)) = scan_smb_update_dir(SMB_UPDATE_DIR).into_iter().next() {
        let tag = format!("v{}.{}.{}", version.0, version.1, version.2);
        let read_ok = match std::fs::metadata(&path) {
            Ok(meta) if meta.len() > 0 && meta.len() <= SMB_MAX_BYTES as u64 => true,
            _ => false,
        };
        if read_ok {
            if let Ok(bytes) = std::fs::read(&path) {
                if let Some((digest, size)) = known {
                    if bytes.len() == size
                        && format!("sha256:{:x}", Sha256::digest(&bytes)) == digest
                    {
                        stage_and_launch(&tag, &bytes);
                        installed = true;
                    }
                } else if !bytes.is_empty() && bytes.len() <= SMB_MAX_BYTES {
                    stage_and_launch(&tag, &bytes);
                    installed = true;
                }
            }
        }
    }
    if explicit {
        unsafe {
            WNetCancelConnection2W(to_wide(SMB_UPDATE_DIR).as_ptr(), 0, 0);
        }
    }
    installed
}

/// Download and execute a verified newer installer, if one is published.
/// GitHub first; the LAN share covers proxy outages, offline sites, and
/// legacy TLS stacks. The updater accepts only the public GitHub release API
/// response for this repository and verifies the release's SHA-256 digest
/// before execution.
fn install_latest_release() {
    match fetch_release_info() {
        // Reachable and current: the share mirrors releases, so nothing newer
        // can hide there. (A blind share scan here would let a compromised
        // share push code while GitHub is healthy.)
        ReleaseCheck::Current => {}
        ReleaseCheck::Available(info) => {
            if install_verified_github_bytes(&info) {
                return;
            }
            install_from_smb(Some((&info.digest, info.size)));
        }
        // API unreachable (proxy down, Win7 TLS, no internet): version-gated
        // LAN trust.
        ReleaseCheck::Unknown => {
            install_from_smb(None);
        }
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

    #[test]
    fn parses_smb_setup_filenames() {
        use super::parse_smb_filename_version as parse;
        assert_eq!(parse("TaskbarIP_Setup_v1.2.4.exe"), Some((1, 2, 4, 0)));
        assert_eq!(parse("TaskbarIP_Setup_v1.2.4.0.exe"), Some((1, 2, 4, 0)));
        assert_eq!(parse("taskbarip-setup-v10.0.135.exe"), Some((10, 0, 135, 0)));
        assert_eq!(parse("setup.exe"), None);
        assert_eq!(parse("TaskbarIP_Setup_final.exe"), None);
        assert_eq!(parse("notes_v1.txt"), None);
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

/// Only unowned popups compete in the topmost z-order band and need a restore
/// timer. Windows 7 and Windows 11 use an unowned `WS_EX_TOPMOST` popup (see
/// `main()`); Windows 8/10 use a taskbar-owned popup that always stays above
/// its owner without `TOPMOST`, so no polling is needed there.
///
/// NOTE: making the Win11 overlay a `WS_CHILD` of `Shell_TrayWnd` was tried
/// and abandoned: the fullscreen `Windows.UI.Composition.
/// DesktopWindowContentBridge` XAML layer paints over foreign classic children
/// (verified by hiding it: overlay instantly bright, tray visuals gone with
/// it), even at sibling-top. A top-level topmost popup is the only working
/// host on Win11; while Start/Search is open its DWM layer covers us and we
/// return on close instead.
fn should_enforce_topmost() -> bool {
    is_windows_7_or_lower() || is_windows_11_or_higher()
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

        // Right-side controls.  Keep this list deliberately specific: Windows 11's
        // XAML taskbar exposes large internal child windows (often ToolbarWindow32
        // or composition bridge windows) for the Start button and task list.  Those
        // windows are not tray-adjacent, even though their bounds can cover most of
        // the taskbar.  Treating every child to the right of Start as a deskband
        // moves the overlay all the way back to Start.
        let is_right_element = class_name == "TrayNotifyWnd"
            || class_name == "TrayClockWClass"
            || class_name == "TrayShowDesktopButtonWClass"
            || class_name == "CiceroUIWndFrame"
            || class_name == "TrayButton"
            || class_name == "InputIndicatorFlyout"
            || class_name.contains("DeskBand")
            || class_name.contains("Deskband");

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
            || class_name == "CiceroUIWndFrame"
            || class_name.contains("DeskBand")
            || class_name.contains("Deskband");

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
            // Do not infer that an arbitrary top-level window is a docked
            // toolbar.  Windows 11 can report taskbar composition windows in
            // this rectangle; only the documented language-bar windows belong
            // in the reserved area.
            let is_docked = class_name == "CiceroUIWndFrame"
                || class_name == "TF_FloatingLangBar_WndTitle";

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

        // 2. Query legacy ReBar bands directly (catches Language Bar, Help
        // button, and deskbands on Win7/8/10).  Windows 11's taskbar is XAML
        // based; its internal layout windows are not ReBar deskbands and must
        // not be allowed to replace the notification-area boundary.
        let rebar = FindWindowExW(taskbar, ptr::null_mut(), to_wide("ReBarWindow32").as_ptr(), ptr::null_mut());
        if !is_windows_11_or_higher() && !rebar.is_null() && IsWindowVisible(rebar) != 0 {
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

/// Single async topmost re-assertion: no move, no size, no repaint. This is
/// what makes the restore invisible; the visible flicker/blackout observed on
/// Win11 was the overlay sitting behind Shell_TrayWnd, not this call.
fn restore_topmost(hwnd: HWND) {
    unsafe {
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
        );
    }
}

/// Recompute the IP text and overlay geometry, moving/resizing/repainting only
/// what changed. Called from the overlay's 1 s tick on every OS version.
fn refresh_layout(overlay: HWND) {
    unsafe {
        if overlay.is_null() || IsWindow(overlay) == 0 {
            return;
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
            // Make the popup topmost once, then keep its existing z-order.
            // Reasserting HWND_TOPMOST every second causes visible flashing
            // when Explorer changes taskbar or Start-menu focus.
            let flags = if LAYOUT_INITIALIZED {
                SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_NOZORDER | SWP_ASYNCWINDOWPOS
            } else {
                SWP_NOACTIVATE | SWP_NOSENDCHANGING | SWP_ASYNCWINDOWPOS
            };
            // Owned Win8/10 popups stay above their owner without TOPMOST;
            // passing HWND_TOPMOST there is ignored at best and contends
            // with the taskbar focus at worst.
            let insert_after = if LAYOUT_INITIALIZED {
                ptr::null_mut()
            } else if should_enforce_topmost() {
                HWND_TOPMOST
            } else {
                HWND_TOP
            };
            if SetWindowPos(overlay, insert_after, x, y, w, WINDOW_H, flags) != 0 {
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
    }
}

/// WinEvent callback. Runs on a system thread (WINEVENT_OUTOFCONTEXT), so it
/// only posts a message; the UI thread does the actual check/restore.
unsafe extern "system" fn win_event_proc(
    _hook: HWINEVENTHOOK,
    _event: DWORD,
    _hwnd: HWND,
    _id_object: i32,
    _id_child: i32,
    _id_thread: DWORD,
    _time: DWORD,
) {
    use std::sync::atomic::Ordering;
    if OVERLAY_HWND.is_null() || RECHECK_PENDING.swap(true, Ordering::SeqCst) {
        return;
    }
    if PostMessageW(OVERLAY_HWND, RECHECK_Z_ORDER_MSG, 0, 0) == 0 {
        RECHECK_PENDING.store(false, Ordering::SeqCst);
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: UINT, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // Event-driven fast path for the Win11 blackout: Explorer puts
    // Shell_TrayWnd above the overlay on taskbar clicks and Start close (and
    // its DWM layer covers the overlay while Start is open, which no classic
    // z-order call can defeat). Fired by the foreground/reorder hook.
    if RECHECK_Z_ORDER_MSG != 0 && msg == RECHECK_Z_ORDER_MSG {
        use std::sync::atomic::Ordering;
        // Clear first so events arriving during the check queue a fresh pass.
        RECHECK_PENDING.store(false, Ordering::SeqCst);
        if should_enforce_topmost() && taskbar_is_above_overlay(hwnd) {
            restore_topmost(hwnd);
            Z_ORDER_MISSES = 0;
        }
        return 0;
    }
    // Explorer broadcasts this when Shell_TrayWnd is recreated (explorer
    // restart). Re-assert topmost once for unowned popups; owned Win8/10
    // popups re-anchor on the next 1s layout tick via find_tray_pos().
    if TASKBAR_CREATED_MSG != 0 && msg == TASKBAR_CREATED_MSG {
        Z_ORDER_MISSES = 0;
        if should_enforce_topmost() {
            restore_topmost(hwnd);
        }
        return 0;
    }
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
            // Win8/10 use a taskbar-owned popup that always stays above its
            // owner: no topmost polling needed, so no timer and no contention
            // with taskbar/Start focus changes. Win7 polls slowly with
            // debounce; Win11 polls fast with immediate restore (see WM_TIMER:
            // the blackout is TB covering us, so delay only hurts).
            if should_enforce_topmost() {
                let interval = if is_windows_11_or_higher() {
                    Z_ORDER_CHECK_INTERVAL_MS_WIN11
                } else {
                    Z_ORDER_CHECK_INTERVAL_MS
                };
                SetTimer(hwnd, Z_ORDER_CHECK_TIMER_ID, interval, None);
            }
            0
        }
        WM_CTLCOLORSTATIC => {
            let hdc = wp as winapi::shared::windef::HDC;
            SetBkColor(hdc, 0x00FFFFFF);
            SetTextColor(hdc, 0);
            GetStockObject(WHITE_BRUSH as i32) as LRESULT
        }
        // Parent has a NULL background brush and the STATIC child paints the
        // white face. Claiming the erase avoids the white-flash background
        // repaint that reads as flicker during z-order swaps.
        WM_ERASEBKGND => 1,
        // Keep an unowned popup in the topmost band without an extra
        // SetWindowPos round-trip. This fires for our own moves; taskbar/Start
        // focus changes do not send us this message, so they no longer cause
        // a synchronous flicker swap.
        WM_WINDOWPOSCHANGING => {
            if should_enforce_topmost() {
                let pos = &mut *(lp as *mut WINDOWPOS);
                if (pos.flags & SWP_NOZORDER) == 0 {
                    pos.hwndInsertAfter = HWND_TOPMOST;
                }
            }
            0
        }
        WM_TIMER => {
            if wp == Z_ORDER_CHECK_TIMER_ID {
                // Restore as soon as Explorer covers us: measured on Win11,
                // every taskbar click and Start close puts Shell_TrayWnd above
                // the overlay and the outage lasts exactly until this restore.
                // The restore itself (async, no move/size/repaint) is
                // invisible. Win7 keeps a 2-hit debounce (untestable here).
                let threshold = if is_windows_11_or_higher() {
                    Z_ORDER_RESTORE_THRESHOLD_WIN11
                } else {
                    Z_ORDER_RESTORE_THRESHOLD
                };
                if taskbar_is_above_overlay(hwnd) {
                    Z_ORDER_MISSES += 1;
                    if Z_ORDER_MISSES >= threshold {
                        restore_topmost(hwnd);
                        Z_ORDER_MISSES = 0;
                    }
                } else {
                    Z_ORDER_MISSES = 0;
                }
                return 0;
            }
            if wp != IP_REFRESH_TIMER_ID {
                return 0;
            }
            refresh_layout(hwnd);
            0
        }
        WM_DESTROY => {
            if !FOREGROUND_HOOK.is_null() {
                UnhookWinEvent(FOREGROUND_HOOK);
                FOREGROUND_HOOK = ptr::null_mut();
            }
            if !REORDER_HOOK.is_null() {
                UnhookWinEvent(REORDER_HOOK);
                REORDER_HOOK = ptr::null_mut();
            }
            OVERLAY_HWND = ptr::null_mut();
            if !LABEL_FONT.is_null() {
                DeleteObject(LABEL_FONT as *mut _);
                LABEL_FONT = ptr::null_mut();
            }
            LAYOUT_INITIALIZED = false;
            LAST_TEXT = None;
            Z_ORDER_MISSES = 0;
            PostQuitMessage(0);
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn main() {
    // A newer per-user copy (from a silent update) takes over before the
    // mutex or autostart handling, so a stale shared copy never re-downloads.
    // Preview builds are exempt: they must never touch installed copies.
    if !is_preview_mode() {
        yield_to_newer_local_install();
    }

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
        TASKBAR_CREATED_MSG =
            RegisterWindowMessageW(to_wide("TaskbarCreated").as_ptr());

        // Per-OS ownership (do not unify without testing each version):
        // - Win7: taskbar-owned popups layer UNDER Shell_TrayWnd, so use an
        //   unowned topmost popup with a restore poll.
        // - Win11: independent topmost tool window (Start can suppress a
        //   taskbar-owned popup). A WS_CHILD was tried and abandoned: the
        //   fullscreen DesktopWindowContentBridge XAML layer paints over
        //   foreign classic children even at sibling-top.
        // - Win8/10: taskbar-owned popup stays above its owner without TOPMOST
        //   and never contends with taskbar/Start focus, so no extra handling.
        let ip = get_first_ipv4();
        let text = format!("Broj Racunara: {}", ip);
        let tw = measure_text(&text);
        let w = tw + PADDING_X * 2;
        let (x, y) = find_tray_pos(w);
        let taskbar = FindWindowW(to_wide("Shell_TrayWnd").as_ptr(), ptr::null_mut());

        // Per-OS ownership (do not unify without testing each version):
        // - Win7: taskbar-owned popups layer UNDER Shell_TrayWnd, so use an
        //   unowned topmost popup.
        // - Win11: Start activation can suppress a taskbar-owned popup, so
        //   use an independent topmost tool window (stays visible, no
        //   Alt+Tab). A WS_CHILD was tried and abandoned: the fullscreen
        //   DesktopWindowContentBridge XAML layer paints over foreign
        //   classic children even at sibling-top.
        // - Win8/10: taskbar-owned popup stays above its owner without
        //   TOPMOST and never contends with taskbar/Start focus.
        let enforce_topmost = should_enforce_topmost();
        let parent_hwnd = if enforce_topmost {
            ptr::null_mut()
        } else {
            taskbar
        };

        // WS_EX_COMPOSITED double-buffers the popup and WS_CLIPCHILDREN
        // keeps parent/STATIC-label repaints from flashing over each other
        // during a z-order swap. Owned Win8/10 popups drop WS_EX_TOPMOST.
        let ex_style = if enforce_topmost {
            WS_EX_TOOLWINDOW | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_COMPOSITED
        } else {
            WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_COMPOSITED
        };

        let _hwnd = CreateWindowExW(
            ex_style,
            name.as_ptr(), to_wide("Broj Racunara").as_ptr(),
            WS_POPUP | WS_VISIBLE | WS_CLIPCHILDREN,
            x, y, w, WINDOW_H,
            parent_hwnd,
            ptr::null_mut(),
            GetModuleHandleW(ptr::null_mut()),
            ptr::null_mut(),
        );
        // Win11 only: instant notice when foreground changes (Start open/
        // close, taskbar click) or top-level z-order is rearranged, so the
        // overlay is restored in milliseconds instead of at the next poll.
        // Out-of-context hook: the callback only PostMessages (coalesced,
        // so WM_TIMER never starves); safe across threads. Failures are
        // non-fatal (the poll timer remains the fallback). Win7 keeps
        // poll-only behavior; Win8/10 need nothing (owned popup).
        if enforce_topmost && is_windows_11_or_higher() && !_hwnd.is_null() {
            OVERLAY_HWND = _hwnd;
            FOREGROUND_HOOK = SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                ptr::null_mut(),
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            REORDER_HOOK = SetWinEventHook(
                EVENT_OBJECT_REORDER,
                EVENT_OBJECT_REORDER,
                ptr::null_mut(),
                Some(win_event_proc),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
        }
        PostMessageW(_hwnd, WM_TIMER, 0, 0);
        let mut msg: MSG = mem::zeroed();
        while GetMessageW(&mut msg, ptr::null_mut(), 0, 0) != 0 {
            TranslateMessage(&msg); DispatchMessageW(&msg);
        }
    }
}
