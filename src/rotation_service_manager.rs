//! Per-user registration for the wallpaper worker. Registration never enables rotation;
//! the worker reads the saved configuration. Linux login uses an XDG autostart bootstrap
//! to capture the new graphical session before systemd starts the worker.
use anyhow::{Context, Result};
use std::{
    ffi::OsStr,
    fs,
    io::Write,
    path::Path,
    process::{Command, Output},
};

#[cfg(any(target_os = "linux", test))]
const SERVICE_NAME: &str = "pinacora-rotation.service";
#[cfg(any(target_os = "macos", test))]
const LAUNCH_LABEL: &str = "com.pinacora.rotation";
#[cfg(any(target_os = "linux", test))]
const SESSION_VARIABLES: &[&str] = &[
    "HYPRLAND_INSTANCE_SIGNATURE",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XDG_CURRENT_DESKTOP",
    "DBUS_SESSION_BUS_ADDRESS",
    "XDG_RUNTIME_DIR",
    "XDG_DATA_HOME",
    "XDG_CACHE_HOME",
    "XDG_CONFIG_HOME",
    "PATH",
];

/// Install/update the user's registration and start the worker if needed.
/// Call for explicit rotation commands and `--rotation-service-bootstrap`, never
/// merely because the GUI opened. No root privileges or shell commands are used.
pub fn ensure_running() -> Result<()> {
    let executable = std::env::current_exe().context("Could not locate Pinacora executable")?;
    #[cfg(target_os = "linux")]
    ensure_linux(&executable)?;
    #[cfg(target_os = "macos")]
    ensure_macos(&executable)?;
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = executable;
        anyhow::bail!("Background rotation supports Linux and macOS")
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    wait_until_ready(
        || crate::rotation_service::request(crate::rotation_service::Command::Status).map(|_| ()),
        std::time::Duration::from_secs(10),
        std::time::Duration::from_millis(100),
    )
}

fn update_worker_loaded(present: bool, probe: impl FnOnce() -> Result<bool>) -> Result<bool> {
    if !present {
        return Ok(false);
    }
    probe()
}
#[cfg(target_os = "macos")]
fn update_launch_job() -> Result<String> {
    let uid = run("id", &[OsStr::new("-u")])?;
    anyhow::ensure!(uid.status.success(), "Could not determine launchd user");
    let uid = std::str::from_utf8(&uid.stdout)?.trim();
    anyhow::ensure!(
        !uid.is_empty() && uid.bytes().all(|b| b.is_ascii_digit()),
        "Invalid launchd user"
    );
    Ok(format!("gui/{uid}/{LAUNCH_LABEL}"))
}
/// Whether the current installed executable has a loaded worker registration.
/// Checking for updates never creates a registration or changes preferences.
pub fn registered_for_update() -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        let path = home()?.join("Library/LaunchAgents/com.pinacora.rotation.plist");
        if !update_worker_loaded(path.exists(), || {
            Ok(run(
                "launchctl",
                &[OsStr::new("print"), OsStr::new(&update_launch_job()?)],
            )?
            .status
            .success())
        })? {
            return Ok(false);
        }
        let value = plist::Value::from_file(path)?;
        let executable = std::env::current_exe()?;
        let registered = value
            .as_dictionary()
            .and_then(|d| d.get("ProgramArguments"))
            .and_then(plist::Value::as_array)
            .and_then(|a| a.first())
            .and_then(plist::Value::as_string);
        anyhow::ensure!(
            registered == executable.to_str(),
            "Rotation is registered for a different installation; update that app manually"
        );
        Ok(true)
    }
    #[cfg(target_os = "linux")]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(std::path::PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or(home()?.join(".config"));
        let path = config.join("systemd/user").join(SERVICE_NAME);
        if !update_worker_loaded(path.exists(), || {
            Ok(run(
                "systemctl",
                &[
                    OsStr::new("--user"),
                    OsStr::new("is-active"),
                    OsStr::new("--quiet"),
                    OsStr::new(SERVICE_NAME),
                ],
            )?
            .status
            .success())
        })? {
            return Ok(false);
        }
        let unit = fs::read_to_string(path)?;
        let expected = format!(
            "ExecStart={} --rotation-service",
            systemd_quote(path_text(&std::env::current_exe()?)?, true)?
        );
        anyhow::ensure!(
            unit.lines().any(|line| line == expected),
            "Rotation is registered for a different installation; update that app manually"
        );
        Ok(true)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    Ok(false)
}

