//! macOS 14+ WallpaperAgent store support. The global desktop overrides every Space.
use anyhow::{Context, Result, bail};
use plist::{Dictionary, Value};
use std::path::Path;

fn dictionary(value: &Value) -> Result<&Dictionary> {
    value
        .as_dictionary()
        .context("Unsupported wallpaper store: expected dictionary")
}

fn transform(original: &[u8], path: &Path) -> Result<Vec<u8>> {
    let mut store = Value::from_reader(std::io::Cursor::new(original))
        .context("Could not decode wallpaper store")?;
    let root = dictionary(&store)?;
    for key in ["Spaces", "Displays"] {
        let entries = dictionary(
            root.get(key)
                .with_context(|| format!("Unsupported wallpaper store: missing {key}"))?,
        )?;
        for entry in entries.values() {
            dictionary(entry)?;
        }
    }
    let global = root
        .get("AllSpacesAndDisplays")
        .context("Unsupported wallpaper store: missing global entry")?;
    match global {
        Value::Dictionary(entries) => {
            if !matches!(
                entries.get("Type").and_then(Value::as_string),
                Some("desktop" | "idle" | "individual")
            ) {
                bail!("Unsupported wallpaper store: unknown wallpaper mode");
            }
            for key in ["Desktop", "Idle"] {
                if let Some(entry) = entries.get(key) {
                    let entry = dictionary(entry)?;
                    let content = dictionary(
                        entry
                            .get("Content")
                            .context("Unsupported wallpaper entry: missing Content")?,
                    )?;
                    if content.get("Choices").and_then(Value::as_array).is_none() {
                        bail!("Unsupported wallpaper entry: missing Choices");
                    }
                }
            }
        }
        Value::String(value) if value == "$null" => {}
        _ => bail!("Unsupported wallpaper store: invalid global entry"),
    }
    let url = url::Url::from_file_path(path)
        .map_err(|_| anyhow::anyhow!("Wallpaper path must be absolute"))?;
    let config = Value::Dictionary(Dictionary::from_iter([
        ("type".to_string(), Value::String("imageFile".into())),
        (
            "url".into(),
            Value::Dictionary(Dictionary::from_iter([(
                "relative".to_string(),
                Value::String(url.into()),
            )])),
        ),
    ]));
    let mut configuration = vec![];
    config.to_writer_binary(&mut configuration)?;
    let choice = Value::Dictionary(Dictionary::from_iter([
        ("Configuration".to_string(), Value::Data(configuration)),
        ("Files".to_string(), Value::Array(vec![])),
        (
            "Provider".into(),
            Value::String("com.apple.wallpaper.choice.image".into()),
        ),
    ]));
    let content = Value::Dictionary(Dictionary::from_iter([
        ("Choices".to_string(), Value::Array(vec![choice])),
        (
            "EncodedOptionValues".to_string(),
            Value::String("$null".into()),
        ),
        ("Shuffle".to_string(), Value::String("$null".into())),
    ]));
    let now = plist::Date::from(std::time::SystemTime::now());
    let desktop = Value::Dictionary(Dictionary::from_iter([
        ("Content".to_string(), content),
        ("LastSet".to_string(), Value::Date(now)),
        ("LastUse".to_string(), Value::Date(now)),
    ]));
    let global = store
        .as_dictionary_mut()
        .unwrap()
        .get_mut("AllSpacesAndDisplays")
        .unwrap();
    if global.as_dictionary().is_none() {
        *global = Value::Dictionary(Dictionary::new());
    }
    let global = global.as_dictionary_mut().unwrap();
    global.insert("Desktop".to_string(), desktop);
    global.insert("Type".to_string(), Value::String("desktop".into()));
    let mut bytes = vec![];
    store.to_writer_binary(&mut bytes)?;
    Ok(bytes)
}

#[cfg(target_os = "macos")]
pub(super) fn supported() -> bool {
    static SUPPORTED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SUPPORTED.get_or_init(|| {
        std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|version| version.trim().split('.').next()?.parse::<u32>().ok())
            .is_some_and(|major| major >= 14)
    })
}

