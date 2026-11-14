//! bato-native: `text/uri-list` (RFC 2483) to local paths, for desktop file
//! drops on Wayland (`wl_data_device`, see
//! `platform_impl/linux/wayland/dnd.rs`).

use std::path::PathBuf;

/// The local files of a `text/uri-list`: one URI per line (CRLF or LF),
/// `#` comment lines and blank lines skipped, only `file:` URIs (`file:///p`,
/// `file://localhost/p`, `file://<hostname>/p`, `file:/p`), percent-decoded
/// byte-wise (paths need not be UTF-8). Other schemes and other hosts are
/// dropped.
pub fn parse(list: &[u8], hostname: Option<&str>) -> Vec<PathBuf> {
    list.split(|&b| b == b'\n')
        .map(|line| line.strip_suffix(b"\r").unwrap_or(line))
        .map(trim)
        .filter(|line| !line.is_empty() && line[0] != b'#')
        .filter_map(|uri| file_path(uri, hostname))
        .collect()
}

fn trim(mut s: &[u8]) -> &[u8] {
    while let [b' ' | b'\t', rest @ ..] = s {
        s = rest;
    }
    while let [rest @ .., b' ' | b'\t'] = s {
        s = rest;
    }
    s
}

/// The path of one `file:` URI.
fn file_path(uri: &[u8], hostname: Option<&str>) -> Option<PathBuf> {
    if uri.len() < 5 || !uri[..5].eq_ignore_ascii_case(b"file:") {
        return None;
    }
    let rest = &uri[5..];
    let path = if let Some(after) = rest.strip_prefix(b"//") {
        // `//host/path`: the authority ends at the next `/`.
        let slash = after.iter().position(|&b| b == b'/')?;
        let host = &after[..slash];
        let local = host.is_empty()
            || host.eq_ignore_ascii_case(b"localhost")
            || hostname.is_some_and(|h| host.eq_ignore_ascii_case(h.as_bytes()));
        if !local {
            return None;
        }
        &after[slash..]
    } else if rest.starts_with(b"/") {
        rest
    } else {
        return None;
    };
    // A query or fragment is not part of the path.
    let end = path.iter().position(|&b| b == b'?' || b == b'#').unwrap_or(path.len());
    let bytes = percent_decode(&path[..end]);
    if bytes.is_empty() {
        return None;
    }
    Some(from_bytes(bytes))
}

fn percent_decode(s: &[u8]) -> Vec<u8> {
    let hex = |b: u8| match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
    let mut out = Vec::with_capacity(s.len());
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'%' && i + 2 < s.len() {
            if let (Some(h), Some(l)) = (hex(s[i + 1]), hex(s[i + 2])) {
                out.push(h << 4 | l);
                i += 3;
                continue;
            }
        }
        out.push(s[i]);
        i += 1;
    }
    out
}

#[cfg(unix)]
fn from_bytes(bytes: Vec<u8>) -> PathBuf {
    use std::os::unix::ffi::OsStringExt;
    PathBuf::from(std::ffi::OsString::from_vec(bytes))
}

#[cfg(not(unix))]
fn from_bytes(bytes: Vec<u8>) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(&bytes).into_owned())
}
