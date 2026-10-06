use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
pub fn validate_url(raw: &str) -> Result<()> {
    let u = url::Url::parse(raw)?;
    if u.scheme() != "https"
        || u.host_str() != Some("cdn.reframed.gallery")
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || !u.path().starts_with("/originals/")
    {
        bail!("Unexpected original image URL");
    }
    Ok(())
}
pub fn cache_dir(original: bool) -> Result<PathBuf> {
    let path = if original {
        crate::storage::data_dir()?.join("originals")
    } else {
        crate::storage::cache_dir()?.join("previews")
    };
    std::fs::create_dir_all(&path)?;
    Ok(path)
}
pub fn fetch(url: &str, original: bool) -> Result<(PathBuf, u32, u32)> {
    if original {
        validate_url(url)?;
    }
    let dir = cache_dir(original)?;
    let key = format!("{:x}", Sha256::digest(url.as_bytes()));
    for extension in ["jpg", "png", "webp"] {
        let path = dir.join(format!("{key}.{extension}"));
        if path.exists()
            && let Ok((w, h, _)) = dimensions(&path)
        {
            return Ok((path, w, h));
        }
    }
    let response = crate::catalogue::agent()
        .get(url)
        .set("User-Agent", "Mozilla/5.0")
        .set("Referer", "https://www.reframed.gallery/")
        .call()
        .map_err(|error| {
            let message = request_error_message(&error);
            anyhow::Error::new(error).context(message)
        })?;
    let limit = if original {
        100 * 1024 * 1024
    } else {
        12 * 1024 * 1024
    };
    if response
        .header("Content-Length")
        .and_then(|s| s.parse::<u64>().ok())
        .is_some_and(|n| n > limit)
    {
        bail!("Image exceeds download size limit");
    }
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = dir.join(format!(
        "{key}.{}.{nanos}.{serial}.part",
        std::process::id()
    ));
    let result = (|| -> Result<(PathBuf, u32, u32)> {
        let mut reader = response.into_reader().take(limit + 1);
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let n = std::io::copy(&mut reader, &mut file)?;
        file.flush()?;
        if n > limit {
            bail!("Image exceeds download size limit");
        }
        let (w, h, extension) = dimensions(&temp)?;
        let path = dir.join(format!("{key}.{extension}"));
        std::fs::rename(&temp, &path)?;
        Ok((path, w, h))
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}
fn request_error_message(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Status(403, _) => "The image server denied access. Try again later".into(),
        ureq::Error::Status(429, _) => {
            "The image server is limiting requests. Try again later".into()
        }
        ureq::Error::Status(404 | 410, _) => "The original image is no longer available".into(),
        ureq::Error::Status(status, _) => {
            format!("The image server returned HTTP {status}. Try again later")
        }
        ureq::Error::Transport(error) => transport_error_message(error.kind()).into(),
    }
}
fn transport_error_message(kind: ureq::ErrorKind) -> &'static str {
    match kind {
        ureq::ErrorKind::Dns => {
            "Could not resolve the image server. Check your connection and retry"
        }
        ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Io => {
            "Could not connect to the image server. Check your connection and retry"
        }
        _ => "Image download failed. Try again later",
    }
}
fn dimensions(path: &Path) -> Result<(u32, u32, &'static str)> {
    let reader = image::ImageReader::open(path)?.with_guessed_format()?;
    let extension = match reader.format() {
        Some(image::ImageFormat::Jpeg) => "jpg",
        Some(image::ImageFormat::Png) => "png",
        Some(image::ImageFormat::WebP) => "webp",
        _ => bail!("Unsupported downloaded image format"),
    };
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(20000);
    limits.max_image_height = Some(20000);
    limits.max_alloc = Some(512 * 1024 * 1024);
    let mut reader = reader;
    reader.limits(limits);
    let decoded = reader
        .decode()
        .context("Downloaded file is not a supported image")?;
    Ok((decoded.width(), decoded.height(), extension))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn allows_only_original_https_source() {
        assert!(
            validate_url("https://cdn.reframed.gallery/originals/Artist%20-%20Title.jpg").is_ok()
        );
        for u in [
            "http://cdn.reframed.gallery/originals/a.jpg",
            "https://evil.example/originals/a.jpg",
            "https://cdn.reframed.gallery/cdn-cgi/image/width=700/originals/a.jpg",
            "https://user@cdn.reframed.gallery/originals/a.jpg",
            "file:///etc/passwd",
            "https://cdn.reframed.gallery/originals/../secret",
        ] {
            assert!(validate_url(u).is_err(), "{u}");
        }
    }
    #[test]
    fn network_failures_do_not_claim_access_was_blocked() {
        assert_eq!(
            transport_error_message(ureq::ErrorKind::Dns),
            "Could not resolve the image server. Check your connection and retry"
        );
        let error = ureq::Error::Status(403, ureq::Response::new(403, "Forbidden", "").unwrap());
        assert_eq!(
            request_error_message(&error),
            "The image server denied access. Try again later"
        );
        let error = ureq::Error::Status(404, ureq::Response::new(404, "Not Found", "").unwrap());
        assert_eq!(
            request_error_message(&error),
            "The original image is no longer available"
        );
    }
    #[test]
    fn html_is_not_accepted_as_wallpaper() {
        let path =
            std::env::temp_dir().join(format!("pinacora-invalid-image-{}", std::process::id()));
        std::fs::write(&path, b"<html>Access denied</html>").unwrap();
        let result = dimensions(&path);
        let _ = std::fs::remove_file(path);
        assert!(result.is_err());
    }
}
