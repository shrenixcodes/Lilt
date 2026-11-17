//! Idle clock/date carousel and weather for a manually selected place.
use std::io::Read;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, VecModel};

use super::{App, post};
use crate::settings::WeatherLocation;
use crate::ui::{AppState, Theme};

thread_local! {
    static WEATHER_LOCATION: std::cell::Cell<Option<WeatherLocation>> = const { std::cell::Cell::new(None) };
}

impl App {
    pub(super) fn connect_idle(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        app.island.global::<crate::ui::AppActions>().on_idle_changed(move |active| {
            if let Some(app) = weak.upgrade() {
                Self::idle_changed(&app, active);
            }
        });
        let state = app.island.global::<AppState>();
        let media = state.get_media();
        let media_mode = state.get_show_media()
            && (state.get_media_grace()
                || (media.availability == crate::ui::MediaAvailability::Active
                    && (media.playing || state.get_pointer_inside())));
        Self::idle_changed(app, state.get_on_screen() && !state.get_expanded() && !media_mode);
        Self::refresh_weather(app);
        let weak = Rc::downgrade(app);
        app.weather_timer.start(slint::TimerMode::Repeated, Duration::from_secs(20 * 60), move || {
            if let Some(app) = weak.upgrade() {
                Self::refresh_weather(&app);
            }
        });
    }

    fn idle_changed(app: &Rc<Self>, active: bool) {
        if app.idle_active.replace(active) != active {
            Self::restart_idle_cycle(app);
        }
    }

    pub(super) fn restart_idle_cycle(app: &Rc<Self>) {
        app.idle_timer.stop();
        app.idle_transition_timer.stop();
        let state = app.island.global::<AppState>();
        state.set_idle_index(0);
        state.set_idle_visible(true);
        Self::schedule_idle_item(app);
    }

