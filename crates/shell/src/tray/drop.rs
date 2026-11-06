use super::drop_data::{self as data, MAX_IMAGE, MAX_TEXT};
use gtk::{gdk, gio, glib, prelude::*};
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

pub fn accepts_drop(drop: &gdk::Drop) -> bool {
    let formats = drop.formats();
    let mimes = formats.mime_types();
    !data::pick_forms(&mimes.iter().map(|m| m.as_str()).collect::<Vec<_>>()).is_empty()
        || formats
            .clone()
            .union_deserialize_types()
            .contains_type(gdk::FileList::static_type())
        || formats.contains_type(gdk::Texture::static_type())
        || formats.contains_type(String::static_type())
}

async fn timed<T>(future: impl std::future::Future<Output = T>) -> Option<T> {
    use futures_util::future::{Either, select};
    let timeout = glib::timeout_future(Duration::from_secs(30));
    futures_util::pin_mut!(future);
    match select(future, timeout).await {
        Either::Left((value, _)) => Some(value),
        Either::Right(_) => None,
    }
}

async fn read(drop: &gdk::Drop, mime: &str, cap: usize) -> Option<Vec<u8>> {
    timed(async {
        let (stream, _) = drop
            .read_future(&[mime], glib::Priority::DEFAULT)
            .await
            .ok()?;
        let mut bytes = Vec::new();
        let result = loop {
            let chunk = match stream
                .read_bytes_future(
                    (cap + 1 - bytes.len()).min(64 * 1024),
                    glib::Priority::DEFAULT,
                )
                .await
            {
                Ok(chunk) => chunk,
                Err(_) => break None,
            };
            if chunk.is_empty() {
                break Some(bytes);
            }
            bytes.extend_from_slice(&chunk);
            if bytes.len() > cap {
                break None;
            }
        };
        let _ = stream.close_future(glib::Priority::DEFAULT).await;
        result
    })
    .await
    .flatten()
}

fn keep_image(bytes: &[u8]) -> Option<PathBuf> {
    if bytes.len() > MAX_IMAGE {
        return None;
    }
    let ext = data::extension(data::magic(bytes)?)?;
    // Exclusive names avoid replacing another drop made within the same second.
    let path = super::fresh(&format!(
        "Image {} {}.{ext}",
        super::stamp(),
        glib::uuid_string_random()
    ))?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .ok()?;
    if file.write_all(bytes).and_then(|_| file.sync_all()).is_err() {
        let _ = std::fs::remove_file(&path);
        return None;
    }
    Some(path)
}

fn fetch_image(url: &str, cap: usize) -> Option<Vec<u8>> {
    if !data::http_url(url) {
        return None;
    }
    // Same HTTP client as weather.rs. Limit protocols for both the initial URL
    // and every redirect; never import browser cookies or other session secrets.
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--fail",
            "--location",
            "--max-redirs",
            "10",
            "--max-time",
            "30",
            "--proto",
            "=http,https",
            "--proto-redir",
            "=http,https",
            "--max-filesize",
            &cap.to_string(),
            "--url",
            url,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()?
        .take(cap as u64 + 1)
        .read_to_end(&mut bytes);
    if read.is_err() || bytes.len() > cap {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    }
    if !child.wait().ok()?.success() {
        return None;
    }
    // Signature-based validation also handles extensionless Google Photos URLs
    // and rejects login/HTML responses regardless of their claimed Content-Type.
    data::magic(&bytes)?;
    Some(bytes)
}

fn download(url: &str) -> Option<PathBuf> {
    keep_image(&fetch_image(url, MAX_IMAGE)?)
}

fn local_files(text: &str) -> Vec<PathBuf> {
    data::uri_list(text)
        .into_iter()
        .filter_map(|uri| {
            let file = gio::File::for_uri(uri);
            file.is_native()
                .then(|| file.path())
                .flatten()
                .filter(|p| p.exists())
        })
        .collect()
}

