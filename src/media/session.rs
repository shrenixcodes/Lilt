//! Thin, typed wrapper over the GSMTC session manager.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
use windows::Win32::System::SystemInformation::GetSystemTimePreciseAsFileTime;
use windows::core::{HSTRING, IUnknown, Interface, Result};

use super::{Command, Message};

/// Thumbnails larger than this are ignored; real album art is far smaller.
const MAX_THUMBNAIL_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlaybackStatus {
    Playing,
    Paused,
    Stopped,
    /// Opening, changing or closed.
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Capabilities {
    pub play_pause: bool,
    pub next: bool,
    pub previous: bool,
    pub seek: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timeline {
    /// Position at `sampled_at`.
    pub position: Duration,
    pub duration: Duration,
    pub sampled_at: Instant,
    /// 1.0 while playing at normal speed, 0.0 while paused.
    pub rate: f64,
}

impl Timeline {
    /// Position extrapolated to `now`, clamped to the track length.
    pub fn position_at(&self, now: Instant) -> Duration {
        let elapsed = now.saturating_duration_since(self.sampled_at).as_secs_f64() * self.rate;
        (self.position + Duration::from_secs_f64(elapsed.max(0.0))).min(self.duration)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MediaState {
    /// Application user model ID of the source, e.g. `Spotify.exe`.
    pub source_id: String,
    /// Human-friendly source name, e.g. `Spotify`.
    pub source_name: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub status: PlaybackStatus,
    pub capabilities: Capabilities,
    pub timeline: Option<Timeline>,
    pub session_count: usize,
}

/// Identifies the artwork of a track without reading the thumbnail stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArtworkKey {
    source: String,
    title: String,
    artist: String,
    album: String,
}

impl ArtworkKey {
    pub fn of(state: &MediaState) -> Self {
        Self {
            source: state.source_id.clone(),
            title: state.title.clone(),
            artist: state.artist.clone(),
            album: state.album.clone(),
        }
    }
}

struct Properties {
    title: String,
    artist: String,
    album: String,
    thumbnail: Option<IRandomAccessStreamReference>,
}

struct Subscription {
    session: Session,
    source_id: String,
    identity: usize,
    tokens: [i64; 3],
}

/// Playback-state subscription on a session that is not necessarily followed, so that
/// another application starting playback is noticed.
struct Watch {
    session: Session,
    identity: usize,
    token: i64,
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.session.RemovePlaybackInfoChanged(self.token);
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let [properties, playback, timeline] = self.tokens;
        let _ = self.session.RemoveMediaPropertiesChanged(properties);
        let _ = self.session.RemovePlaybackInfoChanged(playback);
        let _ = self.session.RemoveTimelinePropertiesChanged(timeline);
    }
}

pub struct Tracker {
    manager: Manager,
    events: Sender<Message>,
    manager_tokens: [i64; 2],
    current: Option<Subscription>,
    properties: Option<Properties>,
    /// Source chosen explicitly by the user; overrides automatic selection while it exists.
    pinned: Option<String>,
    /// Specific COM session identity chosen by [`Command::CycleSession`] (distinguishes browser tabs).
    pinned_identity: Option<usize>,
    /// Sessions that were playing at the last selection, to detect sessions that start.
    playing: Vec<String>,
    watches: Vec<Watch>,
    session_count: usize,
}

fn notify(events: &Sender<Message>, make: fn() -> Message) -> impl Fn() + Send + 'static {
    let events = events.clone();
    move || {
        let _ = events.send(make());
    }
}

impl Tracker {
    pub fn new(events: Sender<Message>) -> Result<Self> {
        let manager = Manager::RequestAsync()?.join()?;
        let sessions_changed = notify(&events, || Message::SessionsChanged);
        let current_changed = notify(&events, || Message::SessionsChanged);
        let manager_tokens = [
            manager.SessionsChanged(&TypedEventHandler::new(move |_, _| {
                sessions_changed();
                Ok(())
            }))?,
            manager.CurrentSessionChanged(&TypedEventHandler::new(move |_, _| {
                current_changed();
                Ok(())
            }))?,
        ];
        Ok(Self {
            manager,
            events,
            manager_tokens,
            current: None,
            properties: None,
            pinned: None,
            pinned_identity: None,
            playing: Vec::new(),
            watches: Vec::new(),
            session_count: 0,
        })
    }

    fn sessions(&self) -> Vec<Session> {
        self.manager.GetSessions().map(|list| list.into_iter().collect()).unwrap_or_default()
    }

    /// Chooses the session to follow. Returns true when the selection changed.
    ///
    /// The followed session is kept for as long as it exists. Lilt moves to another
    /// session only when that one starts playing (a fresh user action) or when the user
    /// picks a source. It never moves because the followed session was paused: the next
    /// click on the same button would then control a different application.
    pub fn select_session(&mut self) -> bool {
        let sessions = self.sessions();
        self.session_count = sessions.len();
        let ids: Vec<String> = sessions.iter().map(source_id).collect();
        let identities: Vec<usize> = sessions.iter().map(identity).collect();
        let statuses: Vec<PlaybackStatus> = sessions.iter().map(status).collect();
        self.watch(&sessions, &identities);
        let playing: Vec<String> = ids
            .iter()
            .zip(&statuses)
            .filter(|(_, status)| **status == PlaybackStatus::Playing)
            .map(|(id, _)| id.clone())
            .collect();

        if self.pinned.as_ref().is_some_and(|p| !ids.contains(p)) {
            self.pinned = None;
        }
        if self.pinned_identity.is_some_and(|id| !identities.contains(&id)) {
            self.pinned_identity = None;
        }
        let system_current = self.manager.GetCurrentSession().ok();
        let chosen_id = choose(&Selection {
            ids: &ids,
            playing: &playing,
            previously_playing: &self.playing,
            followed: self.current.as_ref().map(|c| c.source_id.as_str()),
            pinned: self.pinned.as_deref(),
            system_current: system_current.as_ref().map(source_id).as_deref(),
        });
        self.playing = playing;
        // Several sessions can share the chosen source id: browsers publish one per tab.
        let candidates: Vec<Candidate> = (0..sessions.len())
            .filter(|&i| Some(&ids[i]) == chosen_id.as_ref())
            .map(|i| Candidate { identity: identities[i], playing: statuses[i] == PlaybackStatus::Playing })
            .collect();
        let chosen_identity =
            self.pinned_identity.filter(|id| candidates.iter().any(|c| c.identity == *id)).or_else(|| {
                pick_session(
                    &candidates,
                    self.current.as_ref().map(|c| c.identity),
                    system_current.as_ref().map(identity),
                )
            });
        let chosen = chosen_identity.and_then(|wanted| identities.iter().position(|&i| i == wanted));

        if self.current.as_ref().map(|c| c.identity) == chosen_identity {
            return false;
        }
        self.current = None;
        self.properties = None;
        tracing::debug!(source = chosen_id.as_deref().unwrap_or("none"), "following media session");
        if let Some(i) = chosen {
            match self.subscribe(sessions[i].clone(), ids[i].clone()) {
                Ok(subscription) => self.current = Some(subscription),
                Err(err) => tracing::warn!(%err, "could not subscribe to media session"),
            }
        }
        true
    }

    /// Keeps one playback subscription per existing session.
    fn watch(&mut self, sessions: &[Session], identities: &[usize]) {
        self.watches.retain(|w| identities.contains(&w.identity));
        for (session, &id) in sessions.iter().zip(identities) {
            if self.watches.iter().any(|w| w.identity == id) {
                continue;
            }
            let changed = notify(&self.events, || Message::SessionsChanged);
            let handler = TypedEventHandler::new(move |_, _| {
                changed();
                Ok(())
            });
            match session.PlaybackInfoChanged(&handler) {
                Ok(token) => self.watches.push(Watch { session: session.clone(), identity: id, token }),
                Err(err) => tracing::debug!(%err, "could not watch media session"),
            }
        }
    }

    fn subscribe(&self, session: Session, source_id: String) -> Result<Subscription> {
        let properties = notify(&self.events, || Message::PropertiesChanged);
        let playback = notify(&self.events, || Message::PlaybackChanged);
        let timeline = notify(&self.events, || Message::TimelineChanged);
        let tokens = [
            session.MediaPropertiesChanged(&TypedEventHandler::new(move |_, _| {
                properties();
                Ok(())
            }))?,
            session.PlaybackInfoChanged(&TypedEventHandler::new(move |_, _| {
                playback();
                Ok(())
            }))?,
            session.TimelinePropertiesChanged(&TypedEventHandler::new(move |_, _| {
                timeline();
                Ok(())
            }))?,
        ];
        Ok(Subscription { identity: identity(&session), session, source_id, tokens })
    }

    /// Reads the current state. Media properties are only re-read when `refresh` is set
    /// because they require an asynchronous round trip to the source application.
    pub fn read_state(&mut self, refresh: bool) -> Option<MediaState> {
        let current = self.current.as_ref()?;
        let session = &current.session;
        if refresh || self.properties.is_none() {
            match read_properties(session) {
                Ok(p) => self.properties = Some(p),
                Err(err) => tracing::debug!(%err, "media properties unavailable"),
            }
        }
        let properties = self.properties.as_ref()?;
        let info = session.GetPlaybackInfo().ok();
        let controls = info.as_ref().and_then(|i| i.Controls().ok());
        let flag = |f: fn(
            &windows::Media::Control::GlobalSystemMediaTransportControlsSessionPlaybackControls,
        ) -> Result<bool>| { controls.as_ref().and_then(|c| f(c).ok()).unwrap_or(false) };
        let capabilities = Capabilities {
            play_pause: flag(|c| c.IsPlayPauseToggleEnabled())
                || flag(|c| c.IsPlayEnabled())
                || flag(|c| c.IsPauseEnabled()),
            next: flag(|c| c.IsNextEnabled()),
            previous: flag(|c| c.IsPreviousEnabled()),
            seek: flag(|c| c.IsPlaybackPositionEnabled()),
        };
        let status = status(session);
        let rate = match status {
            PlaybackStatus::Playing => {
                info.and_then(|i| i.PlaybackRate().ok()).and_then(|r| r.Value().ok()).unwrap_or(1.0)
            }
            _ => 0.0,
        };

        Some(MediaState {
            source_name: friendly_source_name(&current.source_id),
            source_id: current.source_id.clone(),
            title: properties.title.clone(),
            artist: properties.artist.clone(),
            album: properties.album.clone(),
            status,
            capabilities,
            timeline: read_timeline(session, rate),
            session_count: self.session_count,
        })
    }

    /// Encoded thumbnail bytes of the current track, if the source provides one.
    pub fn read_thumbnail(&self) -> Option<Vec<u8>> {
        let reference = self.properties.as_ref()?.thumbnail.as_ref()?;
        match read_stream(reference) {
            Ok(bytes) => Some(bytes),
            Err(err) => {
                tracing::debug!(%err, "thumbnail unavailable");
                None
            }
        }
    }

    pub fn execute(&mut self, command: Command) -> Result<()> {
        if let Command::CycleSession = command {
            let sessions = self.sessions();
            let ids: Vec<String> = sessions.iter().map(source_id).collect();
            let identities: Vec<usize> = sessions.iter().map(identity).collect();
            let target = cycle_target(&identities, self.current.as_ref().map(|c| c.identity));
            self.pinned_identity = target;
            self.pinned = target
                .and_then(|wanted| identities.iter().position(|&id| id == wanted))
                .and_then(|i| ids.get(i).cloned());
            return Ok(());
        }
        let Some(current) = &self.current else { return Ok(()) };
        let session = &current.session;
        let accepted = match command {
            Command::TogglePlayPause => session.TryTogglePlayPauseAsync()?.join()?,
            Command::Next => session.TrySkipNextAsync()?.join()?,
            Command::Previous => session.TrySkipPreviousAsync()?.join()?,
            Command::Seek(position) => {
                session.TryChangePlaybackPositionAsync((position.as_nanos() / 100) as i64)?.join()?
            }
            Command::CycleSession => unreachable!("handled above"),
        };
        if !accepted {
            tracing::debug!(?command, "media source declined the command");
        }
        Ok(())
    }
}

impl Drop for Tracker {
    fn drop(&mut self) {
        self.current = None;
        self.watches.clear();
        let [sessions, current] = self.manager_tokens;
        let _ = self.manager.RemoveSessionsChanged(sessions);
        let _ = self.manager.RemoveCurrentSessionChanged(current);
    }
}

/// Inputs to [`choose`], kept free of WinRT types so the policy can be tested.
struct Selection<'a> {
    ids: &'a [String],
    playing: &'a [String],
    previously_playing: &'a [String],
    followed: Option<&'a str>,
    pinned: Option<&'a str>,
    system_current: Option<&'a str>,
}