fn save(store: &Path, path: &Path) -> Result<std::path::PathBuf> {
    use std::{fs, io::Write};
    let metadata = fs::symlink_metadata(store).context("Could not read macOS wallpaper store")?;
    if !metadata.file_type().is_file() {
        bail!("Wallpaper store is not a regular file");
    }
    let original = fs::read(store)?;
    let updated = transform(&original, path)?;
    let directory = store.parent().unwrap();
    let mut backup = tempfile::Builder::new()
        .prefix("Index.plist.reframed-backup-")
        .tempfile_in(directory)
        .context("Could not create wallpaper backup")?;
    backup.write_all(&original)?;
    backup.as_file().set_permissions(metadata.permissions())?;
    backup.as_file().sync_all()?;
    let (_, backup_path) = backup.keep().context("Could not retain wallpaper backup")?;
    let mut replacement = tempfile::Builder::new()
        .prefix(".reframed-wallpaper-")
        .tempfile_in(directory)?;
    replacement.write_all(&updated)?;
    replacement
        .as_file()
        .set_permissions(metadata.permissions())?;
    replacement.as_file().sync_all()?;
    if fs::read(store)? != original {
        bail!(
            "Wallpaper settings changed during this operation; try again. Backup: {}",
            backup_path.display()
        );
    }
    replacement
        .persist(store)
        .context("Could not atomically replace wallpaper store")?;
    fs::File::open(directory)?
        .sync_all()
        .context("Wallpaper settings saved, but syncing the store failed")?;
    Ok(backup_path)
}