    fn schedule_idle_item(app: &Rc<Self>) {
        let settings = app.settings.borrow().clock.clone();
        if !app.idle_active.get() || !settings.cycle_enabled {
            return;
        }
        let wait = idle_hold(&settings, app.island.global::<AppState>().get_idle_index());
        let weak = Rc::downgrade(app);
        app.idle_timer.start(slint::TimerMode::SingleShot, wait, move || {
            let Some(app) = weak.upgrade() else { return };
            if !app.idle_active.get() {
                return;
            }
            app.island.global::<AppState>().set_idle_visible(false);
            let weak = Rc::downgrade(&app);
            let delay = if app.island.global::<Theme>().get_motion() > 0.0 { 220 } else { 1 };
            app.idle_transition_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(delay), move || {
                let Some(app) = weak.upgrade() else { return };
                if !app.idle_active.get() {
                    return;
                }
                let state = app.island.global::<AppState>();
                let count = if state.get_weather().is_empty() { 2 } else { 3 };
                state.set_idle_index((state.get_idle_index() + 1) % count);
                state.set_idle_visible(true);
                Self::schedule_idle_item(&app);
            });
        });
    }

    pub(super) fn search_weather(app: &Rc<Self>, query: &str) {
        let Some(window) = app.settings_window.borrow().as_ref().map(ComponentHandle::clone_strong) else { return };
        let query = query.trim().to_owned();
        let request_id = window.get_weather_search_id().wrapping_add(1);
        window.set_weather_search_id(request_id);
        window.set_weather_results(ModelRc::default());
        if query.chars().count() < 2 || query.chars().count() > 100 || query.chars().any(char::is_control) {
            window.set_weather_message("Enter a city or place name between 2 and 100 characters.".into());
            window.set_weather_searching(false);
            return;
        }
        window.set_weather_searching(true);
        window.set_weather_message("".into());
        let weak_window = window.as_weak();
        let result = std::thread::Builder::new().name("lilt-place-search".into()).spawn(move || {
            let places = search_places(&query);
            let _ = weak_window.upgrade_in_event_loop(move |window| {
                if window.get_weather_search_id() != request_id {
                    return;
                }
                window.set_weather_searching(false);
                match places {
                    Some(places) => {
                        window.set_weather_message(
                            if places.is_empty() {
                                "No places found. Try a nearby city or add the country."
                            } else {
                                "Choose a place below."
                            }
                            .into(),
                        );
                        let rows: Vec<_> = places
                            .into_iter()
                            .map(|place| crate::ui::WeatherPlace {
                                name: place.name.into(),
                                latitude: place.location.latitude as f32,
                                longitude: place.location.longitude as f32,
                            })
                            .collect();
                        window.set_weather_results(ModelRc::new(VecModel::from(rows)));
                    }
                    None => window.set_weather_message(
                        "Place search is unavailable. Check your connection and try again.".into(),
                    ),
                }
            });
        });
        if let Err(error) = result {
            tracing::warn!(%error, "Could not start place search");
            window.set_weather_searching(false);
            window.set_weather_message("Place search could not start. Try again.".into());
        }
    }

    pub(super) fn select_weather(app: &Rc<Self>, index: i32) {
        let row = app.settings_window.borrow().as_ref().and_then(|window| {
            usize::try_from(index).ok().and_then(|index| window.get_weather_results().row_data(index))
        });
        let Some(row) = row else { return };
        let Some(location) = WeatherLocation::new(row.latitude as f64, row.longitude as f64) else { return };
        {
            let mut settings = app.settings.borrow_mut();
            settings.weather_location = Some(location);
            settings.weather_location_name = row.name.to_string();
        }
        Self::update_weather_location(app);
        Self::schedule_save(app);
        app.refresh_settings_window();
        if let Some(window) = app.settings_window.borrow().as_ref() {
            window.set_weather_results(ModelRc::default());
            window.set_weather_message("Location saved.".into());
        }
    }

    pub(super) fn update_weather_location(app: &Rc<Self>) {
        let location = app.settings.borrow().weather_location;
        if WEATHER_LOCATION.with(|last| last.get() != location) {
            Self::refresh_weather(app);
        }
    }

    fn refresh_weather(app: &Rc<Self>) {
        let location = app.settings.borrow().weather_location;
        WEATHER_LOCATION.with(|last| last.set(location));
        let state = app.island.global::<AppState>();
        state.set_weather("".into());
        if state.get_idle_index() == 2 {
            Self::restart_idle_cycle(app);
        }
        let Some(location) = location else { return };
        if let Err(error) = std::thread::Builder::new().name("lilt-weather".into()).spawn(move || {
            let weather = fetch_weather(location);
            if weather.is_none() {
                tracing::warn!("Weather unavailable; showing clock and date until the next refresh");
            }
            post(move |app| {
                if app.settings.borrow().weather_location == Some(location) {
                    app.island.global::<AppState>().set_weather(weather.unwrap_or_default().into());
                }
            });
        }) {
            tracing::warn!(%error, "Could not start weather refresh");
        }
    }
}

fn idle_hold(settings: &crate::settings::Clock, index: i32) -> Duration {
    Duration::from_secs(if index == 0 { settings.clock_seconds } else { settings.item_seconds }.clamp(3, 60) as u64)
}

#[derive(Debug, PartialEq)]
struct Place {
    name: String,
    location: WeatherLocation,
}

fn search_places(query: &str) -> Option<Vec<Place>> {
    let response = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(12))
        .build()
        .get("https://geocoding-api.open-meteo.com/v1/search")
        .query("name", query)
        .query("count", "8")
        .query("language", "en")
        .call()
        .ok()?;
    let mut bytes = Vec::new();
    response.into_reader().take(256 * 1024).read_to_end(&mut bytes).ok()?;
    parse_places(&serde_json::from_slice::<serde_json::Value>(&bytes).ok()?)
}

