use std::ffi::{OsString, c_void};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HWND};
use windows_sys::Win32::Networking::WinHttp::*;
use windows_sys::Win32::System::Com::CoTaskMemFree;
use windows_sys::Win32::System::Registry::*;
use windows_sys::Win32::UI::Shell::{FOLDERID_Documents, SHGetKnownFolderPath, ShellExecuteW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

const RUN_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_VALUE: &str = "ClickShow";
const RELEASE_URL: &str = "https://github.com/cuiliang/ClickShow/releases";
const CURRENT_VERSION: [i32; 4] = [1, 4, 1, 0];
const MAX_VERSION_RESPONSE: usize = 64 * 1024;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn registry_error(action: &str, code: u32) -> String {
    format!(
        "{action}：{}",
        std::io::Error::from_raw_os_error(code as i32)
    )
}

pub fn settings_path() -> Result<PathBuf, String> {
    let mut raw = null_mut();
    // The returned buffer belongs to the COM task allocator, including on a
    // successful call made without a COM apartment.
    let result = unsafe { SHGetKnownFolderPath(&FOLDERID_Documents, 0, null_mut(), &mut raw) };
    if result < 0 || raw.is_null() {
        if !raw.is_null() {
            unsafe { CoTaskMemFree(raw.cast()) };
        }
        return Err(format!("无法获取文档目录：HRESULT 0x{:08X}", result as u32));
    }
    let path = unsafe {
        let mut length = 0;
        while *raw.add(length) != 0 {
            length += 1;
        }
        let value = OsString::from_wide(std::slice::from_raw_parts(raw, length));
        CoTaskMemFree(raw.cast());
        value
    };
    Ok(PathBuf::from(path).join("ClickShow.setting"))
}

struct RegistryKey(HKEY);

impl Drop for RegistryKey {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open_run_key(access: REG_SAM_FLAGS, create: bool) -> Result<Option<RegistryKey>, String> {
    let name = wide(RUN_KEY);
    let mut key = null_mut();
    let status = unsafe {
        if create {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                access,
                null(),
                &mut key,
                null_mut(),
            )
        } else {
            RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut key)
        }
    };
    match status {
        ERROR_SUCCESS => Ok(Some(RegistryKey(key))),
        ERROR_FILE_NOT_FOUND if !create => Ok(None),
        code => Err(registry_error("无法打开开机启动配置", code)),
    }
}

pub fn autostart_enabled() -> Result<bool, String> {
    let Some(key) = open_run_key(KEY_QUERY_VALUE, false)? else {
        return Ok(false);
    };
    let name = wide(RUN_VALUE);
    let mut size = 0;
    // Existence is the original application's criterion, regardless of value
    // type or whether another copy wrote the executable path.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            name.as_ptr(),
            null(),
            null_mut(),
            null_mut(),
            &mut size,
        )
    };
    match status {
        ERROR_SUCCESS => Ok(true),
        ERROR_FILE_NOT_FOUND => Ok(false),
        code => Err(registry_error("无法加载开机自动启动状态", code)),
    }
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let Some(key) = open_run_key(KEY_SET_VALUE, enabled)? else {
        return Ok(());
    };
    let name = wide(RUN_VALUE);
    let status = if enabled {
        let executable = std::env::current_exe().map_err(|e| format!("无法获取程序路径：{e}"))?;
        let command: Vec<u16> = std::iter::once(u16::from(b'"'))
            .chain(executable.as_os_str().encode_wide())
            .chain([u16::from(b'"'), 0])
            .collect();
        let size = u32::try_from(std::mem::size_of_val(command.as_slice()))
            .map_err(|_| "程序路径过长".to_owned())?;
        unsafe {
            RegSetValueExW(
                key.0,
                name.as_ptr(),
                0,
                REG_SZ,
                command.as_ptr().cast(),
                size,
            )
        }
    } else {
        unsafe { RegDeleteValueW(key.0, name.as_ptr()) }
    };
    if status == ERROR_SUCCESS || (!enabled && status == ERROR_FILE_NOT_FOUND) {
        Ok(())
    } else {
        Err(registry_error("无法保存开机自动启动状态", status))
    }
}

fn is_http_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    (scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
        && !rest.is_empty()
        && !url.chars().any(char::is_control)
}

