//! "Find the firmware on this machine" for the locked panel.
//!
//! On Windows the button opens an Explorer search window for the Peak image
//! files the linker accepts, so a user who has downloaded the firmware but
//! forgotten where it went can find it without leaving the DAW. The search
//! itself is an ordinary Explorer window: nothing is read or uploaded by the
//! plugin.
//!
//! The URI builder is a pure function so it is tested on every platform; only
//! the process launch is Windows-specific.

/// File-name pattern for the images the linker accepts. Novation's own
/// downloads are named `peak-firmware-<version>-build<n>.syx` and
/// `peak-fpga-<version>-build<n>.syx`; matching on `peak*.syx` covers both and
/// is still narrow enough to skip ordinary patch dumps.
pub const FIRMWARE_SEARCH_PATTERN: &str = "peak*.syx";

/// What the search window is titled in Explorer.
const DISPLAY_NAME: &str = "Peak firmware";

/// True on platforms where [`open_firmware_search`] can do something. The panel
/// hides the button elsewhere instead of offering one that always fails.
pub const fn search_supported() -> bool {
    cfg!(windows)
}

/// Percent-encode `text` for use inside a `search-ms:` URI.
///
/// Unreserved characters and `*` (the wildcard Explorer expects to see
/// literally) pass through; everything else, including the `\` and `:` of a
/// Windows path, is escaped byte by byte.
fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'*' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Build the `search-ms:` URI Explorer understands.
///
/// `location` is the folder tree to search; `None` leaves the choice to
/// Explorer, which then uses its indexed locations.
pub fn windows_search_uri(pattern: &str, location: Option<&str>) -> String {
    let mut uri = format!(
        "search-ms:displayname={}&query={}",
        encode(DISPLAY_NAME),
        encode(pattern)
    );
    if let Some(location) = location.filter(|l| !l.trim().is_empty()) {
        uri.push_str("&crumb=location:");
        uri.push_str(&encode(location));
    }
    uri
}

/// Open an Explorer search for the firmware image.
///
/// Returns as soon as Explorer has been started. The search covers the user's
/// profile folder (Downloads, Desktop, Documents and the rest), which is where
/// the Novation download ends up in nearly every case; the Explorer window it
/// opens lets the user widen the location.
#[cfg(windows)]
pub fn open_firmware_search() -> Result<(), String> {
    use std::path::PathBuf;
    use std::process::Command;

    let location = std::env::var("USERPROFILE")
        .ok()
        .filter(|path| !path.trim().is_empty());
    let uri = windows_search_uri(FIRMWARE_SEARCH_PATTERN, location.as_deref());

    // Name Explorer by full path. A bare `explorer.exe` is resolved against the
    // directory of the running executable first, and inside a plugin that is
    // the host's folder, not ours to trust.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let explorer = PathBuf::from(root).join("explorer.exe");

    // Explorer reports exit status 1 even when it opens the window, so the
    // status is deliberately not inspected; only a failure to start counts.
    Command::new(explorer)
        .arg(uri)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start Windows search: {error}"))
}

/// Windows search does not exist here; see [`search_supported`].
#[cfg(not(windows))]
pub fn open_firmware_search() -> Result<(), String> {
    Err("Windows search is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_keeps_its_wildcard() {
        let uri = windows_search_uri(FIRMWARE_SEARCH_PATTERN, None);
        assert_eq!(uri, "search-ms:displayname=Peak%20firmware&query=peak*.syx");
    }

    #[test]
    fn location_is_escaped_as_a_crumb() {
        let uri = windows_search_uri("peak*.syx", Some(r"C:\Users\Dysk 0"));
        assert!(
            uri.ends_with("&crumb=location:C%3A%5CUsers%5CDysk%200"),
            "{uri}"
        );
    }

    #[test]
    fn blank_location_is_ignored() {
        assert!(!windows_search_uri("peak*.syx", Some("  ")).contains("crumb"));
    }

    #[test]
    fn nothing_that_could_split_the_uri_survives_unescaped() {
        let uri = windows_search_uri("a b&c=d", Some(r"D:\x&y\z"));
        let query = uri.split("&query=").nth(1).unwrap();
        // The only raw `&` left after the query value is the crumb separator.
        assert_eq!(query.matches('&').count(), 1, "{uri}");
        assert!(!uri.contains(' '));
    }

    #[test]
    fn non_ascii_paths_are_escaped_per_byte() {
        assert_eq!(encode("é"), "%C3%A9");
    }

    #[cfg(not(windows))]
    #[test]
    fn other_platforms_report_unsupported() {
        assert!(!search_supported());
        assert!(open_firmware_search().is_err());
    }
}