/// Session selection policy: an explicit choice, then a session that just started
/// playing, then the session already followed, then the one Windows considers current.
fn choose(s: &Selection) -> Option<String> {
    let exists = |id: &str| s.ids.iter().any(|candidate| candidate == id);
    let newly_playing: Vec<&String> = s.playing.iter().filter(|id| !s.previously_playing.contains(id)).collect();
    let started = newly_playing
        .iter()
        .find(|id| s.system_current == Some(id.as_str()))
        .or(newly_playing.first())
        .map(|id| id.as_str());
    s.pinned
        .filter(|id| exists(id))
        .or(started)
        .or(s.followed.filter(|id| exists(id)))
        .or(s.system_current.filter(|id| exists(id)))
        .or(s.ids.first().map(String::as_str))
        .map(str::to_owned)
}

/// One of the sessions sharing the chosen source id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Candidate {
    identity: usize,
    playing: bool,
}

/// Which of the sessions sharing a source id to follow: the followed one while it plays,
/// then one that plays (Windows' current session first), then the followed one, then
/// Windows' current one, then the first.
fn pick_session(candidates: &[Candidate], followed: Option<usize>, system_current: Option<usize>) -> Option<usize> {
    let find = |wanted: Option<usize>, playing: bool| {
        candidates.iter().find(|c| Some(c.identity) == wanted && (!playing || c.playing)).map(|c| c.identity)
    };
    find(followed, true)
        .or(find(system_current, true))
        .or(candidates.iter().find(|c| c.playing).map(|c| c.identity))
        .or(find(followed, false))
        .or(find(system_current, false))
        .or(candidates.first().map(|c| c.identity))
}

