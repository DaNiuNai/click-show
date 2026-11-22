const RELEASE_URL: &str = "https://github.com/cuiliang/ClickShow/releases";
const CURRENT_VERSION: [i32; 4] = [1, 4, 1, 0];
const MAX_VERSION_RESPONSE: usize = 64 * 1024;

fn is_http_url(url: &str) -> bool {
    let Some((scheme, rest)) = url.split_once("://") else {
        return false;
    };
    (scheme.eq_ignore_ascii_case("https") || scheme.eq_ignore_ascii_case("http"))
        && !rest.is_empty()
        && !url.chars().any(char::is_control)
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct UpdateInfo {
    #[serde(skip)]
    pub raw: String,
    pub version: String,
    pub url: String,
}

fn download_version() -> Option<String> {
    use std::{io::Read, time::Duration};
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .user_agent(concat!("ClickShow/", env!("CARGO_PKG_VERSION")))
        .build()
        .ok()?;
    let response = client
        .get("https://helperservice.getquicker.cn/clickshow/version")
        .send()
        .ok()?
        .error_for_status()
        .ok()?;
    let mut text = String::new();
    response
        .take(MAX_VERSION_RESPONSE as u64 + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() <= MAX_VERSION_RESPONSE).then_some(text)
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
