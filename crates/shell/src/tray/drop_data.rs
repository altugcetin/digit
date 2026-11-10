//! Browser drag payloads; no display or network required.
//!
//! Native Chromium forms (including named octet streams):
//! https://chromium.googlesource.com/chromium/src/+/acca47dc0a9e6971232cf29c4482cc410740f278/ui/ozone/platform/wayland/host/wayland_exchange_data_provider.cc
//! Firefox image URL/HTML forms:
//! https://developer.mozilla.org/en-US/docs/Web/API/HTML_Drag_and_Drop_API/Drag_data_store#dragging_images

pub const MAX_IMAGE: usize = 50 * 1024 * 1024;
pub const MAX_TEXT: usize = MAX_IMAGE.div_ceil(3) * 4 + 4096;

pub fn mime_base(mime: &str) -> &str {
    mime.split(';').next().unwrap_or(mime).trim()
}

pub fn extension(mime: &str) -> Option<&'static str> {
    match mime_base(mime).to_ascii_lowercase().as_str() {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/bmp" | "image/x-ms-bmp" => Some("bmp"),
        "image/tiff" => Some("tiff"),
        "image/x-icon" | "image/vnd.microsoft.icon" => Some("ico"),
        "image/avif" => Some("avif"),
        _ => None,
    }
}

/// URI lists go first so local files win. Their remote URLs are deferred until
/// after all binary image forms (including Chromium's named octet stream).
pub fn pick_forms<'a>(mimes: &[&'a str]) -> Vec<&'a str> {
    let rank = |mime: &str| match mime_base(mime).to_ascii_lowercase().as_str() {
        "text/uri-list" => Some(0),
        "image/png" => Some(1),
        "image/jpeg" | "image/jpg" => Some(2),
        "image/webp" => Some(3),
        "image/gif" => Some(4),
        m if m.starts_with("image/") => Some(5),
        "application/octet-stream" if mime.contains(';') => Some(6),
        "text/x-moz-url" | "text/x-moz-url-data" => Some(7),
        "text/html" => Some(8),
        "text/plain" | "utf8_string" => Some(9),
        _ => None,
    };
    let mut forms: Vec<_> = mimes
        .iter()
        .copied()
        .filter(|m| rank(m).is_some())
        .collect();
    forms.sort_by_key(|m| rank(m));
    forms.dedup();
    forms
}

pub fn uri_list(text: &str) -> Vec<&str> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

pub fn moz_url(text: &str) -> Option<&str> {
    text.lines().next().map(str::trim).filter(|s| !s.is_empty())
}

/// Firefox's native x-moz-url is UTF-16; Chromium's Wayland provider emits
/// UTF-8 for the same MIME type. HTML may carry an explicit UTF-16 charset.
pub fn decode_text(bytes: &[u8], mime: &str) -> Option<String> {
    let lower = mime.to_ascii_lowercase();
    let sample = &bytes[..bytes.len().min(64)];
    let be = bytes.starts_with(&[0xfe, 0xff])
        || lower.contains("utf-16be")
        || sample
            .as_chunks::<2>()
            .0
            .iter()
            .any(|p| p[0] == 0 && p[1] != 0);
    let wide = be
        || bytes.starts_with(&[0xff, 0xfe])
        || lower.contains("utf-16")
        || sample
            .as_chunks::<2>()
            .0
            .iter()
            .any(|p| p[1] == 0 && p[0] != 0);
    let text = if wide {
        if !bytes.len().is_multiple_of(2) {
            return None;
        }
        let units: Vec<u16> = bytes
            .as_chunks::<2>()
            .0
            .iter()
            .map(|p| {
                if be {
                    u16::from_be_bytes([p[0], p[1]])
                } else {
                    u16::from_le_bytes([p[0], p[1]])
                }
            })
            .collect();
        String::from_utf16(&units).ok()?
    } else {
        String::from_utf8(bytes.to_vec()).ok()?
    };
    Some(
        text.trim_start_matches('\u{feff}')
            .trim_end_matches('\0')
            .to_string(),
    )
}