fn cycle_target(identities: &[usize], current: Option<usize>) -> Option<usize> {
    if identities.is_empty() {
        return None;
    }
    let next = current
        .and_then(|id| identities.iter().position(|&candidate| candidate == id))
        .map_or(0, |i| (i + 1) % identities.len());
    identities.get(next).copied()
}

/// A session's COM identity. Browsers publish one session per tab under the same source
/// id and replace them as tabs start and stop playing, so the id alone does not tell
/// sessions apart.
fn identity(session: &Session) -> usize {
    session.cast::<IUnknown>().map_or(0, |unknown| unknown.as_raw() as usize)
}

fn source_id(session: &Session) -> String {
    session.SourceAppUserModelId().map(|s| s.to_string()).unwrap_or_default()
}

fn status(session: &Session) -> PlaybackStatus {
    match session.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()) {
        Ok(Status::Playing) => PlaybackStatus::Playing,
        Ok(Status::Paused) => PlaybackStatus::Paused,
        Ok(Status::Stopped) => PlaybackStatus::Stopped,
        _ => PlaybackStatus::Other,
    }
}

fn read_properties(session: &Session) -> Result<Properties> {
    let p = session.TryGetMediaPropertiesAsync()?.join()?;
    let text = |r: Result<HSTRING>| r.map(|s| s.to_string().trim().to_owned()).unwrap_or_default();
    let artist = text(p.Artist());
    Ok(Properties {
        title: text(p.Title()),
        artist: if artist.is_empty() { text(p.AlbumArtist()) } else { artist },
        album: text(p.AlbumTitle()),
        thumbnail: p.Thumbnail().ok(),
    })
}