/// Refresh the already registered worker after executable replacement. This does
/// not enable rotation or create an absent service and leaves saved state intact.
pub fn restart_after_update(registered: bool) -> Result<()> {
    if !registered {
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    checked(
        "systemctl",
        &[
            OsStr::new("--user"),
            OsStr::new("try-restart"),
            OsStr::new(SERVICE_NAME),
        ],
    )?;
    #[cfg(target_os = "macos")]
    {
        let job = update_launch_job()?;
        if run("launchctl", &[OsStr::new("print"), OsStr::new(&job)])?
            .status
            .success()
        {
            checked(
                "launchctl",
                &[OsStr::new("kickstart"), OsStr::new("-k"), OsStr::new(&job)],
            )?;
        }
    }
    Ok(())
}

fn wait_until_ready(
    mut probe: impl FnMut() -> Result<()>,
    timeout: std::time::Duration,
    poll_interval: std::time::Duration,
) -> Result<()> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match probe() {
            Ok(()) => return Ok(()),
            Err(error) => {
                // An absent socket or stale socket whose process is restarting is
                // expected during startup. Other protocol/worker errors need an
                // immediate, actionable report rather than repeated requests.
                let starting = error.chain().any(|cause| {
                    cause.downcast_ref::<std::io::Error>().is_some_and(|error| {
                        matches!(
                            error.kind(),
                            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
                        )
                    })
                });
                if !starting || std::time::Instant::now() >= deadline {
                    return Err(error).context(
                        "Background rotation service did not become ready. Retry; if it still fails, check `journalctl --user -u pinacora-rotation.service` on Linux or the com.pinacora.rotation LaunchAgent in macOS Console",
                    );
                }
                std::thread::sleep(
                    poll_interval
                        .min(deadline.saturating_duration_since(std::time::Instant::now())),
                );
            }
        }
    }
}

fn run(program: &str, args: &[&OsStr]) -> Result<Output> {
    Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Could not run {program} for background rotation"))
}

