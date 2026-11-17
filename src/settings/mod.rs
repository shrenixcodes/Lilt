//! User settings: a versioned JSON document stored under `%APPDATA%\Lilt`.
//!
//! Loading never fails: missing, unreadable or invalid files fall back to defaults,
//! unknown fields are ignored, and every value is clamped to its valid range so a
//! hand-edited file cannot put the application into an unusable state.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use crate::paths;

/// Current schema version. Bump together with a step in [`migrate`].
pub const CURRENT_VERSION: u32 = 3;

/// Settings files larger than this are rejected as corrupt.
const MAX_FILE_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub general: General,
    pub clock: Clock,
    pub weather_location: Option<WeatherLocation>,
    pub weather_location_name: String,
    pub appearance: Appearance,
    pub position: Position,
    pub widgets: Widgets,
    pub music: Music,
    pub ambient: Ambient,
    pub performance: Performance,
    pub shortcuts: Shortcuts,
    pub calendar: Calendar,
    pub tasks: Tasks,
    pub clipboard: Clipboard,
}

/// Clock formats: time 0 = Windows, 1 = 12-hour, 2 = 24-hour;
/// date 0 = day/month name, 1 = month name/day, 2 = ISO year-month-day.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clock {
    pub time_format: u8,
    pub date_format: u8,
    pub cycle_enabled: bool,
    pub clock_seconds: u32,
    pub item_seconds: u32,
}

