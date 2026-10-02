//! Platform locations for retained originals and expendable previews.
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

// Keep existing directories in place: macOS and Linux may retain wallpaper URLs.
fn app_directory(base: &Path, current: &str, legacy: &str) -> PathBuf {
    let legacy = base.join(legacy);
    if legacy.is_dir() {
        legacy
    } else {
        base.join(current)
    }
}

fn resolve_directory(
    value: Option<PathBuf>,
    home: Option<PathBuf>,
    fallback: &str,
) -> Result<PathBuf> {
    if let Some(path) = value.filter(|path| path.is_absolute()) {
        return Ok(app_directory(&path, "pinacora", "reframed"));
    }
    let home = home.context("HOME is unavailable")?;
    anyhow::ensure!(home.is_absolute(), "HOME must be an absolute path");
    Ok(app_directory(&home.join(fallback), "pinacora", "reframed"))
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
        Ok(app_directory(
            &PathBuf::from(home).join("Library/Application Support"),
            "Pinacora",
            "Reframed",
        ))
    } else {
        directory("XDG_DATA_HOME", ".local/share")
    }
}
pub fn cache_dir() -> Result<PathBuf> {
    if cfg!(target_os = "macos") {
        let home = std::env::var_os("HOME").context("HOME is unavailable")?;
        Ok(app_directory(
            &PathBuf::from(home).join("Library/Caches"),
            "Pinacora",
            "Reframed",
        ))
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
            PathBuf::from("/data/pinacora")
        );
    }
    #[test]
    fn absent_empty_and_relative_xdg_use_home() {
        for value in [None, Some("".into()), Some("relative".into())] {
            assert_eq!(
                resolve_directory(value, Some("/home/person".into()), ".cache").unwrap(),
                PathBuf::from("/home/person/.cache/pinacora")
            );
        }
        assert!(resolve_directory(None, None, ".cache").is_err());
        assert!(resolve_directory(None, Some("relative".into()), ".cache").is_err());
    }
    #[test]
    fn new_installations_use_pinacora_directories() {
        let root = tempfile::tempdir().unwrap();
        for (current, legacy) in [("pinacora", "reframed"), ("Pinacora", "Reframed")] {
            assert_eq!(
                app_directory(root.path(), current, legacy),
                root.path().join(current)
            );
        }
    }
    #[test]
    fn existing_legacy_directories_take_precedence_without_moving_files() {
        let root = tempfile::tempdir().unwrap();
        for (current, legacy) in [("pinacora", "reframed"), ("Pinacora", "Reframed")] {
            let old = root.path().join(legacy);
            std::fs::create_dir(&old).unwrap();
            std::fs::create_dir(root.path().join(current)).unwrap();
            std::fs::write(old.join("walkthrough.json"), "existing preference").unwrap();
            assert_eq!(app_directory(root.path(), current, legacy), old);
            assert_eq!(
                std::fs::read_to_string(old.join("walkthrough.json")).unwrap(),
                "existing preference"
            );
        }
    }
}
