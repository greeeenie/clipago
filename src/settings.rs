use anyhow::{Context, Result};
use directories::{ProjectDirs, UserDirs};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum Encoder {
    #[default]
    Auto,
    Nvidia,
    Intel,
    Amd,
    Cpu,
}

impl Encoder {
    pub const ALL: [Self; 5] = [Self::Auto, Self::Nvidia, Self::Intel, Self::Amd, Self::Cpu];
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "Автоматически",
            Self::Nvidia => "NVIDIA NVENC",
            Self::Intel => "Intel Quick Sync",
            Self::Amd => "AMD AMF",
            Self::Cpu => "CPU · x264",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub enum ReplayShortcut {
    #[default]
    CtrlShiftS,
    AltF10,
    CtrlAltS,
}

impl ReplayShortcut {
    pub const ALL: [Self; 3] = [Self::CtrlShiftS, Self::AltF10, Self::CtrlAltS];
    pub fn label(self) -> &'static str {
        match self {
            Self::CtrlShiftS => "Ctrl + Shift + S",
            Self::AltF10 => "Alt + F10",
            Self::CtrlAltS => "Ctrl + Alt + S",
        }
    }
}

pub fn snap_buffer_seconds(seconds: u32) -> u32 {
    ((seconds.clamp(30, 120) + 2) / 5) * 5
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct EditorHotkeys {
    pub split: String,
    pub duplicate: String,
    pub delete: String,
    pub snapping: String,
    pub inspector: String,
    pub export: String,
    pub undo: String,
    pub redo: String,
    pub play: String,
    pub zoom_in: String,
    pub zoom_out: String,
}

impl Default for EditorHotkeys {
    fn default() -> Self {
        Self {
            split: "KeyB".into(),
            duplicate: "Ctrl+KeyD".into(),
            delete: "Delete".into(),
            snapping: "KeyM".into(),
            inspector: "KeyP".into(),
            export: "Ctrl+KeyE".into(),
            undo: "Ctrl+KeyZ".into(),
            redo: "Ctrl+Shift+KeyZ".into(),
            play: "Space".into(),
            zoom_in: "Equal".into(),
            zoom_out: "Minus".into(),
        }
    }
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub struct Settings {
    pub buffer_seconds: u32,
    pub fps: u32,
    pub monitor: u32,
    pub encoder: Encoder,
    pub output_dir: PathBuf,
    pub ffmpeg: String,
    pub ffprobe: String,
    pub shortcut: ReplayShortcut,
    pub system_audio: bool,
    pub editor_hotkeys: Box<EditorHotkeys>,
}

impl Default for Settings {
    fn default() -> Self {
        let base = UserDirs::new()
            .and_then(|d| d.video_dir().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            buffer_seconds: 60,
            fps: 30,
            monitor: 0,
            encoder: Encoder::Auto,
            output_dir: base.join("Rewind"),
            ffmpeg: "ffmpeg".into(),
            ffprobe: "ffprobe".into(),
            shortcut: ReplayShortcut::default(),
            system_audio: true,
            editor_hotkeys: Box::default(),
        }
    }
}

pub fn app_dirs() -> Result<ProjectDirs> {
    ProjectDirs::from("app", "Rewind", "Rewind").context("Не удалось найти папку приложения")
}

impl Settings {
    pub fn load() -> Result<Self> {
        let path = config_file()?;
        Self::load_from(&path)
    }

    fn load_from(path: &std::path::Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let mut settings: Self = serde_json::from_slice(&std::fs::read(path)?)?;
        settings.buffer_seconds = snap_buffer_seconds(settings.buffer_seconds);
        if ![30, 60].contains(&settings.fps) {
            settings.fps = 30;
        }
        Ok(settings)
    }

    pub fn save(&self) -> Result<()> {
        let path = config_file()?;
        self.save_to(&path)
    }

    fn save_to(&self, path: &std::path::Path) -> Result<()> {
        std::fs::create_dir_all(path.parent().unwrap())?;
        let temporary = path.with_extension("tmp");
        let mut normalized = self.clone();
        normalized.buffer_seconds = snap_buffer_seconds(normalized.buffer_seconds);
        std::fs::write(&temporary, serde_json::to_vec_pretty(&normalized)?)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}

pub fn data_file(name: &str) -> Result<PathBuf> {
    // Isolated developer/test profiles never touch the user's recording settings.
    Ok(std::env::var_os("REWIND_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or(app_dirs()?.config_dir().to_path_buf())
        .join(name))
}
fn config_file() -> Result<PathBuf> {
    data_file("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_buffer_stop_round_trips_and_old_values_snap() {
        let work = crate::media::WorkDir::new(&std::env::temp_dir(), "rewind-settings").unwrap();
        let path = work.0.join("settings.json");
        assert_eq!(Settings::load_from(&path).unwrap().buffer_seconds, 60);
        for seconds in (30..=120).step_by(5) {
            let settings = Settings {
                buffer_seconds: seconds,
                shortcut: ReplayShortcut::AltF10,
                ..Default::default()
            };
            settings.save_to(&path).unwrap();
            let restored = Settings::load_from(&path).unwrap();
            assert_eq!(restored.buffer_seconds, seconds);
            assert_eq!(restored.shortcut, ReplayShortcut::AltF10);
        }
        for (input, expected) in [
            (0, 30),
            (32, 30),
            (33, 35),
            (63, 65),
            (87, 85),
            (u32::MAX, 120),
        ] {
            assert_eq!(snap_buffer_seconds(input), expected);
        }
        std::fs::write(&path, br#"{"buffer_seconds":63}"#).unwrap();
        assert_eq!(Settings::load_from(&path).unwrap().buffer_seconds, 65);
    }
}