impl Default for Clock {
    fn default() -> Self {
        Self { time_format: 0, date_format: 0, cycle_enabled: true, clock_seconds: 7, item_seconds: 5 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct WeatherLocation {
    pub latitude: f64,
    pub longitude: f64,
}

impl WeatherLocation {
    pub fn new(latitude: f64, longitude: f64) -> Option<Self> {
        (latitude.is_finite()
            && longitude.is_finite()
            && (-90.0..=90.0).contains(&latitude)
            && (-180.0..=180.0).contains(&longitude))
        .then_some(Self { latitude, longitude })
    }
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
    /// Keep the island tucked away, revealing it only when the pointer reaches the top
    /// centre of the screen.
    pub auto_hide: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MotionPreference {
    /// Follow the Windows "Animation effects" setting.
    System,
    Reduced,
    Full,
}

/// The overall look of Lilt. Stored by name; a name this version does not know falls
/// back to Midnight without affecting any other setting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum AppearanceMode {
    /// The original minimal dark appearance.
    #[default]
    Midnight,
    /// Dark smoked translucent glass.
    SmokedGlass,
    /// Clear glass that darkens only as much as the desktop behind it requires.
    LiquidGlass,
}

impl AppearanceMode {
    pub fn id(self) -> &'static str {
        match self {
            Self::Midnight => "midnight",
            Self::SmokedGlass => "smoked-glass",
            Self::LiquidGlass => "liquid-glass",
        }
    }

    /// Whether the appearance is one of the glass ones.
    pub fn is_glass(self) -> bool {
        self != Self::Midnight
    }
}

impl From<String> for AppearanceMode {
    fn from(id: String) -> Self {
        match id.as_str() {
            "smoked-glass" => Self::SmokedGlass,
            "liquid-glass" => Self::LiquidGlass,
            // Includes "lilt-dark", Midnight's name in version 1.
            _ => Self::Midnight,
        }
    }
}

impl From<AppearanceMode> for String {
    fn from(mode: AppearanceMode) -> Self {
        mode.id().into()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: AppearanceMode,
    /// Use frosted surfaces instead of a live blur in the glass appearances.
    pub reduce_transparency: bool,
    /// How clear the glass appearances are over the live blur, 0..=100 percent. 50 is
    /// each appearance's own design; Lilt still darkens the glass over bright windows so
    /// text stays legible.
    pub glass_transparency: u32,
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
    /// The Tasks side tab of the expanded island.
    pub tasks: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Music {
    pub show_artwork: bool,
    pub show_progress: bool,
    /// Hovering the compact island fades the now-playing visualizer out and the
    /// play/next buttons in. Off keeps the visualizer visible at all times.
    pub hover_controls: bool,
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskOrder {
    /// Priority, then the urgency of the due date.
    #[default]
    Priority,
    /// Priority, then the user's own order.
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TaskLayout {
    /// The tasks box sits to the right of the expanded island.
    #[default]
    Beside,
    /// The tasks box sits below the expanded island.
    Below,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Tasks {
    pub order: TaskOrder,
    pub layout: TaskLayout,
}

/// How long copied text is kept. Pinned entries and snippets are kept until deleted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ClipboardRetention {
    Day,
    Week,
    #[default]
    Month,
    Quarter,
    Forever,
}

impl ClipboardRetention {
    pub const ALL: [Self; 5] = [Self::Day, Self::Week, Self::Month, Self::Quarter, Self::Forever];

    pub fn retention(self) -> crate::clipboard::Retention {
        use crate::clipboard::Retention::{Days, Forever};
        match self {
            Self::Day => Days(1),
            Self::Week => Days(7),
            Self::Month => Days(30),
            Self::Quarter => Days(90),
            Self::Forever => Forever,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clipboard {
    /// The clipboard shelf in the box beside the expanded island.
    pub show: bool,
    /// Remember copied text. Off until the user turns it on.
    pub capture: bool,
    /// The shelf has explained capture once (the user turned it on or chose "Not now").
    pub introduced: bool,
    /// Skip copies when Windows does not say which app made them.
    pub skip_unknown_sources: bool,
    /// Skip text that looks like a password, key or token.
    pub skip_secrets: bool,
    /// Executable names whose copies are never remembered.
    pub excluded_apps: Vec<String>,
    pub retention: ClipboardRetention,
    /// Most captured entries kept; pinned entries and snippets do not count.
    pub max_items: u32,
    /// Global shortcut that opens the shelf. None by default: the obvious choice,
    /// Ctrl+Shift+V, is "paste as plain text" in browsers and "paste" in Windows Terminal,
    /// and a global shortcut would take it from them.
    pub shortcut: Option<String>,
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
            clock: Clock::default(),
            weather_location: None,
            weather_location_name: String::new(),
            appearance: Appearance::default(),
            position: Position::default(),
            widgets: Widgets::default(),
            music: Music::default(),
            ambient: Ambient::default(),
            performance: Performance::default(),
            shortcuts: Shortcuts::default(),
            calendar: Calendar::default(),
            tasks: Tasks::default(),
            clipboard: Clipboard::default(),
        }
    }
}

impl Default for General {
    fn default() -> Self {
        Self {
            auto_collapse: true,
            auto_collapse_seconds: 8,
            collapse_on_outside_click: true,
            activity_banners: true,
            auto_hide: false,
        }
    }
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: AppearanceMode::Midnight,
            reduce_transparency: false,
            glass_transparency: 50,
            compact_width: 196,
            expanded_width: 460,
            motion: MotionPreference::System,
        }
    }
}

impl Default for Position {
    fn default() -> Self {
        Self { monitor: None, offset_x: 0, top_margin: 8, allow_drag: true }
    }
}

impl Default for Widgets {
    fn default() -> Self {
        Self { media: true, calendar: true, system: true, quick_actions: true, tasks: true }
    }
}

impl Default for Music {
    fn default() -> Self {
        Self { show_artwork: true, show_progress: true, hover_controls: true }
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

impl Default for Clipboard {
    fn default() -> Self {
        Self {
            show: true,
            capture: false,
            introduced: false,
            skip_unknown_sources: false,
            skip_secrets: true,
            excluded_apps: crate::clipboard::DEFAULT_EXCLUDED_APPS.iter().map(|s| (*s).to_owned()).collect(),
            retention: ClipboardRetention::Month,
            max_items: 500,
            shortcut: None,
        }
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
    pub const GLASS_TRANSPARENCY: (u32, u32) = (0, 100);
    pub const CLIPBOARD_MAX_ITEMS: (u32, u32) = (50, 5000);
    /// Excluded apps kept, and the longest name.
    pub const EXCLUDED_APPS: usize = 100;
    pub const APP_NAME_CHARS: usize = 128;
}

impl Settings {
    /// Clamps every value into its valid range.
    pub fn sanitize(mut self) -> Self {
        use limits::*;
        self.version = CURRENT_VERSION;
        self.clock.time_format = self.clock.time_format.min(2);
        self.clock.date_format = self.clock.date_format.min(2);
        self.clock.clock_seconds = self.clock.clock_seconds.clamp(3, 60);
        self.clock.item_seconds = self.clock.item_seconds.clamp(3, 60);
        self.weather_location_name = self.weather_location_name.chars().filter(|c| !c.is_control()).take(160).collect();
        self.weather_location = self.weather_location.and_then(|p| WeatherLocation::new(p.latitude, p.longitude));
        self.appearance.compact_width = self.appearance.compact_width.clamp(COMPACT_WIDTH.0, COMPACT_WIDTH.1);
        self.appearance.expanded_width = self.appearance.expanded_width.clamp(EXPANDED_WIDTH.0, EXPANDED_WIDTH.1);
        self.position.top_margin = self.position.top_margin.clamp(TOP_MARGIN.0, TOP_MARGIN.1);
        self.position.offset_x = self.position.offset_x.clamp(OFFSET_X.0, OFFSET_X.1);
        self.position.monitor = self.position.monitor.filter(|m| !m.is_empty() && m.len() <= 64);
        self.general.auto_collapse_seconds =
            self.general.auto_collapse_seconds.clamp(AUTO_COLLAPSE_SECONDS.0, AUTO_COLLAPSE_SECONDS.1);
        self.ambient.intensity = self.ambient.intensity.clamp(AMBIENT_INTENSITY.0, AMBIENT_INTENSITY.1);
        self.appearance.glass_transparency =
            self.appearance.glass_transparency.clamp(GLASS_TRANSPARENCY.0, GLASS_TRANSPARENCY.1);
        self.shortcuts.toggle = self.shortcuts.toggle.filter(|s| s.len() <= 48);
        self.clipboard.shortcut = self.clipboard.shortcut.filter(|s| s.len() <= 48);
        self.clipboard.max_items = self.clipboard.max_items.clamp(CLIPBOARD_MAX_ITEMS.0, CLIPBOARD_MAX_ITEMS.1);
        let mut apps: Vec<String> = Vec::new();
        for app in &self.clipboard.excluded_apps {
            let app = app.trim();
            if !app.is_empty()
                && app.chars().count() <= APP_NAME_CHARS
                && !apps.iter().any(|a| a.eq_ignore_ascii_case(app))
                && apps.len() < EXCLUDED_APPS
            {
                apps.push(app.to_owned());
            }
        }
        self.clipboard.excluded_apps = apps;
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
    if from < 2 {
        // Version 2 added appearances; the single version 1 theme is now called Midnight.
        if let Some(theme) = value.pointer_mut("/appearance/theme")
            && theme == "lilt-dark"
        {
            *theme = AppearanceMode::Midnight.id().into();
        }
        value["version"] = 2.into();
    }
    if from < 3 {
        // Version 3 added a clear Liquid Glass; version 2's Liquid Glass was the smoked
        // glass, now called Smoked Glass, so those who chose it keep the same look.
        if let Some(theme) = value.pointer_mut("/appearance/theme")
            && theme == "liquid-glass"
        {
            *theme = AppearanceMode::SmokedGlass.id().into();
        }
        value["version"] = 3.into();
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
    #[test]
    fn idle_timing_defaults_and_user_values_are_safe() {
        let settings: Settings = serde_json::from_str(r#"{"clock":{"time_format":1}}"#).unwrap();
        assert_eq!(settings.clock.clock_seconds, 7);
        assert_eq!(settings.clock.item_seconds, 5);
        assert!(settings.clock.cycle_enabled);
        let mut settings = settings;
        settings.clock.clock_seconds = 0;
        settings.clock.item_seconds = 999;
        settings.clock.cycle_enabled = false;
        settings.weather_location_name = "City\nName".into();
        let settings = settings.sanitize();
        assert_eq!(settings.clock.clock_seconds, 3);
        assert_eq!(settings.clock.item_seconds, 60);
        assert!(!settings.clock.cycle_enabled);
        assert_eq!(settings.weather_location_name, "CityName");
    }

    #[test]
    fn manual_weather_location_and_clock_settings_are_safe_and_round_trip() {
        let defaults: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.weather_location, None);
        assert_eq!(defaults.clock, Clock::default());
        assert!(WeatherLocation::new(f64::NAN, 0.0).is_none());
        assert!(WeatherLocation::new(0.0, f64::INFINITY).is_none());
        assert!(WeatherLocation::new(91.0, 0.0).is_none());
        assert!(WeatherLocation::new(0.0, -181.0).is_none());
        let mut chosen = defaults;
        chosen.clock = Clock { time_format: 1, date_format: 2, ..Default::default() };
        chosen.weather_location = WeatherLocation::new(22.5726, 88.3639);
        let text = serde_json::to_string(&chosen).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap().sanitize(), chosen);
        chosen.weather_location = Some(WeatherLocation { latitude: 999.0, longitude: 0.0 });
        assert_eq!(chosen.sanitize().weather_location, None);
    }

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
        assert_eq!(s.appearance.theme, AppearanceMode::Midnight);
        assert_eq!(s.ambient.intensity, 100);
        assert_eq!(s.calendar.ics_path, None);
        // Unspecified sections keep their defaults.
        assert_eq!(s.widgets, Widgets::default());
    }

    #[test]
    fn clipboard_capture_is_off_by_default_and_values_are_cleaned() {
        let defaults = Clipboard::default();
        assert!(!defaults.capture && !defaults.introduced && defaults.skip_secrets && defaults.show);
        assert_eq!(defaults.shortcut, None);
        assert!(defaults.excluded_apps.iter().any(|a| a == "KeePass.exe"));
        let dir = temp_dir("clipboard");
        let path = dir.join("settings.json");
        // Earlier versions have no clipboard section: it takes the defaults.
        std::fs::write(&path, r#"{"version":3}"#).unwrap();
        assert_eq!(load_from(&path).0.clipboard, Clipboard::default());
        std::fs::write(
            &path,
            r#"{"version":3,"clipboard":{"capture":true,"max_items":1,"retention":"week",
               "excluded_apps":[" Foo.exe ","foo.exe","","Bar.exe"]}}"#,
        )
        .unwrap();
        let (s, status) = load_from(&path);
        assert_eq!(status, LoadStatus::Loaded);
        assert!(s.clipboard.capture);
        assert_eq!(s.clipboard.max_items, limits::CLIPBOARD_MAX_ITEMS.0);
        assert_eq!(s.clipboard.retention, ClipboardRetention::Week);
        assert_eq!(s.clipboard.excluded_apps, ["Foo.exe", "Bar.exe"]);
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
    fn new_installations_use_midnight() {
        assert_eq!(Settings::default().appearance.theme, AppearanceMode::Midnight);
        assert!(!Settings::default().appearance.reduce_transparency);
        assert_eq!(Settings::default().appearance.glass_transparency, 50);
    }

    #[test]
    fn version_1_documents_keep_their_appearance_and_settings() {
        let dir = temp_dir("v1");
        let path = dir.join("settings.json");
        std::fs::write(
            &path,
            r#"{"version":1,"appearance":{"theme":"lilt-dark","compact_width":220,"expanded_width":500,"motion":"reduced"},"ambient":{"intensity":35}}"#,
        )
        .unwrap();
        let (s, status) = load_from(&path);
        assert_eq!(status, LoadStatus::Loaded);
        assert_eq!(s.version, CURRENT_VERSION);
        assert_eq!(s.appearance.theme, AppearanceMode::Midnight);
        assert_eq!((s.appearance.compact_width, s.appearance.expanded_width), (220, 500));
        assert_eq!(s.appearance.motion, MotionPreference::Reduced);
        assert_eq!(s.ambient.intensity, 35);
        // Loading never writes: the file is upgraded on the next real change.
        assert!(std::fs::read_to_string(&path).unwrap().contains("lilt-dark"));
    }

    #[test]
    fn version_2_liquid_glass_keeps_its_smoked_look() {
        let dir = temp_dir("v2-glass");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"version":2,"appearance":{"theme":"liquid-glass","reduce_transparency":true}}"#)
            .unwrap();
        let (s, status) = load_from(&path);
        assert_eq!(status, LoadStatus::Loaded);
        assert_eq!(s.appearance.theme, AppearanceMode::SmokedGlass);
        assert!(s.appearance.reduce_transparency);
        // In a version 3 document the name means the new, clear Liquid Glass.
        std::fs::write(&path, r#"{"version":3,"appearance":{"theme":"liquid-glass"}}"#).unwrap();
        assert_eq!(load_from(&path).0.appearance.theme, AppearanceMode::LiquidGlass);
        std::fs::write(&path, r#"{"version":3,"appearance":{"theme":"smoked-glass"}}"#).unwrap();
        assert_eq!(load_from(&path).0.appearance.theme, AppearanceMode::SmokedGlass);
    }

    #[test]
    fn appearance_is_saved_by_name() {
        let dir = temp_dir("appearance");
        let path = dir.join("settings.json");
        let mut s = Settings::default();
        s.appearance.theme = AppearanceMode::LiquidGlass;
        s.appearance.reduce_transparency = true;
        save_to(&path, &s).unwrap();
        assert!(std::fs::read_to_string(&path).unwrap().contains(r#""theme": "liquid-glass""#));
        assert_eq!(load_from(&path), (s, LoadStatus::Loaded));
    }

    #[test]
    fn unknown_appearance_falls_back_to_midnight_only() {
        let dir = temp_dir("unknown-appearance");
        let path = dir.join("settings.json");
        std::fs::write(&path, r#"{"version":2,"appearance":{"theme":"aurora"},"widgets":{"system":false}}"#).unwrap();
        let (s, status) = load_from(&path);
        assert_eq!(status, LoadStatus::Loaded);
        assert_eq!(s.appearance.theme, AppearanceMode::Midnight);
        assert!(!s.widgets.system);
    }

    #[test]
    fn oversized_file_is_rejected() {
        let dir = temp_dir("oversized");
        let path = dir.join("settings.json");
        std::fs::write(&path, vec![b' '; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert_eq!(load_from(&path).1, LoadStatus::Invalid);
    }
}