#[cfg(target_os = "macos")]
pub(super) fn apply(path: &Path) -> Result<()> {
    use std::{fs, process::Command};
    if !supported() {
        bail!("Applying wallpaper to all Desktops requires macOS 14 or newer");
    }
    let path = fs::canonicalize(path).context("Could not find wallpaper image")?;
    if !path.is_file() {
        bail!("Wallpaper image must be a file");
    }
    let home = std::env::var_os("HOME").context("Could not locate wallpaper store")?;
    let store =
        Path::new(&home).join("Library/Application Support/com.apple.wallpaper/Store/Index.plist");
    let backup_path = save(&store, &path)?;
    let uid = Command::new("/usr/bin/id")
        .arg("-u")
        .output()
        .context("Wallpaper settings saved, but user lookup failed")?;
    if !uid.status.success() {
        bail!(
            "Wallpaper settings saved, but user lookup failed. Backup: {}",
            backup_path.display()
        );
    }
    let uid = std::str::from_utf8(&uid.stdout)
        .context("Wallpaper settings saved, but user lookup was invalid")?
        .trim();
    if uid.parse::<u32>().is_err() {
        bail!("Wallpaper settings saved, but user lookup was invalid");
    }
    let restart = Command::new("/usr/bin/killall")
        .args(["-u", uid, "WallpaperAgent"])
        .output()
        .context("Wallpaper settings saved, but WallpaperAgent could not be restarted")?;
    if !restart.status.success() {
        bail!(
            "Wallpaper settings saved, but WallpaperAgent restart failed: {}. Backup: {}",
            String::from_utf8_lossy(&restart.stderr).trim(),
            backup_path.display()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn atomic_save_retains_unique_original_backups() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("Index.plist");
        let original = encode(&fixture());
        std::fs::write(&store, &original).unwrap();
        let first = save(&store, Path::new("/first.png")).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), original);
        let previous = std::fs::read(&store).unwrap();
        assert!(previous.starts_with(b"bplist00"));
        let second = save(&store, Path::new("/second.png")).unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read(&first).unwrap(), original);
        assert_eq!(std::fs::read(&second).unwrap(), previous);
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 3);
    }
    #[test]
    fn malformed_store_is_untouched_without_backup() {
        let directory = tempfile::tempdir().unwrap();
        let store = directory.path().join("Index.plist");
        std::fs::write(&store, b"invalid").unwrap();
        assert!(save(&store, Path::new("/new.png")).is_err());
        assert_eq!(std::fs::read(&store).unwrap(), b"invalid");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
    fn fixture() -> Value {
        let idle = Value::Dictionary(Dictionary::from_iter([(
            "Content".to_string(),
            Value::Dictionary(Dictionary::from_iter([(
                "Choices".to_string(),
                Value::Array(vec![]),
            )])),
        )]));
        Value::Dictionary(Dictionary::from_iter([
            (
                "AllSpacesAndDisplays".into(),
                Value::Dictionary(Dictionary::from_iter([
                    ("Type".to_string(), Value::String("idle".into())),
                    ("Idle".to_string(), idle),
                ])),
            ),
            (
                "Spaces".into(),
                Value::Dictionary(Dictionary::from_iter([(
                    "uuid".to_string(),
                    Value::Dictionary(Dictionary::new()),
                )])),
            ),
            ("Displays".to_string(), Value::Dictionary(Dictionary::new())),
            ("Unrelated".to_string(), Value::String("preserved".into())),
        ]))
    }
    fn encode(value: &Value) -> Vec<u8> {
        let mut bytes = vec![];
        value.to_writer_binary(&mut bytes).unwrap();
        bytes
    }
    #[test]
    fn preserves_idle_overrides_and_other_settings() {
        let old = fixture();
        let new = Value::from_reader(std::io::Cursor::new(
            transform(&encode(&old), Path::new("/new.png")).unwrap(),
        ))
        .unwrap();
        for key in ["Spaces", "Displays", "Unrelated"] {
            assert_eq!(
                dictionary(&old).unwrap()[key],
                dictionary(&new).unwrap()[key]
            );
        }
        let old_global = dictionary(&old).unwrap()["AllSpacesAndDisplays"]
            .as_dictionary()
            .unwrap();
        let new_global = dictionary(&new).unwrap()["AllSpacesAndDisplays"]
            .as_dictionary()
            .unwrap();
        assert_eq!(old_global["Idle"], new_global["Idle"]);
        assert_eq!(new_global["Type"].as_string(), Some("desktop"));
    }
    #[test]
    fn rejects_unknown_and_malformed_store() {
        assert!(transform(b"not a plist", Path::new("/new.png")).is_err());
        let mut value = fixture();
        value
            .as_dictionary_mut()
            .unwrap()
            .insert("Displays".to_string(), Value::Array(vec![]));
        assert!(transform(&encode(&value), Path::new("/new.png")).is_err());
        let mut value = fixture();
        value
            .as_dictionary_mut()
            .unwrap()
            .get_mut("AllSpacesAndDisplays")
            .unwrap()
            .as_dictionary_mut()
            .unwrap()
            .insert("Type".to_string(), Value::String("future".into()));
        assert!(transform(&encode(&value), Path::new("/new.png")).is_err());
    }
    #[test]
    fn accepts_null_global_and_encodes_special_paths() {
        let mut value = fixture();
        value.as_dictionary_mut().unwrap().insert(
            "AllSpacesAndDisplays".to_string(),
            Value::String("$null".into()),
        );
        let path = Path::new("/Pictures/a #?% ü.png");
        let updated = Value::from_reader(std::io::Cursor::new(
            transform(&encode(&value), path).unwrap(),
        ))
        .unwrap();
        let global = dictionary(&updated).unwrap()["AllSpacesAndDisplays"]
            .as_dictionary()
            .unwrap();
        let content = global["Desktop"].as_dictionary().unwrap()["Content"]
            .as_dictionary()
            .unwrap();
        let bytes = content["Choices"].as_array().unwrap()[0]
            .as_dictionary()
            .unwrap()["Configuration"]
            .as_data()
            .unwrap();
        let config = Value::from_reader(std::io::Cursor::new(bytes)).unwrap();
        let relative = dictionary(&config).unwrap()["url"].as_dictionary().unwrap()["relative"]
            .as_string()
            .unwrap();
        assert_eq!(
            url::Url::parse(relative).unwrap().to_file_path().unwrap(),
            path
        );
        assert!(relative.contains("%23%3F%25"));
    }
}
