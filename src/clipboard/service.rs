//! The clipboard worker: a thread that owns the database, reads the clipboard after a
//! change and answers the shelf's requests, so the UI thread never waits on the
//! clipboard or the disk.

use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use super::capture::{self, Policy};
use super::store::{ClipboardStore, Listing};
use super::{Filter, Retention, Skip, Snippet};
use crate::tasks::time::now_unix;

/// Apps often put several formats on the clipboard one after another; reading once
/// they are done avoids reading a half-written clipboard and repeated work.
const SETTLE: Duration = Duration::from_millis(150);
/// Old history is removed at least this often while Lilt runs.
const PRUNE_INTERVAL: Duration = Duration::from_secs(3600);
/// Entries sent to the shelf at a time.
pub const LIST_LIMIT: usize = 200;

#[derive(Clone, Debug)]
pub struct Config {
    pub policy: Policy,
    pub skip_secrets: bool,
    pub retention: Retention,
    pub max_items: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self { policy: Policy::default(), skip_secrets: true, retention: Retention::Days(30), max_items: 500 }
    }
}

#[derive(Debug)]
pub enum Command {
    Configure(Config),
    /// The clipboard changed (`WM_CLIPBOARDUPDATE`).
    Changed,
    List {
        search: String,
        filter: Filter,
    },
    Copy(i64),
    SetPinned(i64, bool),
    Delete(i64),
    /// Loads an entry for editing.
    Edit(i64),
    Save(Snippet),
    ClearHistory,
    DeleteAll,
}

#[derive(Debug)]
pub enum Update {
    Listing {
        listing: Listing,
        total: usize,
        history: usize,
        bytes: u64,
    },
    Copied(i64),
    CopyFailed,
    /// Copied text that was not kept, with the reason. Only reasons worth telling the
    /// user about are reported.
    Skipped(Skip),
    Editing(Snippet),
    Saved,
    Cleared(usize),
    /// The database could not be opened; the message names the problem, not any text.
    Unavailable(String),
}

enum Message {
    Command(Command),
    Shutdown,
}

pub struct ClipboardService {
    sender: Sender<Message>,
    worker: Option<JoinHandle<()>>,
}

impl ClipboardService {
    /// Starts the worker. `publish` is called on the worker thread.
    pub fn start(path: PathBuf, config: Config, publish: impl Fn(Update) + Send + 'static) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("lilt-clipboard".into())
            .spawn(move || run(&path, config, &receiver, &publish))
            .map_err(|err| tracing::error!(%err, "failed to start the clipboard worker"))
            .ok();
        Self { sender, worker }
    }

    pub fn send(&self, command: Command) {
        let _ = self.sender.send(Message::Command(command));
    }
}

