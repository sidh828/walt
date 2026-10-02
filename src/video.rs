//! Video wallpapers (played by the wallpaper setter). The image crate can't
//! read them, so previews and dimensions come from ffmpeg/ffprobe.

use std::{path::Path, process::Command};

pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "webm", "mkv", "mov"];

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .map(|ext| VIDEO_EXTENSIONS.contains(&ext.to_string_lossy().to_lowercase().as_str()))
        .unwrap_or(false)
}

/// The first frame, decoded.
pub fn first_frame(path: &Path) -> anyhow::Result<image::DynamicImage> {
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .output()
        .map_err(|e| anyhow::anyhow!("Couldn't run ffmpeg: {e}"))?;
    if !output.status.success() || output.stdout.is_empty() {
        return Err(anyhow::anyhow!(
            "ffmpeg couldn't read a frame: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(image::load_from_memory(&output.stdout)?)
}

pub fn dimensions(path: &Path) -> Option<(u32, u32)> {
    let output = Command::new("ffprobe")
        .args(["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x"])
        .arg(path)
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (width, height) = text.trim().split_once('x')?;
    Some((width.parse().ok()?, height.parse().ok()?))
}
