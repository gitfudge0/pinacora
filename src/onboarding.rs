//! Versioned, atomic local preferences for the introductory walkthrough.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};

pub const VERSION: u32 = 1;
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Completed,
    Skipped,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Flag {
    version: u32,
    outcome: Outcome,
}

pub fn default_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is unavailable")?;
    Ok(PathBuf::from(home).join("Library/Application Support/Reframed/walkthrough.json"))
}
pub fn dismissed(path: &Path) -> Result<bool> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    let mut bytes = Vec::new();
    file.take(4097).read_to_end(&mut bytes)?;
    if bytes.len() > 4096 {
        anyhow::bail!("Walkthrough preferences exceed the size limit");
    }
    let flag: Flag = serde_json::from_slice(&bytes).context("Invalid walkthrough preferences")?;
    Ok(flag.version == VERSION)
}
pub fn save(path: &Path, outcome: Outcome) -> Result<()> {
    let parent = path.parent().context("Invalid preferences path")?;
    std::fs::create_dir_all(parent)?;
    static SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let serial = SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let temp = parent.join(format!(
        ".walkthrough-{}-{nanos}-{serial}.tmp",
        std::process::id()
    ));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        let data = serde_json::to_vec(&Flag {
            version: VERSION,
            outcome,
        })?;
        file.write_all(&data)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let p = std::env::temp_dir().join(format!(
                "reframed-onboarding-tests-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&p).unwrap();
            Self(p)
        }
        fn flag(&self) -> PathBuf {
            self.0.join("walkthrough.json")
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn first_launch_and_both_dismissal_outcomes() {
        let temp = Temp::new();
        assert!(!dismissed(&temp.flag()).unwrap());
        for outcome in [Outcome::Completed, Outcome::Skipped] {
            save(&temp.flag(), outcome).unwrap();
            assert!(dismissed(&temp.flag()).unwrap());
            let flag: Flag = serde_json::from_slice(&std::fs::read(temp.flag()).unwrap()).unwrap();
            assert_eq!(flag.outcome, outcome);
        }
        assert_eq!(std::fs::read_dir(&temp.0).unwrap().count(), 1);
    }
    #[test]
    fn changed_versions_and_malformed_flags_do_not_skip_intro() {
        let temp = Temp::new();
        for raw in [
            r#"{"version":0,"outcome":"completed"}"#,
            r#"{"version":2,"outcome":"skipped"}"#,
        ] {
            std::fs::write(temp.flag(), raw).unwrap();
            assert!(!dismissed(&temp.flag()).unwrap());
        }
        for raw in [
            "",
            r#"{"version":1}"#,
            r#"{"version":1,"outcome":"unknown"}"#,
            r#"{"version":1,"outcome":"completed","unexpected":true}"#,
        ] {
            std::fs::write(temp.flag(), raw).unwrap();
            assert!(dismissed(&temp.flag()).is_err());
        }
        std::fs::write(temp.flag(), vec![b'x'; 5000]).unwrap();
        assert!(dismissed(&temp.flag()).is_err());
        save(&temp.flag(), Outcome::Skipped).unwrap();
        assert!(dismissed(&temp.flag()).unwrap());
    }
    #[test]
    fn persistence_errors_are_returned_without_partial_flags() {
        let temp = Temp::new();
        let blocking = temp.0.join("not-a-directory");
        std::fs::write(&blocking, b"x").unwrap();
        assert!(save(&blocking.join("walkthrough.json"), Outcome::Completed).is_err());
        assert!(!temp.flag().exists());
    }
}
