//! Windows media session integration via the Global System Media Transport Controls
//! (GSMTC) API.
//!
//! A single worker thread owns the WinRT session manager. WinRT raises change events
//! on thread-pool threads; their handlers only post a message to the worker, which
//! reads the new state, compares it with what was last published and forwards
//! changes. Nothing is polled: between events the worker blocks on its channel.

mod session;

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::Duration;

use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

use crate::artwork::{Artwork, ArtworkCache};

pub use session::{MediaState, PlaybackStatus};

/// Media applications often raise several change events for a single track change
/// (title first, thumbnail shortly after). Events arriving within this window are
/// handled together.
const SETTLE: Duration = Duration::from_millis(150);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    TogglePlayPause,
    Next,
    Previous,
    /// Seek to an absolute position.
    Seek(Duration),
    /// Switch to the next available media session.
    CycleSession,
}

/// Updates published by the media worker.
#[derive(Debug)]
pub enum MediaUpdate {
    /// The media session API could not be initialised on this system.
    Unavailable,
    /// No media session exists.
    Idle,
    State(MediaState),
    /// Artwork for the current track; `None` when the source provides none.
    Artwork(Option<Artwork>),
}

pub(crate) enum Message {
    SessionsChanged,
    PropertiesChanged,
    PlaybackChanged,
    TimelineChanged,
    Command(Command),
    Shutdown,
}

pub struct MediaService {
    sender: Sender<Message>,
    worker: Option<JoinHandle<()>>,
}

impl MediaService {
    /// Starts the worker. `publish` is called from the worker thread.
    pub fn start(publish: impl Fn(MediaUpdate) + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let events = sender.clone();
        let worker = std::thread::Builder::new()
            .name("lilt-media".into())
            .spawn(move || run(receiver, events, publish))
            .map_err(|err| tracing::error!(%err, "failed to start media worker"))
            .ok();
        Self { sender, worker }
    }

    pub fn send(&self, command: Command) {
        let _ = self.sender.send(Message::Command(command));
    }
}

impl Drop for MediaService {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Default)]
struct Pending {
    sessions: bool,
    properties: bool,
    playback: bool,
    timeline: bool,
    commands: Vec<Command>,
    shutdown: bool,
}

impl Pending {
    fn add(&mut self, message: Message) {
        match message {
            Message::SessionsChanged => self.sessions = true,
            Message::PropertiesChanged => self.properties = true,
            Message::PlaybackChanged => self.playback = true,
            Message::TimelineChanged => self.timeline = true,
            Message::Command(c) => self.commands.push(c),
            Message::Shutdown => self.shutdown = true,
        }
    }
}

fn run(receiver: Receiver<Message>, events: Sender<Message>, publish: impl Fn(MediaUpdate)) {
    // WinRT objects on this thread live in the multithreaded apartment, so their event
    // handlers may be invoked from any thread pool thread.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    work(&receiver, events, &publish);
    if com.is_ok() {
        unsafe { CoUninitialize() };
    }
}

fn work(receiver: &Receiver<Message>, events: Sender<Message>, publish: &impl Fn(MediaUpdate)) {
    let mut tracker = match session::Tracker::new(events) {
        Ok(tracker) => tracker,
        Err(err) => {
            tracing::warn!(%err, "media sessions are unavailable");
            publish(MediaUpdate::Unavailable);
            // Keep draining so commands from the UI never block, until shutdown.
            while !matches!(receiver.recv(), Ok(Message::Shutdown) | Err(_)) {}
            return;
        }
    };
    let mut artwork = ArtworkCache::default();
    let mut last_state: Option<MediaState> = None;
    // Track whose artwork was last published, and whether it had any.
    let mut last_artwork: Option<(session::ArtworkKey, bool)> = None;
    let mut idle_published = false;

    let mut pending = Pending { sessions: true, ..Pending::default() };
    loop {
        if pending.shutdown {
            break;
        }

        for command in pending.commands.drain(..) {
            if matches!(command, Command::CycleSession) {
                pending.sessions = true;
            }
            if let Err(err) = tracker.execute(command) {
                tracing::debug!(%err, ?command, "media command failed");
            }
        }
        if pending.sessions && tracker.select_session() {
            pending.properties = true;
        }

        match tracker.read_state(pending.properties) {
            None => {
                if !idle_published {
                    last_state = None;
                    last_artwork = None;
                    idle_published = true;
                    publish(MediaUpdate::Idle);
                }
            }
            Some(state) => {
                idle_published = false;
                let key = session::ArtworkKey::of(&state);
                let new_track = last_artwork.as_ref().is_none_or(|(k, _)| *k != key);
                // Some players attach artwork a moment after the title, so a track without
                // artwork is retried when its properties change again.
                let retry = !new_track && pending.properties && last_artwork.as_ref().is_some_and(|(_, found)| !found);
                if new_track || retry {
                    let image = tracker.read_thumbnail().and_then(|bytes| artwork.process(&bytes));
                    let found = image.is_some();
                    if new_track || found {
                        publish(MediaUpdate::Artwork(image));
                    }
                    last_artwork = Some((key, found));
                }
                if last_state.as_ref() != Some(&state) {
                    last_state = Some(state.clone());
                    publish(MediaUpdate::State(state));
                }
            }
        }

        pending = Pending::default();
        let first = match receiver.recv() {
            Ok(message) => message,
            Err(_) => break,
        };
        let is_command = matches!(first, Message::Command(_) | Message::Shutdown);
        pending.add(first);
        // Commands run immediately; change notifications are coalesced so that a burst
        // of related events results in a single read of the session.
        if !is_command {
            loop {
                match receiver.recv_timeout(SETTLE) {
                    Ok(message) => pending.add(message),
                    Err(RecvTimeoutError::Timeout) => break,
                    Err(RecvTimeoutError::Disconnected) => {
                        pending.shutdown = true;
                        break;
                    }
                }
            }
        }
    }
}