/// Current time as a Windows FILETIME tick count (100 ns since 1601).
fn now_ticks() -> i64 {
    let ft = unsafe { GetSystemTimePreciseAsFileTime() };
    ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64
}

fn read_timeline(session: &Session, rate: f64) -> Option<Timeline> {
    let t = session.GetTimelineProperties().ok()?;
    let ticks = |v: i64| Duration::from_nanos(v.max(0) as u64 * 100);
    let start = t.StartTime().ok()?.Duration;
    let end = t.EndTime().ok()?.Duration;
    if end <= start {
        return None;
    }
    let duration = ticks(end - start);
    let mut position = ticks(t.Position().ok()?.Duration - start);
    // Players update the timeline only occasionally; account for the time since then.
    if let Ok(updated) = t.LastUpdatedTime() {
        let age = now_ticks() - updated.UniversalTime;
        if rate > 0.0 && (0..10_000 * 60 * 60 * 1000).contains(&age) {
            position += Duration::from_secs_f64(ticks(age).as_secs_f64() * rate);
        }
    }
    Some(Timeline { position: position.min(duration), duration, sampled_at: Instant::now(), rate })
}

fn read_stream(reference: &IRandomAccessStreamReference) -> Result<Vec<u8>> {
    let stream = reference.OpenReadAsync()?.join()?;
    let size = stream.Size()?;
    if size == 0 || size > MAX_THUMBNAIL_BYTES {
        return Err(windows::core::Error::from(windows::Win32::Foundation::E_UNEXPECTED));
    }
    let reader = DataReader::CreateDataReader(&stream)?;
    let loaded = reader.LoadAsync(size as u32)?.join()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes)?;
    Ok(bytes)
}