impl Drop for ClipboardService {
    fn drop(&mut self) {
        let _ = self.sender.send(Message::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Worker<'a> {
    store: ClipboardStore,
    config: Config,
    publish: &'a dyn Fn(Update),
    search: String,
    filter: Filter,
    /// The clipboard sequence number last read or written by Lilt.
    seen: u32,
}

fn run(path: &std::path::Path, config: Config, receiver: &Receiver<Message>, publish: &dyn Fn(Update)) {
    let store = match ClipboardStore::open(path) {
        Ok(store) => store,
        Err(err) => {
            tracing::warn!(%err, "the clipboard shelf is unavailable");
            publish(Update::Unavailable(err.to_string()));
            while !matches!(receiver.recv(), Ok(Message::Shutdown) | Err(_)) {}
            return;
        }
    };
    let mut worker =
        Worker { store, config, publish, search: String::new(), filter: Filter::All, seen: capture::sequence() };
    worker.prune();
    worker.publish_listing();
    let mut changed_at: Option<Instant> = None;
    let mut next_prune = Instant::now() + PRUNE_INTERVAL;
    loop {
        let deadline = changed_at.map_or(next_prune, |at| (at + SETTLE).min(next_prune));
        let message = match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(message) => Some(message),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => return,
        };
        // Everything already queued is handled together, and the list is sent once.
        let mut batch: Vec<Message> = message.into_iter().collect();
        batch.extend(receiver.try_iter());
        let mut relist = false;
        for message in batch {
            match message {
                Message::Shutdown => return,
                Message::Command(Command::Changed) => {
                    changed_at.get_or_insert_with(Instant::now);
                }
                Message::Command(command) => relist |= worker.handle(command),
            }
        }
        if changed_at.is_some_and(|at| at.elapsed() >= SETTLE) {
            changed_at = None;
            relist |= worker.capture();
        }
        if Instant::now() >= next_prune {
            next_prune = Instant::now() + PRUNE_INTERVAL;
            relist |= worker.prune();
        }
        if relist {
            worker.publish_listing();
        }
    }
}

impl Worker<'_> {
    /// Handles a request; returns whether the list changed.
    fn handle(&mut self, command: Command) -> bool {
        let now = now_unix();
        let result = match command {
            Command::Changed => return false,
            Command::Configure(config) => {
                self.config = config;
                self.prune();
                return true;
            }
            Command::List { search, filter } => {
                self.search = search;
                self.filter = filter;
                return true;
            }
            Command::Copy(id) => {
                if let Ok(Some(image_bytes)) = self.store.image(id) {
                    match capture::write_image(&image_bytes) {
                        Some(sequence) => {
                            self.seen = sequence;
                            (self.publish)(Update::Copied(id));
                            self.store.touch(id, now)
                        }
                        None => {
                            (self.publish)(Update::CopyFailed);
                            return false;
                        }
                    }
                } else {
                    let text = self.store.text(id);
                    match text {
                        Ok(Some(text)) => match capture::write(&text) {
                            Some(sequence) => {
                                self.seen = sequence;
                                (self.publish)(Update::Copied(id));
                                self.store.touch(id, now)
                            }
                            None => {
                                (self.publish)(Update::CopyFailed);
                                return false;
                            }
                        },
                        Ok(None) => return true,
                        Err(err) => Err(err),
                    }
                }
            }
            Command::SetPinned(id, pinned) => self.store.set_pinned(id, pinned),
            Command::Delete(id) => self.store.delete(id),
            Command::Edit(id) => {
                match self.store.snippet(id) {
                    Ok(Some(snippet)) => (self.publish)(Update::Editing(snippet)),
                    Ok(None) => {}
                    Err(err) => tracing::warn!(%err, "could not load a clipboard entry"),
                }
                return false;
            }
            Command::Save(snippet) => self.store.save_snippet(&snippet, now).map(|_| (self.publish)(Update::Saved)),
            Command::ClearHistory => self.store.clear_history().map(|n| (self.publish)(Update::Cleared(n))),
            Command::DeleteAll => self.store.delete_all().map(|n| (self.publish)(Update::Cleared(n))),
        };
        if let Err(err) = result {
            tracing::warn!(%err, "clipboard shelf change failed");
        }
        true
    }

    /// Reads the clipboard after a change and remembers its text if it may be kept.
    fn capture(&mut self) -> bool {
        let sequence = capture::sequence();
        if sequence == self.seen {
            return false;
        }
        self.seen = sequence;
        let copied = capture::read(&self.config.policy).and_then(|copied| {
            if copied.image.is_none() && self.config.skip_secrets && super::looks_secret(&copied.text) {
                Err(Skip::LooksSecret)
            } else {
                Ok(copied)
            }
        });
        match copied {
            Ok(copied) => {
                if let Some(image) = copied.image {
                    if let Err(err) = self.store.capture_image(
                        &image.thumbnail_png,
                        &image.full_png,
                        copied.source.as_deref(),
                        now_unix(),
                    ) {
                        tracing::warn!(%err, "could not remember copied image");
                        return false;
                    }
                } else {
                    let kind = super::classify(&copied.text);
                    if let Err(err) = self.store.capture(&copied.text, kind, copied.source.as_deref(), now_unix()) {
                        tracing::warn!(%err, "could not remember copied text");
                        return false;
                    }
                }
                self.prune();
                true
            }
            Err(reason) => {
                // The reason only; never the text or the app.
                tracing::debug!(?reason, "clipboard change not kept");
                if matches!(reason, Skip::LooksSecret | Skip::TooLarge) {
                    (self.publish)(Update::Skipped(reason));
                }
                false
            }
        }
    }

    fn prune(&self) -> bool {
        match self.store.prune(self.config.retention, self.config.max_items, now_unix()) {
            Ok(removed) => removed > 0,
            Err(err) => {
                tracing::warn!(%err, "could not remove old clipboard history");
                false
            }
        }
    }

    fn publish_listing(&self) {
        let listing = match self.store.list(&self.search, self.filter, LIST_LIMIT) {
            Ok(listing) => listing,
            Err(err) => {
                tracing::warn!(%err, "could not list the clipboard shelf");
                return;
            }
        };
        let (total, history) = self.store.counts().unwrap_or_default();
        (self.publish)(Update::Listing { listing, total, history, bytes: self.store.disk_usage() });
    }
}