fn image_link(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    [
        ".png", ".jpg", ".jpeg", ".webp", ".gif", ".bmp", ".tiff", ".avif",
    ]
    .iter()
    .any(|ext| path.ends_with(ext))
        || url
            .split("://")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .is_some_and(|host| {
                host == "photos.google.com" || host.ends_with(".googleusercontent.com")
            })
}

/// Read all useful forms before finishing the Wayland drop. Binary forms win
/// over URLs, so authenticated browser images need no independent HTTP request.
pub async fn receive_drop(drop: &gdk::Drop) -> Vec<PathBuf> {
    let formats = drop.formats();
    let mimes = formats.mime_types();
    let refs: Vec<_> = mimes.iter().map(|m| m.as_str()).collect();
    let forms = data::pick_forms(&refs);
    let native = formats.clone().union_deserialize_types();
    if native.contains_type(gdk::FileList::static_type())
        && let Some(Ok(value)) =
            timed(drop.read_value_future(gdk::FileList::static_type(), glib::Priority::DEFAULT))
                .await
        && let Ok(list) = value.get::<gdk::FileList>()
    {
        let paths: Vec<_> = list
            .files()
            .iter()
            .filter_map(|f| f.path())
            .filter(|p| p.exists())
            .collect();
        if !paths.is_empty() {
            return paths;
        }
    }

    let mut texts = Vec::new();
    let mut image_intent = false;
    for mime in &forms {
        let base = data::mime_base(mime).to_ascii_lowercase();
        let binary = base.starts_with("image/") || base == "application/octet-stream";
        image_intent |= binary;
        let Some(bytes) = read(drop, mime, if binary { MAX_IMAGE } else { MAX_TEXT }).await else {
            continue;
        };
        if binary {
            if let Ok(Some(path)) = gio::spawn_blocking(move || keep_image(&bytes)).await {
                return vec![path];
            }
        } else if let Some(text) = data::decode_text(&bytes, mime) {
            // URI lists come before binary forms, but only local paths win here.
            if base == "text/uri-list" {
                let paths = local_files(&text);
                if !paths.is_empty() {
                    return paths;
                }
            }
            texts.push((base, text));
        }
    }
    if formats.contains_type(gdk::Texture::static_type())
        && let Some(Ok(value)) =
            timed(drop.read_value_future(gdk::Texture::static_type(), glib::Priority::DEFAULT))
                .await
        && let Ok(texture) = value.get::<gdk::Texture>()
    {
        image_intent = true;
        if let Some(path) = super::keep_texture(&texture) {
            return vec![path];
        }
    }
    if !texts
        .iter()
        .any(|(mime, _)| mime == "text/plain" || mime == "utf8_string")
        && formats.contains_type(String::static_type())
        && let Some(Ok(value)) =
            timed(drop.read_value_future(String::static_type(), glib::Priority::DEFAULT)).await
        && let Ok(text) = value.get::<String>()
    {
        let paths = local_files(&text);
        if !paths.is_empty() {
            return paths;
        }
        texts.push(("text/plain".into(), text));
    }

    let mut candidates = Vec::new();
    let mut plain = None;
    let mut html_source = None;
    for (mime, text) in &texts {
        match mime.as_str() {
            "text/uri-list" => {
                candidates.extend(data::uri_list(text).into_iter().map(str::to_string))
            }
            "text/x-moz-url" | "text/x-moz-url-data" => {
                if let Some(url) = data::moz_url(text) {
                    candidates.push(url.to_string());
                }
            }
            "text/html" => {
                if let Some(src) = data::html_image(text) {
                    html_source = Some(src);
                }
            }
            "text/plain" | "utf8_string" => {
                plain = Some(text.clone());
                if text.trim().lines().count() == 1 {
                    candidates.push(text.trim().to_string());
                }
            }
            _ => (),
        }
    }
    // An enclosing anchor can offer its page URL in uri-list; the actual img
    // source is the better candidate when HTML supplies one.
    if let Some(src) = html_source {
        image_intent = true;
        candidates.insert(0, src);
    }
    let has_html = texts.iter().any(|(mime, _)| mime == "text/html");
    image_intent |= candidates
        .iter()
        .any(|s| data::is_data_image(s) || (!has_html && image_link(s)));
    for uri in &candidates {
        if data::is_data_image(uri) {
            let uri = uri.clone();
            if let Ok(Some(path)) = gio::spawn_blocking(move || {
                let (bytes, _) = data::data_image(&uri)?;
                keep_image(&bytes)
            })
            .await
            {
                return vec![path];
            }
        }
    }
    let mut tried = Vec::new();
    for url in candidates.iter().filter(|s| data::http_url(s)) {
        if tried.contains(url) {
            continue;
        }
        tried.push(url.clone());
        let url = url.clone();
        if let Ok(Some(path)) = gio::spawn_blocking(move || download(&url)).await {
            return vec![path];
        }
        // A failed image import must not become a link to a login page or a
        // different image from another representation of the dragged element.
        if image_intent {
            return Vec::new();
        }
    }
    if image_intent {
        return Vec::new();
    }
    let fallback = plain.or_else(|| candidates.first().cloned());
    if let Some(text) = fallback {
        return gio::spawn_blocking(move || super::keep_text(&text))
            .await
            .unwrap_or_default();
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server(responses: Vec<Vec<u8>>) -> (String, std::thread::JoinHandle<()>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for response in responses {
                let until = std::time::Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(std::time::Instant::now() < until, "curl did not connect");
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        Err(e) => panic!("{e}"),
                    }
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        break;
                    }
                    request.push(byte[0]);
                }
                // Rejection/size limits may close the socket during this write.
                let _ = stream.write_all(&response);
            }
        });
        (url, worker)
    }

    #[test]
    fn downloads_follow_redirects_and_use_magic_for_extensionless_images() {
        let image = b"\x89PNG\r\n\x1a\n";
        let mut ok = b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n".to_vec();
        ok.extend_from_slice(image);
        let redirect = b"HTTP/1.1 302 Found\r\nLocation: /no-extension\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
        let (url, worker) = server(vec![redirect, ok]);
        assert_eq!(fetch_image(&url, 1024).as_deref(), Some(image.as_slice()));
        worker.join().unwrap();
    }

    #[test]
    fn downloads_reject_html_http_errors_and_non_http_redirects() {
        for response in [
            "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nConnection: close\r\n\r\n<html>Sign in</html>",
            "HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n",
            "HTTP/1.1 302 Found\r\nLocation: file:///etc/passwd\r\nConnection: close\r\n\r\n",
        ] {
            let (url, worker) = server(vec![response.as_bytes().to_vec()]);
            assert!(fetch_image(&url, 1024).is_none());
            worker.join().unwrap();
        }
    }

    #[test]
    fn downloads_reject_oversized_bodies_with_and_without_content_length() {
        for length in ["Content-Length: 8192\r\n", ""] {
            let mut response =
                format!("HTTP/1.1 200 OK\r\n{length}Connection: close\r\n\r\n").into_bytes();
            response.extend_from_slice(b"\x89PNG\r\n\x1a\n");
            response.resize(response.len() + 8192, 0);
            let (url, worker) = server(vec![response]);
            assert!(fetch_image(&url, 1024).is_none());
            worker.join().unwrap();
        }
    }

    #[test]
    fn image_links_include_google_photos_without_file_extensions() {
        assert!(image_link("https://lh3.googleusercontent.com/pw/abc=w2048"));
        assert!(image_link("https://photos.google.com/photo/abc"));
        assert!(image_link("https://example.org/image.JPG?token=abc"));
        assert!(!image_link("https://example.org/page"));
    }

    #[test]
    fn local_uris_keep_native_paths_and_skip_remote_or_missing_files() {
        // No configuration or tray writes, and no display required.
        let dir = std::env::temp_dir();
        let uri = gio::File::for_path(&dir).uri();
        assert_eq!(
            local_files(&format!(
                "# a directory\n{uri}\nhttps://example.org/photo\nfile:///nonexistent/digit-drop-test"
            )),
            vec![dir]
        );
    }
}
