//! User settings: a versioned JSON document stored under `%APPDATA%\Lilt`.
//!
//! Loading never fails: missing, unreadable or invalid files fall back to defaults,
//! unknown fields are ignored, and every value is clamped to its valid range so a
//! hand-edited file cannot put the application into an unusable state.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::paths;

/// Current schema version. Bump together with a step in [`migrate`].
pub const CURRENT_VERSION: u32 = 1;

/// Settings files larger than this are rejected as corrupt.
const MAX_FILE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub general: General,
    pub appearance: Appearance,
    pub position: Position,
    pub widgets: Widgets,
    pub music: Music,
    pub ambient: Ambient,
    pub performance: Performance,
    pub shortcuts: Shortcuts,
    pub calendar: Calendar,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct General {
    /// Collapse the expanded island after a period without pointer interaction.
    pub auto_collapse: bool,
    pub auto_collapse_seconds: u32,
    /// Collapse when the user clicks anywhere outside the island.
    pub collapse_on_outside_click: bool,
    /// Briefly show track changes in the compact island.
    pub activity_banners: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MotionPreference {
    /// Follow the Windows "Animation effects" setting.
    System,
    Reduced,
    Full,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: String,
    /// Compact island width in logical pixels.
    pub compact_width: u32,
    /// Expanded island width in logical pixels.
    pub expanded_width: u32,
    pub motion: MotionPreference,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Position {
    /// Device name of the monitor (e.g. `\\.\DISPLAY2`); `None` selects the primary monitor.
    pub monitor: Option<String>,
    /// Horizontal offset from the monitor centre in logical pixels.
    pub offset_x: i32,
    /// Distance from the top of the work area in logical pixels.
    pub top_margin: u32,
    pub allow_drag: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Widgets {
    pub media: bool,
    pub calendar: bool,
    pub system: bool,
    pub quick_actions: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Music {
    pub show_artwork: bool,
    pub show_progress: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ambient {
    pub enabled: bool,
    /// 0 ..= 100.
    pub intensity: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Performance {
    /// Disables non-essential effects and lengthens sampling intervals.
    pub low_power: bool,
    /// Hide the island while a full-screen application is in the foreground.
    pub hide_in_fullscreen: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcuts {
    /// Global shortcut that toggles the expanded island, e.g. `Ctrl+Alt+Space`.
    pub toggle: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calendar {
    /// Local `.ics` file shown in the agenda preview.
    pub ics_path: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            general: General::default(),
            appearance: Appearance::default(),
            position: Position::default(),
            widgets: Widgets::default(),
            music: Music::default(),
            ambient: Ambient::default(),
            performance: Performance::default(),
            shortcuts: Shortcuts::default(),
            calendar: Calendar::default(),
        }
    }
}

impl Default for General {
    fn default() -> Self {
        Self { auto_collapse: true, auto_collapse_seconds: 8, collapse_on_outside_click: true, activity_banners: true }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self { theme: "lilt-dark".into(), compact_width: 196, expanded_width: 460, motion: MotionPreference::System }
    }
}

impl Default for Position {
    fn default() -> Self {
        Self { monitor: None, offset_x: 0, top_margin: 8, allow_drag: true }
    }
}

impl Default for Widgets {
    fn default() -> Self {
        Self { media: true, calendar: true, system: true, quick_actions: true }
    }
}

impl Default for Music {
    fn default() -> Self {
        Self { show_artwork: true, show_progress: true }
    }
}

impl Default for Ambient {
    fn default() -> Self {
        Self { enabled: true, intensity: 60 }
    }
}

impl Default for Performance {
    fn default() -> Self {
        Self { low_power: false, hide_in_fullscreen: true }
    }
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self { toggle: Some("Ctrl+Alt+Space".into()) }
    }
}

/// Valid ranges, shared with the settings UI.
pub mod limits {
    pub const COMPACT_WIDTH: (u32, u32) = (160, 300);
    pub const EXPANDED_WIDTH: (u32, u32) = (400, 560);
    pub const TOP_MARGIN: (u32, u32) = (0, 48);
    pub const OFFSET_X: (i32, i32) = (-1600, 1600);
    pub const AUTO_COLLAPSE_SECONDS: (u32, u32) = (3, 60);
    pub const AMBIENT_INTENSITY: (u32, u32) = (0, 100);
}

impl Settings {
    /// Clamps every value into its valid range.
    pub fn sanitize(mut self) -> Self {
        use limits::*;
        self.version = CURRENT_VERSION;
        if self.appearance.theme != "lilt-dark" {
            self.appearance.theme = "lilt-dark".into();
        }
        self.appearance.compact_width = self.appearance.compact_width.clamp(COMPACT_WIDTH.0, COMPACT_WIDTH.1);
        self.appearance.expanded_width = self.appearance.expanded_width.clamp(EXPANDED_WIDTH.0, EXPANDED_WIDTH.1);
        self.position.top_margin = self.position.top_margin.clamp(TOP_MARGIN.0, TOP_MARGIN.1);
        self.position.offset_x = self.position.offset_x.clamp(OFFSET_X.0, OFFSET_X.1);
        self.position.monitor = self.position.monitor.filter(|m| !m.is_empty() && m.len() <= 64);
        self.general.auto_collapse_seconds =
            self.general.auto_collapse_seconds.clamp(AUTO_COLLAPSE_SECONDS.0, AUTO_COLLAPSE_SECONDS.1);
        self.ambient.intensity = self.ambient.intensity.clamp(AMBIENT_INTENSITY.0, AMBIENT_INTENSITY.1);
        self.shortcuts.toggle = self.shortcuts.toggle.filter(|s| s.len() <= 48);
        self.calendar.ics_path = self
            .calendar
            .ics_path
            .filter(|p| p.is_absolute() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("ics")));
        self
    }
}

/// Upgrades an older document to the current schema. Each schema change adds one step.
fn migrate(mut value: serde_json::Value, from: u32) -> serde_json::Value {
    if from < 1 {
        // Documents without a version predate the first release; their layout is the
        // version 1 layout, so only the marker is added.
        value["version"] = 1.into();
    }
    value
}

/// Outcome of [`load_from`], so callers can log what happened.
#[derive(Debug, PartialEq)]
pub enum LoadStatus {
    Loaded,
    Missing,
    /// The file could not be parsed; defaults are used and the file was backed up.
    Invalid,
    /// Written by a newer Lilt; it was backed up before being read leniently.
    Newer(u32),
}

pub fn load_from(path: &Path) -> (Settings, LoadStatus) {
    let text = match std::fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_FILE_BYTES => None,
        Ok(_) => std::fs::read_to_string(path).ok(),
        Err(_) => return (Settings::default(), LoadStatus::Missing),
    };
    let Some(value) = text.and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()).filter(|v| v.is_object())
    else {
        backup(path, "invalid");
        return (Settings::default(), LoadStatus::Invalid);
    };

    let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let status = if version > CURRENT_VERSION {
        backup(path, &format!("v{version}"));
        LoadStatus::Newer(version)
    } else {
        LoadStatus::Loaded
    };
    match serde_json::from_value::<Settings>(migrate(value, version)) {
        Ok(settings) => (settings.sanitize(), status),
        Err(_) => {
            backup(path, "invalid");
            (Settings::default(), LoadStatus::Invalid)
        }
    }
}

fn backup(path: &Path, tag: &str) {
    let target = path.with_extension(format!("{tag}.bak"));
    let _ = std::fs::copy(path, target);
}

/// Writes atomically: a temporary file is written and then renamed over the target.
pub fn save_to(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let json = serde_json::to_vec_pretty(settings).map_err(std::io::Error::other)?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, json)?;
    std::fs::rename(&temp, path)
}

pub fn settings_path() -> PathBuf {
    paths::roaming_data_dir().join("settings.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("lilt-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = temp_dir("missing");
        assert_eq!(load_from(&dir.join("settings.json")), (Settings::default(), LoadStatus::Missing));
    }

    #[test]
    fn round_trip() {
        let dir = temp_dir("roundtrip");
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        s.ambient.intensity = 35;
        s.widgets.calendar = false;
        s.position.monitor = Some(r"\\.\DISPLAY2".into());
        save_to(&path, &s).unwrap();
        assert_eq!(load_from(&path), (s, LoadStatus::Loaded));
        assert!(!path.with_extension("json.tmp").exists());
    }

    #[test]
    fn values_are_clamped() {
        let dir = temp_dir("clamp");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"version":1,"appearance":{"compact_width":5000,"theme":"nope"},"ambient":{"intensity":900},"calendar":{"ics_path":"relative.txt"}}"#).unwrap();
        let (s, status) = load_from(&path);
        assert_eq!(status, LoadStatus::Loaded);
        assert_eq!(s.appearance.compact_width, limits::COMPACT_WIDTH.1);
        assert_eq!(s.appearance.theme, "lilt-dark");
        assert_eq!(s.ambient.intensity, 100);
        assert_eq!(s.calendar.ics_path, None);
        // Unspecified sections keep their defaults.
        assert_eq!(s.widgets, Widgets::default());
    }

    #[test]
    fn invalid_file_is_backed_up() {
        let dir = temp_dir("invalid");
        let path = dir.join("settings.json");
        std::fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_from(&path), (Settings::default(), LoadStatus::Invalid));
        assert!(dir.join("settings.invalid.bak").exists());
    }

    #[test]
    fn wrong_types_fall_back_to_defaults() {
        let dir = temp_dir("types");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"version":1,"ambient":{"intensity":"loud"}}"#).unwrap();
        assert_eq!(load_from(&path).1, LoadStatus::Invalid);
    }

    #[test]
    fn unversioned_document_is_migrated() {
        let dir = temp_dir("unversioned");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"music":{"show_progress":false}}"#).unwrap();
        let (s, status) = load_from(&path);
        assert_eq!(status, LoadStatus::Loaded);
        assert_eq!(s.version, CURRENT_VERSION);
        assert!(!s.music.show_progress);
    }

    #[test]
    fn newer_document_is_backed_up_and_read_leniently() {
        let dir = temp_dir("newer");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"version":7,"future_section":{"x":1},"widgets":{"system":false}}"#).unwrap();
        let (s, status) = load_from(&path);
        assert_eq!(status, LoadStatus::Newer(7));
        assert!(!s.widgets.system);
        assert!(dir.join("settings.v7.bak").exists());
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = temp_dir("oversized");
        let path = dir.join("settings.json");
        std::fs::write(&path, vec![b' '; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert_eq!(load_from(&path).1, LoadStatus::Invalid);
    }
}
