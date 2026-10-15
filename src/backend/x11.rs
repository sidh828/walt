//! X11 wallpaper backend, standing in for the hyprpaper one (mounted as
//! `backend::hyprpaper` so the rest of Walt is unchanged).
//!
//! Wallpapers are applied by a setter command: `$WALT_SET_WALLPAPER`, else
//! `~/.config/scripts/set-wallpaper.sh` if it exists, else `feh --bg-fill`.
//! Only set-wallpaper.sh can target one monitor (`--monitor NAME FILE`); with
//! the others every monitor gets the same wallpaper.
//! The active wallpapers are read from set-wallpaper.sh's state:
//! `~/.cache/wallpaper/screen-<monitor>`, else `~/.cache/wallpaper/current`,
//! else `~/.fehbg`.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use log::{debug, info, warn};

const SETTER_ENV: &str = "WALT_SET_WALLPAPER";
const SETTER_SCRIPT: &str = ".config/scripts/set-wallpaper.sh";
const CURRENT_FILE: &str = "wallpaper/current";
const SCREEN_FILE_PREFIX: &str = "wallpaper/screen-";

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
    run_setter(None, Path::new(wallpaper_path), true)
}

pub fn set_wallpaper_for_monitor(monitor_name: &str, wallpaper_path: &str) -> anyhow::Result<()> {
    info!("applying wallpaper monitor={monitor_name} path={wallpaper_path}");
    run_setter(Some(monitor_name), Path::new(wallpaper_path), true)
}

pub fn set_wallpapers_for_monitors(assignments: &[(String, PathBuf)]) -> anyhow::Result<()> {
    info!("applying wallpaper batch assignments={}", assignments.len());
    // recolour once, for the last one, rather than after every monitor
    for (i, (monitor_name, wallpaper_path)) in assignments.iter().enumerate() {
        run_setter(Some(monitor_name), wallpaper_path, i + 1 == assignments.len())?;
    }
    Ok(())
}

#[allow(dead_code)]
pub fn get_active_wallpaper_assignments() -> anyhow::Result<Vec<ActiveWallpaperAssignment>> {
    Ok(get_active_wallpaper_assignments_if_supported()?.unwrap_or_default())
}

#[allow(dead_code)]
pub fn get_active_wallpapers() -> anyhow::Result<Vec<PathBuf>> {
    let mut wallpapers = Vec::new();
    for assignment in get_active_wallpaper_assignments()? {
        if !wallpapers.contains(&assignment.wallpaper_path) {
            wallpapers.push(assignment.wallpaper_path);
        }
    }
    Ok(wallpapers)
}

pub(crate) fn get_active_wallpaper_assignments_if_supported(
) -> anyhow::Result<Option<Vec<ActiveWallpaperAssignment>>> {
    let fallback = current_wallpaper();
    Ok(Some(
        get_monitors()
            .into_iter()
            .filter_map(|monitor| {
                let path = screen_wallpaper(&monitor.name).or_else(|| fallback.clone())?;
                Some(ActiveWallpaperAssignment {
                    monitor_name: monitor.name,
                    wallpaper_path: path,
                })
            })
            .collect(),
    ))
}

/// A wallpaper picked for this monitor alone (set-wallpaper.sh --monitor).
fn screen_wallpaper(monitor_name: &str) -> Option<PathBuf> {
    let file = dirs::cache_dir()?.join(format!("{SCREEN_FILE_PREFIX}{monitor_name}"));
    let path = PathBuf::from(fs::read_to_string(file).ok()?.trim());
    path.is_file().then_some(path)
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

/// Apply `wallpaper` to `monitor`, or to every monitor if none is given (or the
/// setter can't target one). `recolor` false skips set-wallpaper.sh's recolouring.
fn run_setter(monitor: Option<&str>, wallpaper: &Path, recolor: bool) -> anyhow::Result<()> {
    let (mut command, per_monitor) = setter_command();
    if per_monitor {
        if !recolor {
            command.arg("--no-recolor");
        }
        if let Some(monitor) = monitor {
            command.args(["--monitor", monitor]);
        }
    } else if monitor.is_some() {
        warn!("this wallpaper setter can't target one monitor; applying to all");
    }
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

/// The setter, and whether it takes set-wallpaper.sh's `--monitor`/`--no-recolor`.
fn setter_command() -> (Command, bool) {
    if let Some(setter) = std::env::var_os(SETTER_ENV).filter(|s| !s.is_empty()) {
        return (Command::new(setter), false);
    }
    if let Some(script) = dirs::home_dir().map(|home| home.join(SETTER_SCRIPT)) {
        if script.is_file() {
            return (Command::new(script), true);
        }
    }
    let mut feh = Command::new("feh");
    feh.arg("--bg-fill");
    (feh, false)
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