/// Derives a readable application name from an application user model ID.
pub fn friendly_source_name(id: &str) -> String {
    const KNOWN: &[(&str, &str)] = &[
        ("spotify", "Spotify"),
        ("msedge", "Microsoft Edge"),
        ("chrome", "Google Chrome"),
        ("308046b0af4a39cb", "Firefox"),
        ("firefox", "Firefox"),
        ("zunemusic", "Media Player"),
        ("zunevideo", "Movies & TV"),
        ("vlc", "VLC"),
        ("brave", "Brave"),
        ("opera", "Opera"),
        ("applemusic", "Apple Music"),
        ("itunes", "iTunes"),
        ("tidal", "TIDAL"),
        ("deezer", "Deezer"),
        ("foobar2000", "foobar2000"),
        ("musicbee", "MusicBee"),
    ];
    let lower = id.to_ascii_lowercase();
    if let Some((_, name)) = KNOWN.iter().find(|(needle, _)| lower.contains(needle)) {
        return (*name).to_owned();
    }
    // Packaged apps: "Publisher.App_hash!Entry" -> "App"; desktop apps: "C:\...\app.exe" -> "app".
    let base = id.rsplit(['\\', '/']).next().unwrap_or(id);
    let base = base.split('!').next().unwrap_or(base);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.strip_suffix(".exe").or_else(|| base.strip_suffix(".EXE")).unwrap_or(base);
    let base = base.rsplit('.').next().unwrap_or(base);
    if base.is_empty() { "Media".to_owned() } else { base.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn friendly_names() {
        assert_eq!(friendly_source_name("Spotify.exe"), "Spotify");
        assert_eq!(friendly_source_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"), "Spotify");
        assert_eq!(friendly_source_name("MSEdge"), "Microsoft Edge");
        assert_eq!(friendly_source_name("308046B0AF4A39CB"), "Firefox");
        assert_eq!(friendly_source_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"), "Media Player");
        assert_eq!(friendly_source_name(r"C:\Program Files\Foo\tunes.exe"), "tunes");
        assert_eq!(friendly_source_name("Contoso.Player_abc!App"), "Player");
        assert_eq!(friendly_source_name(""), "Media");
    }

    #[test]
    fn among_tabs_of_one_browser_the_playing_one_is_followed() {
        let tab = |identity, playing| Candidate { identity, playing };
        // Followed tab paused, another tab playing: switch to the playing tab.
        assert_eq!(pick_session(&[tab(1, false), tab(2, true)], Some(1), None), Some(2));
        // The followed tab plays: stay, even if another tab plays too.
        assert_eq!(pick_session(&[tab(1, true), tab(2, true)], Some(1), Some(2)), Some(1));
        // Several play: Windows' current one first.
        assert_eq!(pick_session(&[tab(1, true), tab(2, true)], None, Some(2)), Some(2));
        // None plays: stay with the followed tab, then Windows' current, then the first.
        assert_eq!(pick_session(&[tab(1, false), tab(2, false)], Some(2), Some(1)), Some(2));
        assert_eq!(pick_session(&[tab(1, false), tab(2, false)], Some(9), Some(2)), Some(2));
        assert_eq!(pick_session(&[tab(1, false), tab(2, false)], None, None), Some(1));
        // The followed session was replaced: its identity is gone, so the new one is used.
        assert_eq!(pick_session(&[tab(3, true)], Some(1), None), Some(3));
        assert_eq!(pick_session(&[], Some(1), None), None);
    }

    #[test]
    fn cycle_target_switches_between_distinct_identities_including_same_source() {
        let tabs = [101usize, 102, 203];
        assert_eq!(cycle_target(&tabs, Some(101)), Some(102));
        assert_eq!(cycle_target(&tabs, Some(102)), Some(203));
        assert_eq!(cycle_target(&tabs, Some(203)), Some(101));
        assert_eq!(cycle_target(&tabs, Some(999)), Some(101));
        assert_eq!(cycle_target(&[], Some(101)), None);
    }

    /// The same session keeps its identity across lookups, so the followed session is not
    /// re-subscribed on every change. Checked against whatever sessions this PC has.
    #[test]
    fn session_identity_is_stable() {
        use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
        let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let manager = match Manager::RequestAsync().and_then(|m| m.join()) {
            Ok(m) => m,
            Err(_) => return,
        };
        let list = || manager.GetSessions().map(|l| l.into_iter().collect::<Vec<Session>>()).unwrap_or_default();
        let (first, second) = (list(), list());
        assert_eq!(first.len(), second.len());
        for (a, b) in first.iter().zip(&second) {
            assert_eq!(identity(a), identity(b), "{}", source_id(a));
            assert_ne!(identity(a), 0);
        }
    }

    fn ids(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn pick(
        all: &[&str],
        playing: &[&str],
        before: &[&str],
        followed: Option<&str>,
        current: Option<&str>,
    ) -> Option<String> {
        choose(&Selection {
            ids: &ids(all),
            playing: &ids(playing),
            previously_playing: &ids(before),
            followed,
            pinned: None,
            system_current: current,
        })
    }

    #[test]
    fn pausing_does_not_switch_to_another_playing_app() {
        // B was already playing when A, the followed session, is paused.
        assert_eq!(pick(&["A", "B"], &["B"], &["A", "B"], Some("A"), Some("B")).as_deref(), Some("A"));
    }

    #[test]
    fn a_session_that_starts_playing_takes_over() {
        assert_eq!(pick(&["A", "B"], &["A", "B"], &["A"], Some("A"), Some("A")).as_deref(), Some("B"));
    }

    #[test]
    fn falls_back_when_the_followed_session_closes() {
        assert_eq!(pick(&["B", "C"], &[], &[], Some("A"), Some("C")).as_deref(), Some("C"));
        assert_eq!(pick(&["B"], &[], &[], Some("A"), None).as_deref(), Some("B"));
        assert_eq!(pick(&[], &[], &[], Some("A"), None), None);
    }

    #[test]
    fn explicit_choice_wins_while_it_exists() {
        let all = ids(&["A", "B"]);
        let selection = |pinned| Selection {
            ids: &all,
            playing: &[],
            previously_playing: &[],
            followed: Some("A"),
            pinned,
            system_current: None,
        };
        assert_eq!(choose(&selection(Some("B"))).as_deref(), Some("B"));
        assert_eq!(choose(&selection(Some("Z"))).as_deref(), Some("A"));
    }

    #[test]
    fn timeline_extrapolates_and_clamps() {
        let start = Instant::now();
        let t = Timeline {
            position: Duration::from_secs(10),
            duration: Duration::from_secs(12),
            sampled_at: start,
            rate: 1.0,
        };
        assert_eq!(t.position_at(start + Duration::from_secs(1)), Duration::from_secs(11));
        assert_eq!(t.position_at(start + Duration::from_secs(5)), Duration::from_secs(12));
        let paused = Timeline { rate: 0.0, ..t };
        assert_eq!(paused.position_at(start + Duration::from_secs(5)), Duration::from_secs(10));
    }
}
