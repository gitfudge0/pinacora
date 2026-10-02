use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{path::Path, process::Command};

fn run(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("Could not run {program}; install it and ensure it is on PATH"))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        bail!("{program} failed: {stdout} {stderr}");
    }
    Ok(stdout)
}

fn wallpaper_argument(monitor: &str, path: &str) -> Result<String> {
    anyhow::ensure!(
        !path.contains([',', '\n', '\r']) && !monitor.contains([',', '\n', '\r']),
        "hyprpaper cannot apply a path or monitor name containing commas or line breaks"
    );
    Ok(format!("{monitor},{path},cover"))
}

pub fn apply(path: &Path) -> Result<usize> {
    let path = path
        .canonicalize()
        .context("Wallpaper original is unavailable")?;
    let filename = path.to_str().context("Wallpaper path is not UTF-8")?;
    let desktop = std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    if std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some() || desktop.contains("hyprland") {
        // hyprpaper 0.8+ loads images with the wallpaper request; the old
        // preload/reload commands were removed. Target each monitor so existing
        // monitor-specific wallpapers cannot override a wildcard request.
        let version = run("hyprpaper", &["--version"])?;
        let version = version
            .split_whitespace()
            .find_map(|word| word.strip_prefix('v'))
            .context("Could not determine hyprpaper version; requires 0.8 or newer")?;
        let mut parts = version.split('.');
        let major: u32 = parts.next().unwrap_or_default().parse()?;
        let minor: u32 = parts.next().unwrap_or_default().parse()?;
        anyhow::ensure!(
            major > 0 || minor >= 8,
            "Requires hyprpaper 0.8 or newer; upgrade hyprpaper"
        );
        #[derive(Deserialize)]
        struct Monitor {
            name: String,
        }
        let monitors: Vec<Monitor> = serde_json::from_str(&run("hyprctl", &["-j", "monitors"])?)
            .context("Could not read Hyprland monitors")?;
        anyhow::ensure!(!monitors.is_empty(), "No connected Hyprland monitors found");
        for (applied, monitor) in monitors.iter().enumerate() {
            let argument = wallpaper_argument(&monitor.name, filename)?;
            let response =
                run("hyprctl", &["hyprpaper", "wallpaper", &argument]).with_context(|| {
                    format!(
                        "Applied to {applied} of {} displays; start hyprpaper with IPC enabled",
                        monitors.len()
                    )
                })?;
            anyhow::ensure!(
                response == "ok",
                "Applied to {applied} of {} displays: hyprpaper replied {response}; ensure hyprpaper is running with IPC enabled",
                monitors.len()
            );
        }
        return Ok(monitors.len());
    }
    if desktop
        .split(':')
        .any(|part| part == "gnome" || part == "ubuntu")
    {
        let uri = url::Url::from_file_path(&path)
            .map_err(|_| anyhow::anyhow!("Invalid wallpaper path"))?;
        for key in ["picture-uri", "picture-uri-dark"] {
            run(
                "gsettings",
                &["set", "org.gnome.desktop.background", key, uri.as_str()],
            )
            .with_context(|| format!("Could not update GNOME {key}"))?;
        }
        return Ok(1);
    }
    bail!(
        "Wallpaper application supports Hyprland with hyprpaper 0.8+ and GNOME. Current desktop: {desktop:?}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plans_each_monitor_without_shell_interpolation() {
        assert_eq!(
            wallpaper_argument("eDP-1", "/tmp/my art.jpg").unwrap(),
            "eDP-1,/tmp/my art.jpg,cover"
        );
        assert!(wallpaper_argument("eDP-1", "/tmp/a,b.jpg").is_err());
    }
}