fn checked(program: &str, args: &[&OsStr]) -> Result<()> {
    let output = run(program, args)?;
    anyhow::ensure!(
        output.status.success(),
        "{program} failed to manage background rotation: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(())
}

fn home() -> Result<std::path::PathBuf> {
    let path = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .context("HOME is unavailable for per-user service registration")?;
    anyhow::ensure!(path.is_absolute(), "HOME must be an absolute path");
    Ok(path)
}

/// Atomically replace only changed registrations. Private session environment data
/// is never world-readable, including while it is being written.
fn write_changed(path: &Path, contents: &[u8]) -> Result<bool> {
    match fs::read(path) {
        Ok(previous) if previous == contents => return Ok(false),
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", path.display()));
        }
    }
    let directory = path
        .parent()
        .context("Registration has no parent directory")?;
    fs::create_dir_all(directory)
        .with_context(|| format!("Could not create {}", directory.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.write_all(contents)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(path)
        .map_err(|error| error.error)
        .with_context(|| format!("Could not save {}", path.display()))?;
    Ok(true)
}

#[cfg(target_os = "linux")]
fn ensure_linux(executable: &Path) -> Result<()> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or(home()?.join(".config"));
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .context("A graphical login with XDG_RUNTIME_DIR is required for background rotation")?;
    anyhow::ensure!(
        runtime.is_absolute(),
        "XDG_RUNTIME_DIR must be an absolute path"
    );
    let session: Vec<(String, String)> = SESSION_VARIABLES
        .iter()
        .filter_map(|name| {
            std::env::var_os(name).map(|value| {
                value
                    .into_string()
                    .map(|value| (name.to_string(), value))
                    .map_err(|_| anyhow::anyhow!("{name} is not UTF-8"))
            })
        })
        .collect::<Result<_>>()?;
    anyhow::ensure!(
        session.iter().any(|(name, value)| {
            (name == "DISPLAY" || name == "WAYLAND_DISPLAY") && !value.is_empty()
        }),
        "A graphical display session is required for background rotation"
    );
    let session_path = runtime.join("pinacora/rotation-session.env");
    let session_changed = write_changed(&session_path, linux_environment(&session)?.as_bytes())?;
    let unit = linux_unit(executable, &session_path, &session)?;
    let unit_path = config.join("systemd/user").join(SERVICE_NAME);
    let unit_changed = write_changed(&unit_path, unit.as_bytes())?;
    write_changed(
        &config.join("autostart/pinacora-rotation.desktop"),
        linux_autostart(executable)?.as_bytes(),
    )?;
    // A never-loaded registration has no failed state to reset. Only exit 0
    // from is-failed means the unit needs start-limit/failure recovery.
    let unit_failed = run(
        "systemctl",
        &[
            OsStr::new("--user"),
            OsStr::new("is-failed"),
            OsStr::new(SERVICE_NAME),
        ],
    )?
    .status
    .success();
    for arguments in linux_commands(&unit_path, unit_changed, session_changed, unit_failed) {
        let arguments: Vec<&OsStr> = arguments.iter().map(|arg| arg.as_os_str()).collect();
        checked("systemctl", &arguments)?;
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn linux_commands(
    unit: &Path,
    changed: bool,
    session_changed: bool,
    unit_failed: bool,
) -> Vec<Vec<std::ffi::OsString>> {
    use std::ffi::OsString;
    // The manager may have a different XDG_CONFIG_HOME from the GUI. `link`
    // explicitly makes this absolute registration available in its lookup path.
    let mut commands = vec![vec![
        OsString::from("--user"),
        OsString::from("link"),
        unit.as_os_str().to_owned(),
    ]];
    if changed {
        commands.push(vec!["--user".into(), "daemon-reload".into()]);
    }
    if unit_failed {
        commands.push(vec![
            "--user".into(),
            "reset-failed".into(),
            SERVICE_NAME.into(),
        ]);
    }
    commands.push(vec![
        "--user".into(),
        if changed || session_changed {
            "restart"
        } else {
            "start"
        }
        .into(),
        SERVICE_NAME.into(),
    ]);
    commands
}

// systemd uses its own command parser. Percent specifiers and dollar expansion
// must be escaped in addition to quotes and backslashes.
#[cfg(any(target_os = "linux", test))]
fn systemd_quote(value: &str, command: bool) -> Result<String> {
    anyhow::ensure!(
        !value.contains(['\0', '\n', '\r']),
        "Service values cannot contain line breaks or NUL"
    );
    let value = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%");
    Ok(format!(
        "\"{}\"",
        if command {
            value.replace('$', "$$")
        } else {
            value
        }
    ))
}

fn path_text(path: &Path) -> Result<&str> {
    anyhow::ensure!(
        path.is_absolute(),
        "Service executable and session paths must be absolute"
    );
    path.to_str().context("Service path is not UTF-8")
}

#[cfg(any(target_os = "linux", test))]
fn linux_unit(
    executable: &Path,
    environment: &Path,
    session: &[(String, String)],
) -> Result<String> {
    let environment = path_text(environment)?;
    anyhow::ensure!(
        !environment.contains(['\0', '\n', '\r']),
        "Service environment path cannot contain line breaks or NUL"
    );
    let absent: Vec<_> = SESSION_VARIABLES
        .iter()
        .copied()
        .filter(|name| !session.iter().any(|(present, _)| present == name))
        .collect();
    Ok(format!(
        "[Unit]\nDescription=Pinacora wallpaper rotation\nPartOf=graphical-session.target\n\n[Service]\nType=exec\nExecStart={} --rotation-service\nEnvironmentFile={}\nUnsetEnvironment={}\nRestart=on-failure\nRestartSec=5\nTimeoutStopSec=15\n",
        systemd_quote(path_text(executable)?, true)?,
        environment.replace('%', "%%"),
        absent.join(" ")
    ))
}

#[cfg(any(target_os = "linux", test))]
fn linux_environment(session: &[(String, String)]) -> Result<String> {
    let mut result = String::new();
    for (name, value) in session {
        anyhow::ensure!(
            SESSION_VARIABLES.contains(&name.as_str()),
            "Unexpected session variable"
        );
        // EnvironmentFile supports double-quoted values, with no shell or
        // specifier expansion. Backslashes, quotes, dollars and backticks need
        // escaping according to its double-quote grammar.
        anyhow::ensure!(
            !value.contains(['\0', '\n', '\r']),
            "{name} contains unsupported line breaks or NUL"
        );
        let value = value
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('$', "\\$")
            .replace('`', "\\`");
        result.push_str(&format!("{name}=\"{value}\"\n"));
    }
    Ok(result)
}

#[cfg(any(target_os = "linux", test))]
fn linux_autostart(executable: &Path) -> Result<String> {
    let path = path_text(executable)?;
    anyhow::ensure!(
        !path.contains(['\0', '\n', '\r']),
        "Autostart path contains line breaks or NUL"
    );
    // Desktop entries apply string escaping before Exec argument unquoting,
    // hence four backslashes for a literal backslash in the executable path.
    let path = path
        .replace('\\', "\\\\\\\\")
        .replace('"', "\\\\\"")
        .replace('$', "\\\\$")
        .replace('`', "\\\\`")
        .replace('%', "%%");
    Ok(format!(
        "[Desktop Entry]\nType=Application\nName=Pinacora rotation service\nExec=\"{path}\" --rotation-service-bootstrap\nTerminal=false\nNoDisplay=true\n"
    ))
}

#[cfg(target_os = "macos")]
fn ensure_macos(executable: &Path) -> Result<()> {
    let path = home()?
        .join("Library/LaunchAgents")
        .join(format!("{LAUNCH_LABEL}.plist"));
    let changed = write_changed(&path, &macos_plist(executable)?)?;
    let uid = run("id", &[OsStr::new("-u")])?;
    anyhow::ensure!(
        uid.status.success(),
        "Could not determine user ID for launchd"
    );
    let uid = std::str::from_utf8(&uid.stdout)?.trim();
    anyhow::ensure!(
        !uid.is_empty() && uid.bytes().all(|byte| byte.is_ascii_digit()),
        "Invalid user ID for launchd"
    );
    let domain = format!("gui/{uid}");
    let job = format!("{domain}/{LAUNCH_LABEL}");
    let loaded = run("launchctl", &[OsStr::new("print"), OsStr::new(&job)])?
        .status
        .success();
    for arguments in macos_commands(&path, &domain, &job, changed, loaded) {
        let arguments: Vec<&OsStr> = arguments.iter().map(|arg| arg.as_os_str()).collect();
        checked("launchctl", &arguments)?;
    }
    // RunAtLoad and KeepAlive start newly bootstrapped jobs and supervise loaded
    // jobs. Leaving an existing job alone preserves its timer and preparation.
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn macos_commands(
    path: &Path,
    domain: &str,
    job: &str,
    changed: bool,
    loaded: bool,
) -> Vec<Vec<std::ffi::OsString>> {
    let mut commands = Vec::new();
    if changed && loaded {
        commands.push(vec!["bootout".into(), job.into()]);
    }
    if changed || !loaded {
        commands.push(vec![
            "bootstrap".into(),
            domain.into(),
            path.as_os_str().to_owned(),
        ]);
    }
    commands
}

#[cfg(any(target_os = "macos", test))]
fn macos_plist(executable: &Path) -> Result<Vec<u8>> {
    let mut job = plist::Dictionary::new();
    job.insert("Label".into(), LAUNCH_LABEL.into());
    job.insert(
        "ProgramArguments".into(),
        plist::Value::Array(vec![
            path_text(executable)?.into(),
            "--rotation-service".into(),
        ]),
    );
    job.insert("RunAtLoad".into(), true.into());
    job.insert("KeepAlive".into(), true.into());
    job.insert("LimitLoadToSessionType".into(), "Aqua".into());
    job.insert("ThrottleInterval".into(), 5u64.into());
    let mut result = Vec::new();
    plist::Value::Dictionary(job).to_writer_xml(&mut result)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn updates_refresh_only_present_and_loaded_workers() {
        assert!(
            !update_worker_loaded(false, || panic!("absent worker must not be probed")).unwrap()
        );
        assert!(!update_worker_loaded(true, || Ok(false)).unwrap());
        assert!(update_worker_loaded(true, || Ok(true)).unwrap());
        restart_after_update(false).unwrap();
    }
    #[test]
    fn linux_unit_preserves_argument_boundaries_and_session_lifecycle() {
        let unit = linux_unit(
            Path::new("/opt/My Art/$app% \"name\"/pinacora"),
            Path::new("/run/user/1000/my runtime/env"),
            &[("WAYLAND_DISPLAY".into(), "wayland-1".into())],
        )
        .unwrap();
        assert!(unit.contains(
            "ExecStart=\"/opt/My Art/$$app%% \\\"name\\\"/pinacora\" --rotation-service\n"
        ));
        assert!(unit.contains("PartOf=graphical-session.target"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(!unit.contains("WantedBy="));
        let absent = unit
            .lines()
            .find(|line| line.starts_with("UnsetEnvironment="))
            .unwrap();
        assert!(absent.contains("HYPRLAND_INSTANCE_SIGNATURE"));
        assert!(!absent.contains("WAYLAND_DISPLAY"));
        assert!(linux_unit(Path::new("/opt/bad\nprogram"), Path::new("/tmp/env"), &[]).is_err());
    }

    #[test]
    fn linux_commands_link_custom_config_and_only_restart_changed_registration() {
        let path = Path::new("/custom/My Config/systemd/user/pinacora-rotation.service");
        let commands = linux_commands(path, false, false, false);
        assert_eq!(
            commands[0],
            vec![
                std::ffi::OsString::from("--user"),
                "link".into(),
                path.as_os_str().to_owned()
            ]
        );
        assert_eq!(commands.last().unwrap()[1], "start");
        assert!(!commands.iter().any(|args| args[1] == "daemon-reload"));
        assert_eq!(
            linux_commands(path, false, true, false).last().unwrap()[1],
            "restart"
        );
        assert!(
            linux_commands(path, true, false, false)
                .iter()
                .any(|args| args[1] == "daemon-reload")
        );
        let session = vec![
            ("XDG_DATA_HOME".into(), "/custom/My Data".into()),
            ("XDG_CACHE_HOME".into(), "/custom/My Cache".into()),
        ];
        let unit = linux_unit(Path::new("/opt/pinacora"), Path::new("/tmp/env"), &session).unwrap();
        let absent = unit
            .lines()
            .find(|line| line.starts_with("UnsetEnvironment="))
            .unwrap();
        assert!(!absent.contains("XDG_DATA_HOME"));
        assert!(!absent.contains("XDG_CACHE_HOME"));
        let environment = linux_environment(&session).unwrap();
        assert!(environment.contains("XDG_DATA_HOME=\"/custom/My Data\"\n"));
        assert!(environment.contains("XDG_CACHE_HOME=\"/custom/My Cache\"\n"));
    }

    #[test]
    fn first_registration_skips_reset_but_failed_workers_recover_start_limits() {
        let path = Path::new("/home/person/.config/systemd/user/pinacora-rotation.service");
        let first_registration = linux_commands(path, true, true, false);
        assert!(
            !first_registration
                .iter()
                .any(|args| args[1] == "reset-failed")
        );
        assert_eq!(first_registration.last().unwrap()[1], "restart");
        let failed_worker = linux_commands(path, false, false, true);
        let reset_index = failed_worker
            .iter()
            .position(|args| args[1] == "reset-failed")
            .unwrap();
        let start_index = failed_worker
            .iter()
            .position(|args| args[1] == "start")
            .unwrap();
        assert!(reset_index < start_index);
        let active_worker = linux_commands(path, false, false, false);
        assert!(
            !active_worker
                .iter()
                .any(|args| args[1] == "reset-failed" || args[1] == "restart")
        );
        assert_eq!(active_worker.last().unwrap()[1], "start");
    }

    #[test]
    fn session_values_are_data_and_stale_session_variables_are_removed() {
        let environment = linux_environment(&[(
            "PATH".into(),
            "/bin:$HOME:with \"quotes\":`cmd`:a\\b%".into(),
        )])
        .unwrap();
        assert_eq!(
            environment,
            "PATH=\"/bin:\\$HOME:with \\\"quotes\\\":\\`cmd\\`:a\\\\b%\"\n"
        );
        assert!(linux_environment(&[("PATH".into(), "bad\nnew=value".into())]).is_err());
        assert!(linux_environment(&[("UNKNOWN".into(), "data".into())]).is_err());
    }

    #[test]
    fn autostart_starts_a_session_bootstrap_with_exact_executable() {
        let desktop = linux_autostart(Path::new("/opt/My Art/$app`x`%/pinacora")).unwrap();
        assert!(desktop.contains(
            "Exec=\"/opt/My Art/\\\\$app\\\\`x\\\\`%%/pinacora\" --rotation-service-bootstrap\n"
        ));
        assert!(!desktop.contains("sh -c"));
    }

    #[test]
    fn macos_registration_is_a_login_agent_with_exact_arguments() {
        let executable = "/Applications/My <Art> & \"Art\"/$pinacora";
        let bytes = macos_plist(Path::new(executable)).unwrap();
        let job = plist::Value::from_reader(std::io::Cursor::new(bytes)).unwrap();
        let job = job.as_dictionary().unwrap();
        let args = job["ProgramArguments"].as_array().unwrap();
        assert_eq!(args[0].as_string(), Some(executable));
        assert_eq!(args[1].as_string(), Some("--rotation-service"));
        assert_eq!(args.len(), 2);
        assert_eq!(job["RunAtLoad"].as_boolean(), Some(true));
        assert_eq!(job["KeepAlive"].as_boolean(), Some(true));
        assert_eq!(job["LimitLoadToSessionType"].as_string(), Some("Aqua"));
    }

    #[test]
    fn launchd_commands_preserve_paths_and_loaded_workers() {
        let path = Path::new("/Users/me/Library/My Agents/rotation.plist");
        let commands = macos_commands(path, "gui/501", "gui/501/com.pinacora.rotation", true, true);
        assert_eq!(commands.len(), 2);
        assert_eq!(commands[0][0], "bootout");
        assert_eq!(commands[1][0], "bootstrap");
        assert_eq!(commands[1][2], path.as_os_str());
        assert!(
            macos_commands(
                path,
                "gui/501",
                "gui/501/com.pinacora.rotation",
                false,
                true
            )
            .is_empty()
        );
        assert_eq!(
            macos_commands(
                path,
                "gui/501",
                "gui/501/com.pinacora.rotation",
                false,
                false
            )
            .len(),
            1
        );
    }

    #[test]
    fn readiness_retries_missing_socket_and_stops_on_success() {
        let mut attempts = 0;
        wait_until_ready(
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(std::io::Error::from(std::io::ErrorKind::NotFound).into())
                } else {
                    Ok(())
                }
            },
            std::time::Duration::from_secs(1),
            std::time::Duration::ZERO,
        )
        .unwrap();
        assert_eq!(attempts, 3);
    }

    #[test]
    fn readiness_timeout_and_protocol_errors_are_actionable() {
        let error = wait_until_ready(
            || Err(std::io::Error::from(std::io::ErrorKind::ConnectionRefused).into()),
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        )
        .unwrap_err();
        assert!(error.to_string().contains("did not become ready"));
        let mut attempts = 0;
        let error = wait_until_ready(
            || {
                attempts += 1;
                anyhow::bail!("Invalid service reply")
            },
            std::time::Duration::from_secs(1),
            std::time::Duration::ZERO,
        )
        .unwrap_err();
        assert_eq!(attempts, 1);
        assert!(format!("{error:#}").contains("Invalid service reply"));
    }

    #[test]
    fn registration_updates_are_atomic_idempotent_and_private() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/service");
        assert!(write_changed(&path, b"one").unwrap());
        assert!(!write_changed(&path, b"one").unwrap());
        assert!(write_changed(&path, b"two").unwrap());
        assert_eq!(fs::read(&path).unwrap(), b"two");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
}
