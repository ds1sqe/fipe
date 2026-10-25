//! `file:` URIs and paths.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use lsp_types::Uri;

fn decode(text: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(text.len());
    let mut iter = text.bytes();
    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let hex = [iter.next()?, iter.next()?];
            bytes.push(
                u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?,
            );
        } else {
            bytes.push(byte);
        }
    }
    let decoded = String::from_utf8(bytes).ok()?;
    (!decoded.contains('\0')).then_some(decoded)
}

fn encode(text: &str) -> String {
    let mut encoded = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

/// The absolute filesystem path a `file:` URI names.
/// Queries, fragments and unsupported authorities are not filesystem paths.
#[must_use]
pub fn path(uri: &Uri) -> Option<PathBuf> {
    if !uri.scheme()?.as_str().eq_ignore_ascii_case("file")
        || uri.query().is_some()
        || uri.fragment().is_some()
    {
        return None;
    }
    let authority = uri.authority()?;
    if authority.userinfo().is_some() || authority.port().is_some() {
        return None;
    }
    let authority = decode(authority.as_str())?;
    if authority.contains(['/', '\\']) {
        return None;
    }
    let decoded = decode(uri.path().as_str())?;
    native_path(&authority, &decoded)
}

#[cfg(not(windows))]
fn native_path(authority: &str, decoded: &str) -> Option<PathBuf> {
    if (!authority.is_empty() && !authority.eq_ignore_ascii_case("localhost"))
        || !decoded.starts_with('/')
    {
        return None;
    }
    Some(PathBuf::from(decoded))
}

#[cfg(windows)]
fn native_path(authority: &str, decoded: &str) -> Option<PathBuf> {
    if authority.is_empty() || authority.eq_ignore_ascii_case("localhost") {
        let drive = decoded.strip_prefix('/')?;
        let bytes = drive.as_bytes();
        if bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && bytes[2] == b'/'
        {
            return Some(PathBuf::from(drive.replace('/', "\\")));
        }
        if authority.is_empty() {
            return None;
        }
        // A localhost authority with a share path is a local UNC share.
    }
    if authority.contains([':', '[', ']']) {
        return None;
    }
    let share = decoded.strip_prefix('/')?.split('/').next()?;
    if share.is_empty() || matches!(share, "." | "..") {
        return None;
    }
    Some(PathBuf::from(format!(
        "\\\\{authority}{}",
        decoded.replace('/', "\\")
    )))
}

#[cfg(not(windows))]
fn file_uri(path: &Path) -> Option<String> {
    path.is_absolute()
        .then(|| format!("file://{}", encode(&path.to_string_lossy())))
}

#[cfg(windows)]
fn file_uri(path: &Path) -> Option<String> {
    use std::path::{Component, Prefix};
    if !path.is_absolute() {
        return None;
    }
    let Component::Prefix(prefix) = path.components().next()? else {
        return None;
    };
    let text = path.to_string_lossy().replace('\\', "/");
    match prefix.kind() {
        Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => {
            let normal = text.strip_prefix("//?/").unwrap_or(&text);
            Some(format!(
                "file:///{}:{}",
                char::from(drive.to_ascii_uppercase()),
                encode(&normal[2..])
            ))
        }
        Prefix::UNC(_, _) | Prefix::VerbatimUNC(_, _) => {
            let normal = text
                .strip_prefix("//?/UNC/")
                .or_else(|| text.strip_prefix("//"))?;
            Some(format!("file://{}", encode(normal)))
        }
        _ => None,
    }
}

/// Compare selected filesystem spellings without resolving or reading the file.
/// Windows canonical paths may add a verbatim prefix absent from editor URIs.
pub(crate) fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    #[cfg(windows)]
    {
        file_uri(left)
            .zip(file_uri(right))
            .is_some_and(|(left, right)| left == right)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// The `file:` URI of an absolute path.
///
/// # Panics
/// If the path is relative, or is an unsupported Windows device/verbatim path.
/// Windows disk and UNC paths, including their canonical verbatim forms, work.
#[must_use]
pub fn of(path: &Path) -> Uri {
    let encoded =
        file_uri(path).expect("file URI requires an absolute disk or UNC path");
    Uri::from_str(&encoded).expect("a percent-encoded file URI")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> Option<PathBuf> {
        text.parse().ok().and_then(|uri| path(&uri))
    }

    #[test]
    fn uri_components_are_not_filename_suffixes() {
        for uri in [
            "https://localhost/a",
            "file:///a?query",
            "file:///a#fragment",
            "file://user@localhost/a",
            "file://localhost:80/a",
            "file:///a%00b",
            "file:///a%FF",
            "file:///a%GG",
            "file://local%2Fhost/a",
        ] {
            assert_eq!(parsed(uri), None, "{uri}");
        }
    }

    #[test]
    #[should_panic(expected = "file URI requires an absolute")]
    fn relative_paths_do_not_become_authorities() {
        let _ = of(Path::new("relative.fipe"));
    }

    #[cfg(not(windows))]
    #[test]
    fn unix_paths_preserve_filename_bytes_and_backslashes() {
        let file = Path::new("/tmp/é # % ? \\.fipe");
        let uri = "file:///tmp/%C3%A9%20%23%20%25%20%3F%20%5C.fipe";
        assert_eq!(of(file).as_str(), uri);
        assert_eq!(parsed(uri), Some(file.to_path_buf()));
        assert_eq!(
            parsed("file://localhost/tmp/a"),
            Some(PathBuf::from("/tmp/a"))
        );
        for uri in [
            "file://localhostevil/tmp/a",
            "file://server/share/a",
            "file:relative",
        ] {
            assert_eq!(parsed(uri), None);
        }
        assert!(!same_path(
            Path::new("/missing/link"),
            Path::new("/missing/target")
        ));
    }

    #[cfg(windows)]
    #[test]
    fn windows_drive_paths_use_standard_file_uris() {
        let file = Path::new(r"D:\dir space\é#%.fipe");
        let uri = "file:///D:/dir%20space/%C3%A9%23%25.fipe";
        assert_eq!(of(file).as_str(), uri);
        assert_eq!(of(Path::new(r"\\?\D:\dir space\é#%.fipe")).as_str(), uri);
        assert_eq!(parsed(uri), Some(file.to_path_buf()));
        assert_eq!(
            parsed("file://localhost/D:/dir%20space/%C3%A9%23%25.fipe"),
            Some(file.to_path_buf())
        );
        assert_eq!(
            parsed("file:///D%3A/dir%20space/%C3%A9%23%25.fipe"),
            Some(file.to_path_buf())
        );
        assert_eq!(
            parsed("file:///D:/name%23%3F.fipe"),
            Some(PathBuf::from(r"D:\name#?.fipe"))
        );
        assert!(same_path(
            Path::new(r"D:\missing\selected.fipi"),
            Path::new(r"\\?\D:\missing\selected.fipi")
        ));
        assert!(!same_path(
            Path::new(r"D:\missing\link.fipi"),
            Path::new(r"D:\missing\target.fipi")
        ));
        for uri in [
            "file:///relative",
            "file:///D:relative",
            "file:///D:/a?query",
            "file:///D:/a#fragment",
        ] {
            assert_eq!(parsed(uri), None);
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_unc_paths_keep_the_complete_authority() {
        let file = Path::new(r"\\server\share name\é#%.fipe");
        let uri = "file://server/share%20name/%C3%A9%23%25.fipe";
        assert_eq!(of(file).as_str(), uri);
        assert_eq!(
            of(Path::new(r"\\?\UNC\server\share name\é#%.fipe")).as_str(),
            uri
        );
        assert_eq!(parsed(uri), Some(file.to_path_buf()));
        assert_eq!(
            parsed("file://localhost/share/a"),
            Some(PathBuf::from(r"\\localhost\share\a"))
        );
        assert_eq!(
            of(Path::new(r"\\localhost\share\a")).as_str(),
            "file://localhost/share/a"
        );
        assert_eq!(
            parsed("file://localhostevil/share/a"),
            Some(PathBuf::from(r"\\localhostevil\share\a"))
        );
        assert!(same_path(
            Path::new(r"\\server\share\missing.fipi"),
            Path::new(r"\\?\UNC\server\share\missing.fipi")
        ));
        for uri in [
            "file://server/",
            "file://server",
            "file://server/../a",
            "file://server:80/share/a",
            "file://user@server/share/a",
        ] {
            assert_eq!(parsed(uri), None);
        }
    }

    #[cfg(windows)]
    #[test]
    #[should_panic(expected = "file URI requires an absolute")]
    fn windows_non_disk_verbatim_paths_are_unsupported() {
        let _ = of(Path::new(r"\\?\Volume{unrelated-device}\file.fipe"));
    }

    #[cfg(windows)]
    #[test]
    #[should_panic(expected = "file URI requires an absolute")]
    fn windows_devices_do_not_become_file_authorities() {
        let _ = of(Path::new(r"\\.\COM1"));
    }
}
