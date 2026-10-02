//! Platform locations for retained originals and expendable previews.
use anyhow::{Context, Result};
use std::path::PathBuf;

fn resolve_directory(
    value: Option<PathBuf>,
    home: Option<PathBuf>,
    fallback: &str,
) -> Result<PathBuf> {
    if let Some(path) = value.filter(|path| path.is_absolute()) {
        return Ok(path.join("reframed"));
    }
    let home = home.context("HOME is unavailable")?;
    anyhow::ensure!(home.is_absolute(), "HOME must be an absolute path");
    Ok(home.join(fallback).join("reframed"))
}
fn directory(variable: &str, fallback: &str) -> Result<PathBuf> {
    resolve_directory(
        std::env::var_os(variable).map(PathBuf::from),
        std::env::var_os("HOME").map(PathBuf::from),
        fallback,
    )
}
pub fn data_dir() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").context("HOME is unavailable")?;
        Ok(PathBuf::from(home).join("Library/Application Support/Reframed"))
    } else {
        directory("XDG_DATA_HOME", ".local/share")
    }
}
pub fn cache_dir() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").context("HOME is unavailable")?;
        Ok(PathBuf::from(home).join("Library/Caches/Reframed"))
    } else {
        directory("XDG_CACHE_HOME", ".cache")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absolute_xdg_does_not_require_home() {
        assert_eq!(
            resolve_directory(Some("/data".into()), None, ".cache").unwrap(),
            PathBuf::from("/data/reframed")
        );
    }
    #[test]
    fn absent_empty_and_relative_xdg_use_home() {
        for value in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                resolve_directory(value, Some("/home/person".into()), ".cache").unwrap(),
                PathBuf::from("/home/person/.cache/reframed")
            );
        }
        assert!(resolve_directory(None, None, ".cache").is_err());
        assert!(resolve_directory(None, Some("relative".into()), ".cache").is_err());
    }
}