pub fn open_url(owner: HWND, url: &str) -> Result<(), String> {
    if !is_http_url(url) {
        return Err("只允许打开 HTTP 或 HTTPS 网址".to_owned());
    }
    let operation = wide("open");
    let address = wide(url);
    let result = unsafe {
        ShellExecuteW(
            owner,
            operation.as_ptr(),
            address.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result > 32 {
        Ok(())
    } else {
        Err(format!("无法打开网址：{url}（错误 {result}）"))
    }
}

#[derive(Debug)]
pub struct UpdateInfo {
    pub raw: String,
    pub version: String,
    pub url: String,
}

struct HttpHandle(*mut c_void);

impl HttpHandle {
    fn new(raw: *mut c_void) -> Option<Self> {
        if raw.is_null() { None } else { Some(Self(raw)) }
    }
}

impl Drop for HttpHandle {
    fn drop(&mut self) {
        unsafe { WinHttpCloseHandle(self.0) };
    }
}

fn download_version() -> Option<String> {
    let agent = wide("ClickShow/1.4.1.0");
    let session = HttpHandle::new(unsafe {
        WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        )
    })?;
    if unsafe { WinHttpSetTimeouts(session.0, 5000, 5000, 5000, 5000) } == 0 {
        return None;
    }
    let host = wide("helperservice.getquicker.cn");
    let connection = HttpHandle::new(unsafe { WinHttpConnect(session.0, host.as_ptr(), 443, 0) })?;
    let verb = wide("GET");
    let path = wide("/clickshow/version");
    let request = HttpHandle::new(unsafe {
        WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        )
    })?;
    if unsafe { WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) } == 0
        || unsafe { WinHttpReceiveResponse(request.0, null_mut()) } == 0
    {
        return None;
    }
    let mut status = 0_u32;
    let mut status_size = std::mem::size_of_val(&status) as u32;
    if unsafe {
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut status_size,
            null_mut(),
        )
    } == 0
        || !(200..300).contains(&status)
    {
        return None;
    }
    let mut response = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        let mut read = 0;
        if unsafe {
            WinHttpReadData(
                request.0,
                buffer.as_mut_ptr().cast(),
                buffer.len() as u32,
                &mut read,
            )
        } == 0
        {
            return None;
        }
        if read == 0 {
            break;
        }
        if response.len() + read as usize > MAX_VERSION_RESPONSE {
            return None;
        }
        response.extend_from_slice(&buffer[..read as usize]);
    }
    String::from_utf8(response).ok()
}

// System.Version requires two to four nonnegative Int32 components; missing
// build/revision components compare as -1 rather than zero.
fn parse_version(text: &str) -> Option<[i32; 4]> {
    let mut version = [-1; 4];
    let parts: Vec<_> = text.split('.').collect();
    if !(2..=4).contains(&parts.len()) {
        return None;
    }
    for (index, part) in parts.into_iter().enumerate() {
        let value = part.trim().parse::<i32>().ok()?;
        if value < 0 {
            return None;
        }
        version[index] = value;
    }
    Some(version)
}

fn parse_update(raw: String) -> Option<UpdateInfo> {
    let mut parts = raw.trim_start_matches('\u{feff}').split('|');
    let version = parts.next()?.trim();
    if parse_version(version)? <= CURRENT_VERSION {
        return None;
    }
    let url = parts
        .next()
        .map(str::trim)
        .filter(|url| is_http_url(url))
        .unwrap_or(RELEASE_URL)
        .to_owned();
    Some(UpdateInfo {
        version: version.to_owned(),
        url,
        raw,
    })
}

pub fn check_update() -> Option<UpdateInfo> {
    parse_update(download_version()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_system_version_comparison() {
        assert_eq!(parse_version("1.4"), Some([1, 4, -1, -1]));
        assert_eq!(parse_version("1.4.1"), Some([1, 4, 1, -1]));
        assert_eq!(parse_version(" 1.4.1.0\r\n"), Some(CURRENT_VERSION));
        assert!(parse_version("1.4.1.1").unwrap() > CURRENT_VERSION);
        assert!(parse_version("1.4.2").unwrap() > CURRENT_VERSION);
        assert!(parse_version("1.4.1").unwrap() < CURRENT_VERSION);
        for invalid in [
            "",
            "1",
            "1.2.3.4.5",
            "1..2",
            "1.2.-1",
            "1.2147483648",
            "v1.5",
        ] {
            assert!(parse_version(invalid).is_none(), "{invalid}");
        }
    }

    #[test]
    fn update_preserves_response_and_uses_optional_download_url() {
        let raw = "1.5.0|https://example.com/release\r\n".to_owned();
        let update = parse_update(raw.clone()).unwrap();
        assert_eq!(update.raw, raw);
        assert_eq!(update.version, "1.5.0");
        assert_eq!(update.url, "https://example.com/release");
        for response in ["1.5", "1.5|", "1.5|file:///invalid"] {
            assert_eq!(parse_update(response.to_owned()).unwrap().url, RELEASE_URL);
        }
        for response in ["1.4.1.0", "1.4.1", "1.3.99", "broken"] {
            assert!(parse_update(response.to_owned()).is_none());
        }
    }

    #[test]
    fn shell_urls_must_use_http_or_https() {
        assert!(is_http_url(
            "https://github.com/cuiliang/ClickShow/releases"
        ));
        assert!(is_http_url("HTTP://example.com"));
        for url in [
            "file:///C:/test",
            "javascript:alert(1)",
            "https://",
            "https://example.com\0file",
        ] {
            assert!(!is_http_url(url), "{url}");
        }
    }
}
