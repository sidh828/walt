//! X11 wallpaper backend, standing in for the hyprpaper one (mounted as
//! `backend::hyprpaper` so the rest of Walt is unchanged).
//!
//! Wallpapers are applied by a setter command: `$WALT_SET_WALLPAPER`, else
//! `~/.config/scripts/set-wallpaper.sh` if it exists, else `feh --bg-fill`.
//! The setter covers the whole X screen, so every monitor shows the same
//! wallpaper; per-monitor requests apply to all of them.
//! The active wallpaper is read from `~/.cache/wallpaper/current` (written by
//! set-wallpaper.sh) or, failing that, from `~/.fehbg`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use log::{debug, info, warn};

const SETTER_ENV: &str = "WALT_SET_WALLPAPER";
const SETTER_SCRIPT: &str = ".config/scripts/set-wallpaper.sh";
const CURRENT_FILE: &str = "wallpaper/current";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveWallpaperAssignment {
    pub monitor_name: String,
    pub wallpaper_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Monitor {
    pub name: String,
}

// Variant names are kept from the hyprpaper backend; the rotation code matches on them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendUnavailableReason {
    HyprpaperUnavailable,
    NoMonitors,
    MonitorNotReady,
}

pub fn get_monitors() -> Vec<Monitor> {
    let output = match Command::new("xrandr").arg("--listactivemonitors").output() {
        Ok(output) if output.status.success() => output,
        _ => return vec![],
    };
    let monitors = parse_monitors(&String::from_utf8_lossy(&output.stdout));
    debug!("parsed monitors count={} monitors={:?}", monitors.len(), monitors);
    monitors
}

/// `xrandr --listactivemonitors`: a "Monitors: N" header, then one
/// " 0: +*eDP-1 1920/286x1200/178+0+0  eDP-1" line per monitor.
fn parse_monitors(text: &str) -> Vec<Monitor> {
    text.lines()
        .skip(1)
        .filter_map(|line| line.split_whitespace().last())
        .map(|name| Monitor {
            name: name.to_string(),
        })
        .collect()
}

pub fn classify_backend_unavailable(error: &anyhow::Error) -> Option<BackendUnavailableReason> {
    classify_backend_unavailable_message(&error.to_string())
}

fn classify_backend_unavailable_message(message: &str) -> Option<BackendUnavailableReason> {
    let normalized = message.to_lowercase();

    if normalized.contains("couldn't connect to")
        && (normalized.contains(".hyprpaper.sock") || normalized.contains("hyprpaper"))
    {
        return Some(BackendUnavailableReason::HyprpaperUnavailable);
    }

    if normalized.contains("no monitors found") {
        return Some(BackendUnavailableReason::NoMonitors);
    }

    if normalized.contains("monitor")
        && (normalized.contains("not found")
            || normalized.contains("not ready")
            || normalized.contains("unknown output")
            || normalized.contains("invalid format"))
    {
        return Some(BackendUnavailableReason::MonitorNotReady);
    }

    None
}

pub fn set_wallpaper(wallpaper_path: &str) -> anyhow::Result<()> {
    info!("applying wallpaper path={wallpaper_path}");
    if get_monitors().is_empty() {
        return Err(anyhow::anyhow!("No monitors found"));
    }
    run_setter(Path::new(wallpaper_path))
}

pub fn set_wallpaper_for_monitor(monitor_name: &str, wallpaper_path: &str) -> anyhow::Result<()> {
    info!("applying wallpaper (whole screen on X11) monitor={monitor_name} path={wallpaper_path}");
    run_setter(Path::new(wallpaper_path))
}

pub fn set_wallpapers_for_monitors(assignments: &[(String, PathBuf)]) -> anyhow::Result<()> {
    info!("applying wallpaper batch assignments={}", assignments.len());
    let Some((_, first)) = assignments.first() else {
        return Ok(());
    };
    if assignments.iter().any(|(_, path)| path != first) {
        warn!("different wallpapers per monitor aren't supported on X11; using {}", first.display());
    }
    run_setter(first)
}

#[allow(dead_code)]
pub fn get_active_wallpaper_assignments() -> anyhow::Result<Vec<ActiveWallpaperAssignment>> {
    Ok(get_active_wallpaper_assignments_if_supported()?.unwrap_or_default())
}

#[allow(dead_code)]
pub fn get_active_wallpapers() -> anyhow::Result<Vec<PathBuf>> {
    Ok(current_wallpaper().into_iter().collect())
}

pub(crate) fn get_active_wallpaper_assignments_if_supported(
) -> anyhow::Result<Option<Vec<ActiveWallpaperAssignment>>> {
    let Some(path) = current_wallpaper() else {
        return Ok(Some(vec![]));
    };
    Ok(Some(
        get_monitors()
            .into_iter()
            .map(|monitor| ActiveWallpaperAssignment {
                monitor_name: monitor.name,
                wallpaper_path: path.clone(),
            })
            .collect(),
    ))
}

fn current_wallpaper() -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    if let Some(cache) = dirs::cache_dir() {
        if let Ok(text) = fs::read_to_string(cache.join(CURRENT_FILE)) {
            let path = PathBuf::from(text.trim());
            if path.is_file() {
                return Some(path);
            }
        }
    }
    // feh --bg-fill '/path/to/image' (the last quoted argument)
    let fehbg = fs::read_to_string(home.join(".fehbg")).ok()?;
    let line = fehbg.lines().rev().find(|line| line.contains("feh"))?;
    let path = PathBuf::from(line.trim().rsplit('\'').nth(1)?);
    path.is_file().then_some(path)
}

fn run_setter(wallpaper: &Path) -> anyhow::Result<()> {
    let mut command = setter_command();
    command.arg(wallpaper);
    debug!("running wallpaper setter {:?}", command);
    let output = command
        .output()
        .map_err(|e| anyhow::anyhow!("Couldn't run wallpaper setter {:?}: {e}", command.get_program()))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow::anyhow!(
            "wallpaper setter failed: status {}, stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn setter_command() -> Command {
    if let Some(setter) = std::env::var_os(SETTER_ENV).filter(|s| !s.is_empty()) {
        return Command::new(setter);
    }
    if let Some(script) = dirs::home_dir().map(|home| home.join(SETTER_SCRIPT)) {
        if script.is_file() {
            return Command::new(script);
        }
    }
    let mut feh = Command::new("feh");
    feh.arg("--bg-fill");
    feh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_xrandr_active_monitors() {
        let text = "Monitors: 2\n 0: +*eDP-1 1920/286x1200/178+0+0  eDP-1\n 1: +HDMI-1 2560/597x1440/336+1920+0  HDMI-1\n";
        assert_eq!(
            parse_monitors(text),
            vec![
                Monitor { name: "eDP-1".into() },
                Monitor { name: "HDMI-1".into() }
            ]
        );
    }

    #[test]
    fn classifies_missing_monitors() {
        assert_eq!(
            classify_backend_unavailable_message("No monitors found"),
            Some(BackendUnavailableReason::NoMonitors)
        );
    }
}