fn parse_places(json: &serde_json::Value) -> Option<Vec<Place>> {
    if json.get("error").and_then(serde_json::Value::as_bool).unwrap_or(false) {
        return None;
    }
    let Some(results) = json.get("results") else { return Some(Vec::new()) };
    Some(
        results
            .as_array()?
            .iter()
            .take(8)
            .filter_map(|place| {
                let location =
                    WeatherLocation::new(place.get("latitude")?.as_f64()?, place.get("longitude")?.as_f64()?)?;
                let mut parts: Vec<&str> = Vec::new();
                for field in ["name", "admin1", "country"] {
                    let text = place.get(field).and_then(serde_json::Value::as_str).unwrap_or("").trim();
                    if !text.is_empty()
                        && !parts.contains(&text)
                        && text.len() < 160
                        && !text.chars().any(char::is_control)
                    {
                        parts.push(text);
                    }
                }
                if parts.is_empty() {
                    return None;
                }
                Some(Place { name: parts.join(", "), location })
            })
            .collect(),
    )
}

fn fetch_weather(location: WeatherLocation) -> Option<String> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={}&longitude={}&current=temperature_2m,weather_code",
        location.latitude, location.longitude
    );
    let response = ureq::AgentBuilder::new().timeout(Duration::from_secs(12)).build().get(&url).call().ok()?;
    let mut bytes = Vec::new();
    response.into_reader().take(256 * 1024).read_to_end(&mut bytes).ok()?;
    weather_text(&serde_json::from_slice::<serde_json::Value>(&bytes).ok()?)
}

fn weather_text(json: &serde_json::Value) -> Option<String> {
    let current = json.get("current")?;
    let temperature = current.get("temperature_2m")?.as_f64()?;
    if !temperature.is_finite() || !(-100.0..=65.0).contains(&temperature) {
        return None;
    }
    let description = match current.get("weather_code")?.as_u64()? {
        0 => "Clear",
        1 => "Mostly clear",
        2 => "Partly cloudy",
        3 => "Overcast",
        45 | 48 => "Fog",
        51 | 53 | 55 | 56 | 57 => "Drizzle",
        61 | 63 | 65 | 66 | 67 => "Rain",
        71 | 73 | 75 | 77 => "Snow",
        80..=82 => "Rain showers",
        85 | 86 => "Snow showers",
        95 | 96 | 99 => "Thunderstorm",
        _ => return None,
    };
    Some(format!("{temperature:.0}\u{00b0}C \u{00b7} {description}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn clock_hold_starts_at_seven_seconds_and_item_timing_is_independent() {
        let mut settings = crate::settings::Clock::default();
        assert_eq!(idle_hold(&settings, 0), Duration::from_secs(7));
        assert_eq!(idle_hold(&settings, 1), Duration::from_secs(5));
        settings.clock_seconds = 12;
        settings.item_seconds = 4;
        assert_eq!(idle_hold(&settings, 0), Duration::from_secs(12));
        assert_eq!(idle_hold(&settings, 2), Duration::from_secs(4));
    }

    #[test]
    fn place_search_labels_distinguish_regions_and_ignore_bad_coordinates() {
        let places = parse_places(&json!({"results":[
            {"name":"Springfield","admin1":"Illinois","country":"United States","latitude":39.78,"longitude":-89.65},
            {"name":"Invalid","latitude":999,"longitude":0}
        ]}))
        .unwrap();
        assert_eq!(places.len(), 1);
        assert_eq!(places[0].name, "Springfield, Illinois, United States");
        assert_eq!(parse_places(&json!({})), Some(vec![]));
        assert_eq!(parse_places(&json!({"error":true})), None);
    }

    #[test]
    fn formats_current_weather_and_rejects_missing_or_invalid_readings() {
        assert_eq!(
            weather_text(&json!({"current":{"temperature_2m":24.2,"weather_code":0}})),
            Some("24\u{00b0}C \u{00b7} Clear".into())
        );
        assert_eq!(weather_text(&json!({"current":{}})), None);
        assert_eq!(weather_text(&json!({"current":{"temperature_2m":999,"weather_code":0}})), None);
        assert_eq!(weather_text(&json!({"current":{"temperature_2m":24,"weather_code":999}})), None);
    }
}
