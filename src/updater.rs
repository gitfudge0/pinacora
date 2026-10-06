//! Verified, user-initiated updates. Checking never changes the installation.
use anyhow::{Context, Result, bail, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::os::unix::{fs::PermissionsExt, process::CommandExt};
use std::{
    fs,
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;
const MAX_EXPANDED: u64 = 1024 * 1024 * 1024;
const REPOSITORY: &str = "gitfudge0/pinacora";
#[cfg(target_os = "macos")]
const CERTIFICATE: &str = "8a432625e90ddbe5053d595cbad2ba712b976643";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub html_url: String,
    asset: Asset,
    checksum: Option<Asset>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
    size: u64,
}
#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<Asset>,
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(60))
        .redirects(0)
        .build()
}
fn get(url: &str, limit: u64, redirects: bool) -> Result<Vec<u8>> {
    let agent = if redirects {
        ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(120))
            .redirects(4)
            .https_only(true)
            .build()
    } else {
        agent()
    };
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            concat!("Pinacora/", env!("CARGO_PKG_VERSION")),
        )
        .set("Accept", "application/vnd.github+json")
        .call()
        .context("Could not contact GitHub; check your connection and retry")?;
    if redirects {
        let final_url = url::Url::parse(response.get_url())?;
        ensure!(
            matches!(
                final_url.host_str(),
                Some(
                    "github.com"
                        | "release-assets.githubusercontent.com"
                        | "objects.githubusercontent.com"
                )
            ),
            "Unexpected release download host"
        );
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "Release response exceeds size limit"
    );
    Ok(bytes)
}
fn asset_name(version: &str) -> Result<String> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(format!("Pinacora-{version}-macos-arm64.zip")),
        ("macos", "x86_64") => Ok(format!("Pinacora-{version}-macos-x86_64.zip")),
        ("linux", "x86_64") => Ok(format!("Pinacora-{version}-linux-x86_64.tar.gz")),
        _ => bail!(
            "In-app updates are unavailable on this platform; download a compatible release manually"
        ),
    }
}
fn asset_url(asset: &Asset, tag: &str) -> Result<()> {
    let u = url::Url::parse(&asset.browser_download_url)?;
    ensure!(
        u.scheme() == "https"
            && u.host_str() == Some("github.com")
            && u.port().is_none()
            && u.username().is_empty()
            && u.password().is_none()
            && u.query().is_none()
            && u.fragment().is_none(),
        "Invalid release asset URL"
    );
    ensure!(
        u.path() == format!("/{REPOSITORY}/releases/download/{tag}/{}", asset.name),
        "Release asset does not belong to the selected release"
    );
    Ok(())
}
fn select(api: ApiRelease, current: &str) -> Result<Option<Release>> {
    ensure!(
        !api.draft && !api.prerelease,
        "GitHub returned an unpublished or preview release"
    );
    let version = api.tag_name.strip_prefix('v').unwrap_or(&api.tag_name);
    let parsed = Version::parse(version).context("Release has an invalid version")?;
    ensure!(
        parsed.pre.is_empty() && parsed.build.is_empty(),
        "Only stable releases are supported"
    );
    if parsed <= Version::parse(current)? {
        return Ok(None);
    }
    ensure!(
        api.html_url
            == format!(
                "https://github.com/{REPOSITORY}/releases/tag/{}",
                api.tag_name
            ),
        "Invalid release page URL"
    );
    let name = asset_name(version)?;
    let assets: Vec<_> = api.assets.iter().filter(|a| a.name == name).collect();
    ensure!(
        assets.len() == 1,
        "This release does not yet contain a compatible app; try again later"
    );
    let asset = assets[0].clone();
    ensure!(
        asset.size > 0 && asset.size <= MAX_ARCHIVE,
        "Release archive has an invalid size"
    );
    asset_url(&asset, &api.tag_name)?;
    let sums: Vec<_> = api
        .assets
        .iter()
        .filter(|a| a.name == format!("{name}.sha256"))
        .collect();
    ensure!(sums.len() <= 1, "Ambiguous release checksum");
    let checksum = sums.first().map(|a| (*a).clone());
    if let Some(sum) = &checksum {
        asset_url(sum, &api.tag_name)?;
    }
    ensure!(
        checksum.is_some() || asset.digest.as_deref().is_some_and(valid_digest),
        "This release has no SHA-256 checksum; it cannot be installed safely"
    );
    Ok(Some(Release {
        version: version.into(),
        html_url: api.html_url,
        asset,
        checksum,
    }))
}
fn valid_digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub fn check() -> Result<Option<Release>> {
    let bytes = get(
        &format!("https://api.github.com/repos/{REPOSITORY}/releases/latest"),
        2 * 1024 * 1024,
        false,
    )?;
    select(serde_json::from_slice(&bytes)?, env!("CARGO_PKG_VERSION"))
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn verify_archive(bytes: &[u8], expected: &str) -> Result<()> {
    ensure!(
        expected.len() == 64 && expected.bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid SHA-256 checksum"
    );
    ensure!(
        digest(bytes).eq_ignore_ascii_case(expected),
        "Release checksum mismatch; nothing was installed"
    );
    Ok(())
}
fn safe_path(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty()
            && path.components().all(|c| matches!(c, Component::Normal(_))),
        "Unsafe archive path"
    );
    Ok(())
}
fn extract(bytes: &[u8], destination: &Path, zip_archive: bool) -> Result<()> {
    extract_with_limit(bytes, destination, zip_archive, MAX_EXPANDED)
}
fn extract_with_limit(
    bytes: &[u8],
    destination: &Path,
    zip_archive: bool,
    limit: u64,
) -> Result<()> {
    let mut total = 0u64;
    let mut files = 0usize;
    if zip_archive {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))?;
        ensure!(archive.len() <= 20000, "Too many archive entries");
        for i in 0..archive.len() {
            let mut entry = archive.by_index(i)?;
            let path = PathBuf::from(entry.name());
            safe_path(&path)?;
            let mode = entry.unix_mode().unwrap_or(0o644);
            ensure!(
                mode & 0o170000 != 0o120000,
                "Archive symlinks are not supported"
            );
            ensure!(
                mode & 0o170000 == 0 || matches!(mode & 0o170000, 0o100000 | 0o040000),
                "Unsupported archive entry"
            );
            let declared = entry.size();
            ensure!(
                declared <= limit.saturating_sub(total),
                "Expanded archive exceeds size limit"
            );
            let target = destination.join(&path);
            if entry.is_dir() {
                fs::create_dir_all(&target)?;
            } else {
                fs::create_dir_all(target.parent().unwrap())?;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                let copied = std::io::copy(&mut entry.by_ref().take(declared), &mut output)?;
                let mut excess = [0u8; 1];
                ensure!(
                    copied == declared && entry.read(&mut excess)? == 0,
                    "Archive entry does not match its declared size"
                );
                total = total.checked_add(copied).context("Archive size overflow")?;
                ensure!(total <= limit, "Expanded archive exceeds size limit");
                fs::set_permissions(target, fs::Permissions::from_mode(mode & 0o755))?;
            }
        }
    } else {
        let decoder = flate2::read::GzDecoder::new(bytes);
        let mut archive = tar::Archive::new(decoder);
        for entry in archive.entries()? {
            files += 1;
            ensure!(files <= 20000, "Too many archive entries");
            let mut entry = entry?;
            let path = entry.path()?.into_owned();
            safe_path(&path)?;
            let kind = entry.header().entry_type();
            ensure!(
                kind.is_file() || kind.is_dir(),
                "Archive links and special entries are not supported"
            );
            total = total
                .checked_add(entry.size())
                .context("Archive size overflow")?;
            ensure!(total <= limit, "Expanded archive exceeds size limit");
            let target = destination.join(path);
            if kind.is_dir() {
                fs::create_dir_all(target)?;
            } else {
                fs::create_dir_all(target.parent().unwrap())?;
                let mut output = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&target)?;
                std::io::copy(&mut entry, &mut output)?;
                fs::set_permissions(
                    target,
                    fs::Permissions::from_mode(entry.header().mode()? & 0o755),
                )?;
            }
        }
    }
    Ok(())
}
fn installed_target(executable: &Path, home: &Path) -> Result<PathBuf> {
    let executable = fs::canonicalize(executable)?;
    #[cfg(target_os = "macos")]
    {
        let target = executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .context("Install Pinacora.app before updating")?;
        ensure!(
            target.file_name().is_some_and(|n| n == "Pinacora.app")
                && executable == target.join("Contents/MacOS/Pinacora"),
            "In-app updates require an installed Pinacora.app; source builds are not replaced"
        );
        ensure!(
            target.parent() == Some(Path::new("/Applications"))
                || target.parent() == Some(home.join("Applications").as_path()),
            "Move Pinacora.app to Applications before updating"
        );
        validate_bundle_identity(target)?;
        Ok(target.into())
    }
    #[cfg(target_os = "linux")]
    {
        let target = home.join(".local/bin/pinacora");
        ensure!(
            executable == target && !fs::symlink_metadata(&target)?.file_type().is_symlink(),
            "In-app updates require ~/.local/bin/pinacora; source and system installs are not replaced"
        );
        Ok(target)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (executable, home);
        bail!("Unsupported installation")
    }
}
#[cfg(any(target_os = "macos", test))]
fn validate_bundle_identity(path: &Path) -> Result<()> {
    let info = plist::Value::from_file(path.join("Contents/Info.plist"))?;
    let info = info
        .as_dictionary()
        .context("Invalid installed app metadata")?;
    ensure!(
        info.get("CFBundleIdentifier")
            .and_then(plist::Value::as_string)
            == Some("com.gitfudge.pinacora")
            && info
                .get("CFBundleExecutable")
                .and_then(plist::Value::as_string)
                == Some("Pinacora"),
        "Installed bundle is not the Pinacora app"
    );
    Ok(())
}
fn wait_helper_ready(
    mut ready: impl FnMut() -> bool,
    mut running: impl FnMut() -> Result<bool>,
    timeout: Duration,
) -> Result<()> {
    let deadline = Instant::now() + timeout;
    loop {
        ensure!(running()?, "Update helper exited before becoming ready");
        if ready() {
            return Ok(());
        }
        ensure!(
            Instant::now() < deadline,
            "Update helper readiness timed out"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}
fn validate_payload(path: &Path, version: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let info = plist::Value::from_file(path.join("Contents/Info.plist"))?;
        let info = info.as_dictionary().context("Invalid app metadata")?;
        ensure!(
            info.get("CFBundleIdentifier")
                .and_then(plist::Value::as_string)
                == Some("com.gitfudge.pinacora")
                && info
                    .get("CFBundleExecutable")
                    .and_then(plist::Value::as_string)
                    == Some("Pinacora")
                && info
                    .get("CFBundleShortVersionString")
                    .and_then(plist::Value::as_string)
                    == Some(version),
            "Release app metadata does not match the requested update"
        );
        let binary = path.join("Contents/MacOS/Pinacora");
        let architecture = if std::env::consts::ARCH == "aarch64" {
            "arm64"
        } else {
            "x86_64"
        };
        ensure!(
            Command::new("/usr/bin/lipo")
                .arg(&binary)
                .args(["-verify_arch", architecture])
                .status()?
                .success(),
            "Release app architecture does not match this Mac"
        );
        ensure!(
            Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(path)
                .status()?
                .success(),
            "Release app signature is invalid"
        );
        let certificate_dir = tempfile::tempdir()?;
        let prefix = certificate_dir.path().join("signer-");
        ensure!(
            Command::new("/usr/bin/codesign")
                .arg("--display")
                .arg(format!("--extract-certificates={}", prefix.display()))
                .arg(path)
                .status()?
                .success(),
            "Release app is unsigned"
        );
        ensure!(
            certificate_dir.path().join("signer-0").is_file(),
            "Release signing certificate is missing"
        );
        // OpenSSL computes the SHA-1 certificate fingerprint used by the release workflow.
        let output = Command::new("/usr/bin/openssl")
            .args([
                "x509",
                "-inform",
                "DER",
                "-noout",
                "-fingerprint",
                "-sha1",
                "-in",
            ])
            .arg(certificate_dir.path().join("signer-0"))
            .output()?;
        ensure!(
            output.status.success(),
            "Could not verify release signing certificate"
        );
        let fingerprint = String::from_utf8(output.stdout)?
            .split('=')
            .nth(1)
            .unwrap_or("")
            .trim()
            .replace(':', "")
            .to_ascii_lowercase();
        ensure!(
            fingerprint == CERTIFICATE,
            "Release was not signed by Pinacora's trusted release certificate"
        );
    }
    #[cfg(target_os = "linux")]
    {
        let bytes = fs::read(path)?;
        ensure!(
            bytes.len() >= 20
                && &bytes[..4] == b"\x7fELF"
                && bytes[4] == 2
                && bytes[5] == 1
                && u16::from_le_bytes([bytes[18], bytes[19]]) == 62,
            "Release binary is not Linux x86_64"
        );
        ensure!(
            binary_version(path)? == Version::parse(version)?,
            "Release binary version does not match the update"
        );
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn binary_version(path: &Path) -> Result<Version> {
    let output = tempfile::tempfile()?;
    let mut child = Command::new(path)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(output.try_clone()?)
        .stderr(Stdio::null())
        .spawn()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Release binary version check timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    use std::io::{Seek, SeekFrom};
    let mut output = output;
    output.seek(SeekFrom::Start(0))?;
    let mut text = String::new();
    output.take(4097).read_to_string(&mut text)?;
    ensure!(
        status.success() && text.len() <= 4096,
        "Could not read installed binary version"
    );
    Version::parse(
        text.trim()
            .strip_prefix("Pinacora ")
            .context("Invalid binary version response")?,
    )
    .context("Invalid binary version")
}
fn installed_version(target: &Path) -> Result<Version> {
    #[cfg(target_os = "macos")]
    {
        validate_bundle_identity(target)?;
        let value = plist::Value::from_file(target.join("Contents/Info.plist"))?;
        let version = value
            .as_dictionary()
            .and_then(|d| d.get("CFBundleShortVersionString"))
            .and_then(plist::Value::as_string)
            .context("Installed app version is missing")?;
        Version::parse(version).context("Installed app version is invalid")
    }
    #[cfg(target_os = "linux")]
    {
        binary_version(target)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = target;
        bail!("Unsupported installation")
    }
}
fn require_newer(offered: &str, installed: &Version) -> Result<()> {
    ensure!(
        Version::parse(offered)? > *installed,
        "The installed app is already at this version or newer; no update was installed"
    );
    Ok(())
}
fn lock_installation(parent: &Path) -> Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let path = parent.join(".pinacora-update.lock");
    ensure!(
        !fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()),
        "Update lock must not be a symlink"
    );
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    lock.try_lock()
        .context("Another Pinacora update is in progress; keep this app open and retry later")?;
    // Keep this stable lock file after release; unlinking permits separate inode locks.
    Ok(lock)
}
fn reopen(target: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        ensure!(
            Command::new("/usr/bin/open")
                .arg("-n")
                .arg(target)
                .status()?
                .success(),
            "Could not reopen Pinacora"
        );
    } else {
        Command::new(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
    }
    Ok(())
}
#[derive(Serialize, Deserialize)]
struct Manifest {
    target: PathBuf,
    payload: PathBuf,
    version: String,
    old_pid: u32,
    worker_registered: bool,
}
pub struct PendingUpdate {
    directory: tempfile::TempDir,
    manifest: Manifest,
}
pub fn stage_update(release: &Release) -> Result<PendingUpdate> {
    // Revalidate even a deserialized Release before any network or filesystem effects.
    let tag = format!("v{}", release.version);
    let mut assets = vec![release.asset.clone()];
    if let Some(checksum) = &release.checksum {
        assets.push(checksum.clone());
    }
    select(
        ApiRelease {
            tag_name: tag,
            html_url: release.html_url.clone(),
            draft: false,
            prerelease: false,
            assets,
        },
        env!("CARGO_PKG_VERSION"),
    )?
    .context("This release is not newer than the running app")?;

    ensure!(
        Version::parse(&release.version)? > Version::parse(env!("CARGO_PKG_VERSION"))?,
        "Updates cannot downgrade this app"
    );
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is unavailable")?);
    let target = installed_target(&std::env::current_exe()?, &home)?;
    let directory = tempfile::Builder::new()
        .prefix(".pinacora-update-")
        .tempdir_in(target.parent().unwrap())
        .context("The installation folder is not writable; update manually")?;
    let expected = if let Some(checksum) = &release.checksum {
        let text = String::from_utf8(get(&checksum.browser_download_url, 4096, true)?)?;
        let fields: Vec<_> = text.split_whitespace().collect();
        ensure!(
            fields.len() == 2 && fields[1].trim_start_matches('*') == release.asset.name,
            "Checksum does not identify the release archive"
        );
        fields[0].to_owned()
    } else {
        release
            .asset
            .digest
            .as_deref()
            .and_then(|d| d.strip_prefix("sha256:"))
            .context("Missing SHA-256 digest")?
            .into()
    };
    let archive = get(&release.asset.browser_download_url, MAX_ARCHIVE, true)?;
    ensure!(
        archive.len() as u64 == release.asset.size,
        "Incomplete release archive"
    );
    verify_archive(&archive, &expected)?;
    if let Some(value) = release.asset.digest.as_deref()
        && valid_digest(value)
    {
        verify_archive(&archive, &value[7..])?;
    }
    let unpacked = directory.path().join("unpacked");
    fs::create_dir(&unpacked)?;
    extract(&archive, &unpacked, release.asset.name.ends_with(".zip"))?;
    let payload = if cfg!(target_os = "macos") {
        unpacked.join("Pinacora.app")
    } else {
        unpacked.join(format!(
            "Pinacora-{}-linux-x86_64/pinacora",
            release.version
        ))
    };
    validate_payload(&payload, &release.version)?;
    let worker_registered = crate::rotation_service_manager::registered_for_update()?;
    Ok(PendingUpdate {
        directory,
        manifest: Manifest {
            target,
            payload,
            version: release.version.clone(),
            old_pid: std::process::id(),
            worker_registered,
        },
    })
}
impl PendingUpdate {
    pub fn launch(self) -> Result<()> {
        let helper = self.directory.path().join("helper");
        fs::copy(std::env::current_exe()?, &helper)?;
        fs::set_permissions(&helper, fs::Permissions::from_mode(0o700))?;
        let manifest_path = self.directory.path().join("manifest.json");
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&manifest_path)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&serde_json::to_vec(&self.manifest)?)?;
        file.sync_all()?;
        let log = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(self.directory.path().join("update.log"))?;
        let mut command = Command::new(&helper);
        command
            .arg("--update-helper")
            .arg(&manifest_path)
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log)
            .process_group(0);
        let mut child = command
            .spawn()
            .context("Could not start update helper; the current app is unchanged")?;
        let ready = self.directory.path().join("ready");
        let result = wait_helper_ready(
            || ready.is_file(),
            || Ok(child.try_wait()?.is_none()),
            Duration::from_secs(30),
        );
        if let Err(error) = result {
            let _ = child.kill();
            child
                .wait()
                .context("Could not stop the failed update helper")?;
            let log =
                fs::read_to_string(self.directory.path().join("update.log")).unwrap_or_default();
            bail!("{error:#}. The current app is unchanged. {}", log.trim());
        }
        let _ = self.directory.keep();
        Ok(())
    }
}
fn swap(
    target: &Path,
    payload: &Path,
    backup: &Path,
    launch: impl FnOnce() -> Result<()>,
) -> Result<()> {
    fs::rename(target, backup).context("Could not preserve the current installation")?;
    let result = (|| {
        fs::rename(payload, target)?;
        launch()
    })();
    if let Err(error) = result {
        if target.exists() {
            fs::rename(target, payload)?;
        }
        fs::rename(backup, target)
            .context("Update failed and automatic rollback failed; restore the backup manually")?;
        return Err(error).context("Update failed; the previous installation was restored");
    }
    Ok(())
}
fn helper(manifest_path: &Path) -> Result<()> {
    let root = fs::canonicalize(manifest_path.parent().context("Missing helper directory")?)?;
    ensure!(
        root.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".pinacora-update-"))
            && fs::metadata(&root)?.permissions().mode() & 0o077 == 0,
        "Invalid private update directory"
    );
    ensure!(
        fs::canonicalize(std::env::current_exe()?)? == root.join("helper")
            && fs::canonicalize(manifest_path)? == root.join("manifest.json"),
        "Invalid update helper invocation"
    );
    let manifest: Manifest = serde_json::from_slice(&fs::read(manifest_path)?)?;
    let home = PathBuf::from(std::env::var_os("HOME").context("HOME is unavailable")?);
    let old_executable = if cfg!(target_os = "macos") {
        manifest.target.join("Contents/MacOS/Pinacora")
    } else {
        manifest.target.clone()
    };
    ensure!(
        installed_target(&old_executable, &home)? == manifest.target
            && root.parent() == manifest.target.parent(),
        "Invalid installation target"
    );
    let _installation_lock = lock_installation(
        manifest
            .target
            .parent()
            .context("Missing installation directory")?,
    )?;
    // The helper may have been staged by an older running GUI instance.
    require_newer(&manifest.version, &installed_version(&manifest.target)?)?;
    let expected_payload = if cfg!(target_os = "macos") {
        root.join("unpacked/Pinacora.app")
    } else {
        root.join(format!(
            "unpacked/Pinacora-{}-linux-x86_64/pinacora",
            manifest.version
        ))
    };
    ensure!(
        manifest.payload == expected_payload
            && fs::canonicalize(&manifest.payload)? == manifest.payload,
        "Invalid staged update path"
    );
    ensure!(
        Version::parse(&manifest.version)? > Version::parse(env!("CARGO_PKG_VERSION"))?,
        "Update cannot downgrade the app"
    );
    validate_payload(&manifest.payload, &manifest.version)?;
    ensure!(
        manifest.old_pid > 1 && manifest.old_pid != std::process::id(),
        "Invalid old app process"
    );
    // Signal only after all preflight checks: the GUI remains alive on failure.
    let mut ready = tempfile::NamedTempFile::new_in(&root)?;
    ready.write_all(b"ready")?;
    ready.as_file().sync_all()?;
    ready
        .persist(root.join("ready"))
        .map_err(|error| error.error)?;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Command::new("/bin/kill")
        .args(["-0", &manifest.old_pid.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?
        .success()
    {
        ensure!(
            Instant::now() < deadline,
            "App did not exit; update cancelled and current installation retained"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    if let Err(error) = installed_version(&manifest.target)
        .and_then(|version| require_newer(&manifest.version, &version))
    {
        let _ = reopen(&manifest.target);
        return Err(error);
    }
    let launch = || -> Result<()> {
        crate::rotation_service_manager::restart_after_update(manifest.worker_registered)?;
        reopen(&manifest.target)
    };
    let backup = root.join("previous");
    if let Err(error) = swap(&manifest.target, &manifest.payload, &backup, launch) {
        let _ = crate::rotation_service_manager::restart_after_update(manifest.worker_registered);
        let _ = reopen(&manifest.target);
        return Err(error);
    }
    let _ = fs::remove_dir_all(&root);
    Ok(())
}
pub fn run_update_helper_if_requested() -> Option<Result<()>> {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() != Some(std::ffi::OsStr::new("--update-helper")) {
        return None;
    }
    Some((|| {
        let path = args.next().context("Missing update manifest")?;
        ensure!(args.next().is_none(), "Unexpected helper arguments");
        helper(Path::new(&path))
    })())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn api(version: &str) -> ApiRelease {
        let name = asset_name(version).unwrap();
        ApiRelease {
            tag_name: format!("v{version}"),
            html_url: format!("https://github.com/{REPOSITORY}/releases/tag/v{version}"),
            draft: false,
            prerelease: false,
            assets: vec![Asset {
                browser_download_url: format!(
                    "https://github.com/{REPOSITORY}/releases/download/v{version}/{name}"
                ),
                name,
                digest: Some(format!("sha256:{}", "a".repeat(64))),
                size: 100,
            }],
        }
    }
    #[test]
    fn concurrent_helpers_cannot_lock_the_same_installation() {
        let dir = tempfile::tempdir().unwrap();
        let first = lock_installation(dir.path()).unwrap();
        assert!(lock_installation(dir.path()).is_err());
        drop(first);
        assert!(dir.path().join(".pinacora-update.lock").is_file());
        lock_installation(dir.path()).unwrap();
    }
    #[test]
    fn staged_older_offers_never_replace_advanced_installations() {
        let dir = tempfile::tempdir().unwrap();
        let installed = dir.path().join("installed");
        fs::write(&installed, "0.6.0").unwrap();
        let version = Version::parse(&fs::read_to_string(&installed).unwrap()).unwrap();
        assert!(require_newer("0.5.0", &version).is_err());
        assert!(require_newer("0.6.0", &version).is_err());
        require_newer("0.7.0", &version).unwrap();
        assert_eq!(fs::read_to_string(installed).unwrap(), "0.6.0");
    }
    #[test]
    fn versions_never_downgrade_and_preview_rejected() {
        assert!(select(api("0.2.0"), "0.4.0").unwrap().is_none());
        assert!(select(api("0.4.0"), "0.4.0").unwrap().is_none());
        assert_eq!(
            select(api("0.5.0"), "0.4.0").unwrap().unwrap().version,
            "0.5.0"
        );
        let mut preview = api("0.5.0");
        preview.prerelease = true;
        assert!(select(preview, "0.4.0").is_err());
        assert!(select(api("0.5.0-beta.1"), "0.4.0").is_err());
    }
    #[test]
    fn malicious_and_incomplete_assets_rejected() {
        let mut release = api("0.5.0");
        release.assets[0].browser_download_url = "https://evil.example/app".into();
        assert!(select(release, "0.4.0").is_err());
        let mut release = api("0.5.0");
        release.assets.clear();
        assert!(select(release, "0.4.0").is_err());
        let mut release = api("0.5.0");
        release.assets[0].digest = None;
        assert!(select(release, "0.4.0").is_err());
    }
    #[test]
    fn checksum_failure_and_unsafe_paths_are_rejected() {
        assert!(verify_archive(b"wrong", &digest(b"expected")).is_err());
        assert!(verify_archive(b"expected", &digest(b"expected")).is_ok());
        for path in ["../escape", "/absolute", "safe/../../escape"] {
            assert!(safe_path(Path::new(path)).is_err());
        }
    }
    #[test]
    fn failed_relaunch_rolls_back_without_losing_staging() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("installed");
        let payload = dir.path().join("new");
        let backup = dir.path().join("previous");
        fs::write(&target, "old").unwrap();
        fs::write(&payload, "new").unwrap();
        assert!(swap(&target, &payload, &backup, || bail!("launch failed")).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(fs::read(&payload).unwrap(), b"new");
        assert!(!backup.exists());
    }
    #[test]
    fn development_executables_are_never_updated() {
        assert!(
            installed_target(
                &std::env::current_exe().unwrap(),
                &PathBuf::from(std::env::var_os("HOME").unwrap())
            )
            .is_err()
        );
    }
    #[test]
    fn helper_preflight_failure_and_timeout_keep_gui_alive() {
        assert!(
            wait_helper_ready(|| false, || Ok(false), Duration::ZERO)
                .unwrap_err()
                .to_string()
                .contains("exited")
        );
        assert!(
            wait_helper_ready(|| false, || Ok(true), Duration::ZERO)
                .unwrap_err()
                .to_string()
                .contains("timed out")
        );
        wait_helper_ready(|| true, || Ok(true), Duration::ZERO).unwrap();
    }
    #[test]
    fn installed_bundle_identity_rejects_renamed_other_apps() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir(dir.path().join("Contents")).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), "evil.other".into());
        info.insert("CFBundleExecutable".into(), "Pinacora".into());
        plist::Value::Dictionary(info.clone())
            .to_file_xml(dir.path().join("Contents/Info.plist"))
            .unwrap();
        assert!(validate_bundle_identity(dir.path()).is_err());
        info.insert("CFBundleIdentifier".into(), "com.gitfudge.pinacora".into());
        plist::Value::Dictionary(info)
            .to_file_xml(dir.path().join("Contents/Info.plist"))
            .unwrap();
        validate_bundle_identity(dir.path()).unwrap();
    }
    #[test]
    fn zip_declared_size_must_match_actual_and_budget() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            zip.start_file(
                "payload",
                zip::write::SimpleFileOptions::default()
                    .compression_method(zip::CompressionMethod::Stored),
            )
            .unwrap();
            zip.write_all(b"1234567890").unwrap();
            zip.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(extract_with_limit(bytes.get_ref(), dir.path(), true, 5).is_err());
        assert!(!dir.path().join("payload").exists());
        let mut tampered = bytes.into_inner();
        let central = tampered
            .windows(4)
            .position(|w| w == b"PK\x01\x02")
            .unwrap();
        tampered[central + 24..central + 28].copy_from_slice(&2u32.to_le_bytes());
        let dir = tempfile::tempdir().unwrap();
        assert!(extract_with_limit(&tampered, dir.path(), true, 5).is_err());
        assert!(fs::metadata(dir.path().join("payload")).unwrap().len() <= 2);
    }
    #[test]
    fn actual_zip_traversal_is_rejected_before_writes() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            zip.start_file("../escape", zip::write::SimpleFileOptions::default())
                .unwrap();
            zip.write_all(b"bad").unwrap();
            zip.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(extract(bytes.get_ref(), dir.path(), true).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn tar_symlinks_are_rejected_without_writing_targets() {
        let mut archive = tar::Builder::new(Vec::new());
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        header.set_mode(0o777);
        header.set_cksum();
        archive
            .append_link(&mut header, "bundle/link", "../../outside")
            .unwrap();
        let tar = archive.into_inner().unwrap();
        let mut gzip = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gzip.write_all(&tar).unwrap();
        let dir = tempfile::tempdir().unwrap();
        assert!(extract(&gzip.finish().unwrap(), dir.path(), false).is_err());
        assert!(!dir.path().join("bundle").exists());
    }
    #[test]
    fn successful_swap_keeps_previous_until_launch_accepted() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("installed");
        let payload = dir.path().join("new");
        let backup = dir.path().join("previous");
        fs::write(&target, "old").unwrap();
        fs::write(&payload, "new").unwrap();
        swap(&target, &payload, &backup, || {
            assert_eq!(fs::read(&target)?, b"new");
            assert_eq!(fs::read(&backup)?, b"old");
            Ok(())
        })
        .unwrap();
    }
    #[test]
    fn zip_links_and_oversized_entries_are_rejected() {
        let mut bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut bytes);
            zip.add_symlink(
                "Pinacora.app/link",
                "../../outside",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
            zip.finish().unwrap();
        }
        let dir = tempfile::tempdir().unwrap();
        assert!(extract(bytes.get_ref(), dir.path(), true).is_err());
    }
}