fn entities(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let end = rest.as_bytes().iter().take(17).position(|&b| b == b';');
        let decoded = end.and_then(|end| {
            let entity = &rest[1..end];
            match entity {
                "amp" => Some('&'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "lt" => Some('<'),
                "gt" => Some('>'),
                _ => entity
                    .strip_prefix("#x")
                    .or_else(|| entity.strip_prefix("#X"))
                    .and_then(|n| u32::from_str_radix(n, 16).ok())
                    .or_else(|| entity.strip_prefix('#').and_then(|n| n.parse().ok()))
                    .and_then(char::from_u32),
            }
        });
        if let (Some(end), Some(c)) = (end, decoded) {
            out.push(c);
            rest = &rest[end + 1..];
        } else {
            out.push('&');
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Read attributes rather than searching for `src=`: URLs can contain `>` and
/// `=`, and data-src/srcset or an enclosing anchor are not the image source.
pub fn html_image(text: &str) -> Option<String> {
    let mut rest = text;
    while let Some(at) = rest.find('<') {
        rest = &rest[at + 1..];
        if let Some(comment) = rest.strip_prefix("!--") {
            rest = &comment[comment.find("-->")? + 3..];
            continue;
        }
        let end = rest.find(|c: char| c.is_ascii_whitespace() || matches!(c, '/' | '>'))?;
        let tag = &rest[..end];
        rest = &rest[end..];
        if tag.eq_ignore_ascii_case("script") || tag.eq_ignore_ascii_case("style") {
            let close = format!("</{tag}").to_ascii_lowercase();
            let at = rest.to_ascii_lowercase().find(&close)?;
            rest = &rest[at + close.len()..];
            continue;
        }
        let image = tag.eq_ignore_ascii_case("img");
        while !rest.is_empty() {
            rest = rest.trim_start_matches(|c: char| c.is_ascii_whitespace() || c == '/');
            if let Some(tail) = rest.strip_prefix('>') {
                rest = tail;
                break;
            }
            let end = rest
                .find(|c: char| c.is_ascii_whitespace() || matches!(c, '=' | '>'))
                .unwrap_or(rest.len());
            let name = &rest[..end];
            rest = rest[end..].trim_start();
            if let Some(tail) = rest.strip_prefix('=') {
                rest = tail.trim_start();
                let value = if rest.starts_with(['\'', '"']) {
                    let quote = rest.as_bytes()[0] as char;
                    rest = &rest[1..];
                    let end = rest.find(quote)?;
                    let value = &rest[..end];
                    rest = &rest[end + 1..];
                    value
                } else {
                    let end = rest
                        .find(|c: char| c.is_ascii_whitespace() || c == '>')
                        .unwrap_or(rest.len());
                    let value = &rest[..end];
                    rest = &rest[end..];
                    value
                };
                if image && name.eq_ignore_ascii_case("src") {
                    return Some(entities(value).trim().to_string());
                }
            }
        }
        if image {
            return None;
        }
    }
    None
}

pub fn http_url(text: &str) -> bool {
    let Some((scheme, rest)) = text.split_once("://") else {
        return false;
    };
    (scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
        && !rest.is_empty()
        && !rest.starts_with('/')
        && !text.chars().any(char::is_whitespace)
        && !text.chars().any(char::is_control)
}

pub fn is_data_image(text: &str) -> bool {
    text.get(..11)
        .is_some_and(|s| s.eq_ignore_ascii_case("data:image/"))
}

pub fn magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if bytes.starts_with(b"BM") {
        Some("image/bmp")
    } else if bytes.starts_with(b"II\x2a\0") || bytes.starts_with(b"MM\0\x2a") {
        Some("image/tiff")
    } else if bytes.starts_with(b"\0\0\x01\0") {
        Some("image/x-icon")
    } else if bytes.get(4..8) == Some(b"ftyp")
        && matches!(bytes.get(8..12), Some(b"avif" | b"avis"))
    {
        Some("image/avif")
    } else {
        None
    }
}

pub fn data_image(uri: &str) -> Option<(Vec<u8>, &'static str)> {
    if uri.len() > MAX_TEXT || !is_data_image(uri) {
        return None;
    }
    let (header, payload) = uri[5..].split_once(',')?;
    extension(header)?;
    let mut decoded = Vec::new();
    let mut bytes = payload.bytes();
    while let Some(b) = bytes.next() {
        let b = if b == b'%' {
            let hi = (bytes.next()? as char).to_digit(16)?;
            let lo = (bytes.next()? as char).to_digit(16)?;
            (hi * 16 + lo) as u8
        } else {
            b
        };
        decoded.push(b);
        if decoded.len() > MAX_TEXT {
            return None;
        }
    }
    let bytes = if header.split(';').any(|p| p.eq_ignore_ascii_case("base64")) {
        let payload = std::str::from_utf8(&decoded).ok()?;
        if !payload.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=') || b.is_ascii_whitespace()
        }) {
            return None;
        }
        gtk::glib::base64_decode(payload)
    } else {
        decoded
    };
    if bytes.len() > MAX_IMAGE {
        return None;
    }
    let mime = magic(&bytes)?;
    Some((bytes, mime))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_then_binary_then_urls_then_text_regardless_of_offer_order() {
        let mimes = [
            "text/plain;charset=utf-8",
            "text/html",
            "image/webp",
            "text/x-moz-url",
            "image/png",
            "text/uri-list",
            "application/octet-stream;name=photo.jpg",
            "application/x-unknown",
        ];
        assert_eq!(
            pick_forms(&mimes),
            [
                "text/uri-list",
                "image/png",
                "image/webp",
                "application/octet-stream;name=photo.jpg",
                "text/x-moz-url",
                "text/html",
                "text/plain;charset=utf-8"
            ]
        );
        assert!(pick_forms(&["application/x-unknown"]).is_empty());
    }

    #[test]
    fn uri_lists_ignore_comments_and_keep_escaped_paths_and_extensionless_urls() {
        assert_eq!(
            uri_list(
                "# comment\r\n\r\nfile:///tmp/a%20b.png\r\nhttps://lh3.googleusercontent.com/abc=s2048\r\n"
            ),
            [
                "file:///tmp/a%20b.png",
                "https://lh3.googleusercontent.com/abc=s2048"
            ]
        );
        assert_eq!(
            moz_url("https://example.org/image\nPhoto title\n"),
            Some("https://example.org/image")
        );
    }

    #[test]
    fn mozilla_urls_accept_both_native_utf16_and_chromium_utf8() {
        let url = "https://example.org/雪\nTitle";
        for be in [false, true] {
            let bytes: Vec<_> = url
                .encode_utf16()
                .flat_map(|u| if be { u.to_be_bytes() } else { u.to_le_bytes() })
                .collect();
            assert_eq!(decode_text(&bytes, "text/x-moz-url").as_deref(), Some(url));
        }
        assert_eq!(
            decode_text(url.as_bytes(), "text/x-moz-url").as_deref(),
            Some(url)
        );
        assert!(decode_text(&[0xff, 0xfe, 0], "text/html;charset=utf-16").is_none());
    }

    #[test]
    fn entities_bound_lookahead_and_preserve_unicode_and_invalid_entities() {
        let text = format!("{};雪&amp;&#x1F600;&unknown;", "&".repeat(100_000));
        assert_eq!(entities(&text), format!("{};雪&😀&unknown;", "&".repeat(100_000)));
        assert_eq!(entities("&雪雪雪雪雪雪;&amp;"), "&雪雪雪雪雪雪;&");
        assert_eq!(entities("&#0000000000065;"), "A");
        assert_eq!(entities("&#00000000000065;"), "A");
        assert_eq!(entities("&#000000000000065;"), "&#000000000000065;");
    }

    #[test]
    fn html_uses_first_image_src_and_decodes_attributes() {
        assert_eq!(
            html_image(
                "<!-- <img src='wrong'> --><a href='https://page'><IMG data-src='wrong' alt='a > b' SRC = \"https://image/no-extension?a=1&amp;b=&#x32;&#51;\"></a><img src='other'>"
            ),
            Some("https://image/no-extension?a=1&b=23".into())
        );
        assert_eq!(
            html_image("<img src=https://image/a.webp>"),
            Some("https://image/a.webp".into())
        );
        assert_eq!(
            html_image("<script>let x='<img src=wrong>';</script><img src='right'>"),
            Some("right".into())
        );
        assert!(html_image("<a href='https://link'>Link</a>").is_none());
        assert!(html_image("<img data-src='wrong'><img src='other'>").is_none());
        assert!(html_image("<img src='unterminated>").is_none());
    }

    #[test]
    fn mime_extensions_and_signatures_do_not_depend_on_url_suffixes() {
        for (mime, ext, bytes) in [
            ("image/png", "png", b"\x89PNG\r\n\x1a\n".as_slice()),
            ("image/jpeg", "jpg", b"\xff\xd8\xff".as_slice()),
            ("image/gif", "gif", b"GIF89a".as_slice()),
            ("image/webp", "webp", b"RIFF\0\0\0\0WEBP".as_slice()),
        ] {
            assert_eq!(magic(bytes), Some(mime));
            assert_eq!(extension(mime), Some(ext));
        }
        assert_eq!(extension("Image/JPEG; charset=binary"), Some("jpg"));
        for bytes in [
            b"<html>Login</html>".as_slice(),
            b"RIFF\0\0\0\0WAVE",
            b"",
            b"\x89PNG",
        ] {
            assert_eq!(magic(bytes), None);
        }
        assert_eq!(extension("text/html"), None);
    }

    #[test]
    fn data_uris_decode_base64_and_percent_encoded_images() {
        let (bytes, mime) = data_image("data:image/png;base64,iVBORw0KGgo=").unwrap();
        assert_eq!(bytes, b"\x89PNG\r\n\x1a\n");
        assert_eq!(mime, "image/png");
        assert_eq!(
            data_image("data:image/gif,GIF89a%00%FF").unwrap().0,
            b"GIF89a\0\xff"
        );
        assert!(data_image("data:text/html;base64,PGh0bWw+").is_none());
        assert!(data_image("data:image/png;base64,PGh0bWw+").is_none());
        assert!(data_image("data:image/png;base64,!!!!").is_none());
        assert!(data_image("data:image/gif,GIF89a%Q0").is_none());
    }

    #[test]
    fn only_http_and_https_are_downloadable() {
        for url in [
            "https://image/no-extension",
            "http://localhost/image",
            "HTTPS://example.org/image",
        ] {
            assert!(http_url(url));
        }
        for url in [
            "file:///tmp/image",
            "ftp://example.org/image",
            "data:image/png,foo",
            "javascript:alert(1)",
            "https:///tmp",
            "https://",
            "https://example.org/\nimage",
        ] {
            assert!(!http_url(url));
        }
    }
}
