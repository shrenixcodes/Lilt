//! The clipboard shelf: connects the clipboard worker to the UI and the settings.
//!
//! Nothing runs until it is needed: the worker and its database start the first time
//! the shelf is shown or capture is turned on, and Windows is asked for clipboard
//! notifications only while capture is on and not paused.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use super::{App, post};
use crate::clipboard::service::{ClipboardService, Command, Config, Update};
use crate::clipboard::store::Listing;
use crate::clipboard::{self, Filter, Skip, Snippet, Summary, capture};
use crate::tasks::time::now_unix;
use crate::ui::{AppState, ClipFilter, ClipRow, ClipRowKind, ClipboardActions, ClipboardState, SidePanel, SnippetForm};
use crate::window;

/// Row heights of the list, as in ui/clipboard.slint (`ClipMetrics`).
const HEADER_HEIGHT: f32 = 26.0;
const ENTRY_HEIGHT: f32 = 56.0;
const COPIED_TIME: Duration = Duration::from_millis(1600);
const NOTICE_TIME: Duration = Duration::from_secs(4);

pub(crate) struct ClipboardController {
    path: PathBuf,
    service: RefCell<Option<ClipboardService>>,
    rows: Rc<VecModel<ClipRow>>,
    /// The id of the entry chosen with the keyboard.
    selected: Cell<Option<i64>>,
    /// Copy requested with Enter: collapse the island once it is on the clipboard.
    close_after_copy: Cell<Option<i64>>,
    listening: Cell<bool>,
    /// Capture paused until resumed or Lilt restarts.
    paused: Cell<bool>,
    config: RefCell<Option<Config>>,
    counts: Cell<(usize, usize)>,
    /// A search or snippet field is being edited.
    pub busy: Cell<bool>,
    copied_timer: slint::Timer,
    notice_timer: slint::Timer,
}

impl ClipboardController {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            service: RefCell::new(None),
            rows: Rc::new(VecModel::default()),
            selected: Cell::new(None),
            close_after_copy: Cell::new(None),
            listening: Cell::new(false),
            paused: Cell::new(false),
            config: RefCell::new(None),
            counts: Cell::new((0, 0)),
            busy: Cell::new(false),
            copied_timer: slint::Timer::default(),
            notice_timer: slint::Timer::default(),
        }
    }

    fn send(&self, command: Command) {
        if let Some(service) = self.service.borrow().as_ref() {
            service.send(command);
        }
    }

    /// Stops listening and the worker, for shutdown.
    pub fn stop(&self) {
        if self.listening.replace(false) {
            capture::stop_listening(window::control_window());
        }
        self.service.replace(None);
    }
}

fn filter(state: &ClipboardState) -> Filter {
    match state.get_filter() {
        ClipFilter::Pinned => Filter::Pinned,
        ClipFilter::Recent => Filter::Recent,
        ClipFilter::Snippets => Filter::Manual,
        ClipFilter::Links => Filter::Urls,
        ClipFilter::Code => Filter::Code,
        ClipFilter::Other => Filter::Other,
        _ => Filter::All,
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The list's second line: where and when it was copied, and what it is.
fn meta(entry: &Summary, now: i64) -> String {
    let when = clipboard::relative_time(entry.used_at, now);
    let mut parts: Vec<String> = Vec::new();
    if entry.manual {
        if !entry.description.is_empty() {
            parts.push(entry.description.clone());
        } else {
            parts.push("Snippet".into());
        }
    } else {
        // Only what Windows reported; an unknown source stays unknown.
        parts.push(entry.source.as_deref().map_or_else(|| "Unknown app".into(), clipboard::app_name));
        parts.push(when);
    }
    if entry.kind != clipboard::Kind::Text && entry.kind != clipboard::Kind::Multiline {
        parts.push(entry.kind.label().into());
    }
    if entry.lines > 2 {
        parts.push(format!("{} lines", entry.lines));
    }
    parts.join(" · ")
}

fn entry_row(entry: &Summary, now: i64) -> ClipRow {
    let preview = if entry.title.is_empty() {
        entry.preview.clone()
    } else {
        entry.preview.lines().next().unwrap_or_default().to_owned()
    };
    ClipRow {
        kind: ClipRowKind::Entry,
        id: entry.id as i32,
        preview: preview.into(),
        title: entry.title.as_str().into(),
        meta: meta(entry, now).into(),
        technical: entry.kind.is_technical(),
        pinned: entry.pinned,
        manual: entry.manual,
        count: 0,
    }
}

fn header(title: &str, count: usize) -> ClipRow {
    ClipRow { kind: ClipRowKind::Header, title: title.into(), count: count as i32, ..Default::default() }
}

/// The rows of the list. Everything, unsearched, is grouped into pinned entries,
/// snippets and copied text; a filter or search shows one list.
pub fn build_rows(entries: &[Summary], grouped: bool, now: i64) -> Vec<ClipRow> {
    if !grouped {
        return entries.iter().map(|e| entry_row(e, now)).collect();
    }
    let mut rows = Vec::with_capacity(entries.len() + 3);
    // Pinned entries first, then snippets, then copied text.
    let section = |e: &Summary| {
        if e.pinned {
            0
        } else if e.manual {
            1
        } else {
            2
        }
    };
    for (index, title) in ["Pinned", "Snippets", "Recent"].into_iter().enumerate() {
        let members: Vec<&Summary> = entries.iter().filter(|e| section(e) == index).collect();
        if !members.is_empty() {
            rows.push(header(title, members.len()));
            rows.extend(members.into_iter().map(|e| entry_row(e, now)));
        }
    }
    rows
}

/// The y of row `index` in the list.
fn row_top(rows: &[ClipRow], index: usize) -> f32 {
    rows.iter().take(index).map(|r| if r.kind == ClipRowKind::Header { HEADER_HEIGHT } else { ENTRY_HEIGHT }).sum()
}

/// The entry `delta` entries away from `from` (or the first entry), skipping headers.
fn step(rows: &[ClipRow], from: Option<usize>, delta: i32) -> Option<usize> {
    let entries: Vec<usize> =
        rows.iter().enumerate().filter(|(_, r)| r.kind == ClipRowKind::Entry).map(|(i, _)| i).collect();
    let position = from.and_then(|f| entries.iter().position(|&i| i == f));
    let next = match position {
        None => 0,
        Some(p) => (p as i32 + delta).clamp(0, entries.len() as i32 - 1) as usize,
    };
    entries.get(next).copied()
}

impl App {
    pub(super) fn connect_clipboard(app: &Rc<Self>) {
        let state = app.island.global::<ClipboardState>();
        state.set_rows(ModelRc::from(app.clipboard.rows.clone()));
        let actions = app.island.global::<ClipboardActions>();
        let weak = Rc::downgrade(app);
        let with = |f: fn(&Rc<App>)| super::with(&weak, f);

        actions.on_opened(with(|app| {
            App::start_clipboard(app);
            app.request_listing();
        }));
        let w = weak.clone();
        actions.on_search_changed(move |_| {
            let Some(app) = w.upgrade() else { return };
            app.request_listing();
        });
        actions.on_filter_changed(with(|app| app.request_listing()));
        let w = weak.clone();
        actions.on_copy(move |id, close| {
            let Some(app) = w.upgrade() else { return };
            app.clipboard.close_after_copy.set(close.then_some(id as i64));
            app.clipboard.send(Command::Copy(id as i64));
        });
        let w = weak.clone();
        actions.on_toggle_pin(move |id| {
            let Some(app) = w.upgrade() else { return };
            let pinned = app.clipboard.rows.iter().any(|r| r.id == id && r.kind == ClipRowKind::Entry && r.pinned);
            app.clipboard.send(Command::SetPinned(id as i64, !pinned));
        });
        let w = weak.clone();
        actions.on_delete(move |id| {
            let Some(app) = w.upgrade() else { return };
            app.clipboard.send(Command::Delete(id as i64));
        });
        let w = weak.clone();
        actions.on_edit(move |id| {
            let Some(app) = w.upgrade() else { return };
            if id == 0 {
                let state = app.island.global::<ClipboardState>();
                state.set_form(SnippetForm::default());
                state.set_editing(true);
            } else {
                app.clipboard.send(Command::Edit(id as i64));
            }
        });
        let w = weak.clone();
        actions.on_save(move |form| {
            let Some(app) = w.upgrade() else { return };
            if form.content.trim().is_empty() {
                app.clipboard_notice("A snippet needs some text.");
                return;
            }
            app.clipboard.send(Command::Save(Snippet {
                id: (form.id != 0).then_some(form.id as i64),
                title: form.title.to_string(),
                text: form.content.to_string(),
                description: form.description.to_string(),
            }));
        });
        let w = weak.clone();
        actions.on_move_selection(move |delta| {
            let Some(app) = w.upgrade() else { return };
            let rows: Vec<ClipRow> = app.clipboard.rows.iter().collect();
            let current = app.clipboard.selected.get().and_then(|id| rows.iter().position(|r| r.id as i64 == id));
            if let Some(index) = step(&rows, current, delta) {
                app.clipboard.selected.set(Some(rows[index].id as i64));
                app.show_selection(&rows);
            }
        });
        actions.on_copy_selected(with(|app| {
            let rows: Vec<ClipRow> = app.clipboard.rows.iter().collect();
            let id = app.clipboard.selected.get().or_else(|| step(&rows, None, 0).map(|i| rows[i].id as i64));
            if let Some(id) = id {
                app.clipboard.close_after_copy.set(Some(id));
                app.clipboard.send(Command::Copy(id));
            }
        }));
        actions.on_delete_selected(with(|app| {
            let rows: Vec<ClipRow> = app.clipboard.rows.iter().collect();
            let Some(id) = app.clipboard.selected.get() else { return };
            // Pinned entries and snippets are only deleted with their own button, which asks.
            let Some(index) = rows.iter().position(|r| r.id as i64 == id && r.kind == ClipRowKind::Entry) else {
                return;
            };
            if rows[index].pinned || rows[index].manual {
                app.clipboard_notice("Pinned entries and snippets are deleted with their delete button.");
                return;
            }
            let next = step(&rows, Some(index), 1).filter(|&i| i != index).or_else(|| step(&rows, Some(index), -1));
            app.clipboard.selected.set(next.filter(|&i| i != index).map(|i| rows[i].id as i64));
            app.clipboard.send(Command::Delete(id));
        }));
        let w = weak.clone();
        actions.on_set_capture(move |on| {
            let Some(app) = w.upgrade() else { return };
            {
                let mut settings = app.settings.borrow_mut();
                settings.clipboard.capture = on;
                settings.clipboard.introduced = true;
            }
            app.clipboard.paused.set(false);
            App::apply_clipboard(&app);
            App::schedule_save(&app);
            app.refresh_settings_window();
            app.clipboard_notice(if on {
                "Copied text is now remembered."
            } else {
                "Capture is off. Saved entries stay."
            });
        });
        actions.on_dismiss_intro(with(|app| {
            app.settings.borrow_mut().clipboard.introduced = true;
            App::apply_clipboard(app);
            App::schedule_save(app);
        }));
        let w = weak.clone();
        actions.on_set_paused(move |paused| {
            let Some(app) = w.upgrade() else { return };
            app.clipboard.paused.set(paused);
            App::apply_clipboard(&app);
            app.clipboard_notice(if paused {
                "Paused until you resume or Lilt restarts."
            } else {
                "Copied text is remembered again."
            });
        });
        actions.on_clear_history(with(|app| app.clipboard.send(Command::ClearHistory)));
        actions.on_open_settings(with(|app| App::open_settings_at(app, super::preferences::CLIPBOARD_SECTION)));
        let w = weak;
        actions.on_busy(move |busy| {
            let Some(app) = w.upgrade() else { return };
            app.clipboard.busy.set(busy);
            if busy {
                app.collapse_timer.stop();
            } else if app.expanded() && !app.pointer_inside.get() {
                App::schedule_collapse(&app);
            }
        });
    }

    /// Starts the worker if it is not running yet.
    fn start_clipboard(app: &Rc<Self>) {
        if app.clipboard.service.borrow().is_some() {
            return;
        }
        let config = clipboard_config(&app.settings.borrow().clipboard);
        app.clipboard.config.replace(Some(config.clone()));
        let service = ClipboardService::start(app.clipboard.path.clone(), config, |update| {
            post(move |app| App::clipboard_update(app, update));
        });
        app.clipboard.service.replace(Some(service));
    }

    /// Applies the clipboard settings: the shelf, capture and the worker's options.
    pub(super) fn apply_clipboard(app: &Rc<Self>) {
        let settings = app.settings.borrow().clipboard.clone();
        let state = app.island.global::<AppState>();
        state.set_show_clipboard(settings.show);
        if !settings.show && state.get_side_panel() == SidePanel::Clipboard {
            state.set_side_panel(SidePanel::Tasks);
        }
        if !settings.capture {
            app.clipboard.paused.set(false);
        }
        let config = clipboard_config(&settings);
        let changed = app.clipboard.config.borrow().as_ref().is_some_and(|c| !same_config(c, &config));
        if changed {
            app.clipboard.config.replace(Some(config.clone()));
            app.clipboard.send(Command::Configure(config));
        }
        let listen = settings.show && settings.capture && !app.clipboard.paused.get();
        if listen {
            App::start_clipboard(app);
            if !app.clipboard.listening.get() {
                let ok = capture::listen(window::control_window());
                app.clipboard.listening.set(ok);
                if !ok {
                    tracing::warn!("clipboard notifications are unavailable");
                }
            }
        } else if app.clipboard.listening.replace(false) {
            capture::stop_listening(window::control_window());
        }
        let clip = app.island.global::<ClipboardState>();
        clip.set_capture_on(settings.capture);
        clip.set_paused(app.clipboard.paused.get());
        clip.set_show_intro(!settings.introduced && !settings.capture);
        app.update_clipboard_status();
    }

    /// `WM_CLIPBOARDUPDATE`: the worker reads the clipboard once it has settled.
    pub(super) fn clipboard_changed(&self) {
        if self.clipboard.listening.get() {
            self.clipboard.send(Command::Changed);
        }
    }

    /// The shelf's shortcut: opens the island on the shelf with the search field ready,
    /// or closes it if the shelf is already showing.
    pub(super) fn open_clipboard_shelf(app: &Rc<Self>) {
        if !app.settings.borrow().clipboard.show {
            return;
        }
        let state = app.island.global::<AppState>();
        if app.expanded() && state.get_side_panel() == SidePanel::Clipboard {
            App::set_expanded(app, false);
            return;
        }
        state.set_side_panel(SidePanel::Clipboard);
        app.clipboard.selected.set(None);
        App::set_expanded(app, true);
        // After the island has taken the focus for itself.
        let weak = Rc::downgrade(app);
        slint::Timer::single_shot(Duration::from_millis(30), move || {
            if let Some(app) = weak.upgrade() {
                let clip = app.island.global::<ClipboardState>();
                clip.set_search_focus(clip.get_search_focus() + 1);
            }
        });
    }

    fn request_listing(&self) {
        let state = self.island.global::<ClipboardState>();
        self.clipboard.send(Command::List { search: state.get_search().to_string(), filter: filter(&state) });
    }

    fn clipboard_update(app: &Rc<Self>, update: Update) {
        let state = app.island.global::<ClipboardState>();
        match update {
            Update::Listing { listing, total, history, bytes } => {
                app.clipboard.counts.set((total, history));
                app.show_listing(&listing);
                state.set_ready(true);
                state.set_history_count(history as i32);
                app.update_clipboard_status();
                if let Some(window) = app.settings_window.borrow().as_ref() {
                    window.set_clipboard_usage(usage_text(total, bytes).into());
                }
            }
            Update::Copied(id) => {
                state.set_copied_id(id as i32);
                let weak = Rc::downgrade(app);
                app.clipboard.copied_timer.start(slint::TimerMode::SingleShot, COPIED_TIME, move || {
                    if let Some(app) = weak.upgrade() {
                        app.island.global::<ClipboardState>().set_copied_id(0);
                    }
                });
                if app.clipboard.close_after_copy.take() == Some(id) {
                    App::set_expanded(app, false);
                }
            }
            Update::CopyFailed => app.clipboard_notice("The clipboard is busy. Try again."),
            Update::Skipped(Skip::LooksSecret) => app.clipboard_notice("Not kept: it looked like a password or key."),
            Update::Skipped(Skip::TooLarge) => app.clipboard_notice("Not kept: longer than 200,000 characters."),
            Update::Skipped(_) => {}
            Update::Editing(snippet) => {
                state.set_form(SnippetForm {
                    id: snippet.id.unwrap_or(0) as i32,
                    title: snippet.title.into(),
                    content: snippet.text.into(),
                    description: snippet.description.into(),
                });
                state.set_editing(true);
            }
            Update::Saved => {
                state.set_editing(false);
                app.clipboard_notice("Snippet saved.");
            }
            Update::Cleared(n) => app.clipboard_notice(&format!("Deleted {}.", plural(n, "entry", "entries"))),
            Update::Unavailable(reason) => {
                state.set_error(reason.into());
                state.set_ready(true);
                state.set_empty_title("Clipboard history is unavailable".into());
                state.set_empty_detail("Lilt could not open its clipboard database. See the log for details.".into());
            }
        }
    }

    fn show_listing(&self, listing: &Listing) {
        let state = self.island.global::<ClipboardState>();
        let search = state.get_search().trim().to_string();
        let grouped = search.is_empty() && state.get_filter() == ClipFilter::All;
        let rows = build_rows(&listing.entries, grouped, now_unix());
        // Keep the chosen entry; while searching, the best match is chosen.
        let keep = self.clipboard.selected.get().filter(|id| rows.iter().any(|r| r.id as i64 == *id));
        let selected =
            keep.or_else(|| (!search.is_empty()).then(|| step(&rows, None, 0)).flatten().map(|i| rows[i].id as i64));
        self.clipboard.selected.set(selected);
        self.clipboard.rows.set_vec(rows.clone());
        self.show_selection(&rows);

        let (title, detail) = if !search.is_empty() {
            ("No matches".to_owned(), format!("Nothing matches \u{201c}{search}\u{201d}."))
        } else {
            let capture = self.settings.borrow().clipboard.capture;
            match state.get_filter() {
                ClipFilter::Pinned => (
                    "Nothing pinned".into(),
                    "Pin an entry to keep it at the top. Pinned entries are never removed automatically.".into(),
                ),
                ClipFilter::Snippets => {
                    ("No snippets yet".into(), "Save text you use often with +, or edit a copied entry.".into())
                }
                ClipFilter::All if capture => {
                    ("Nothing copied yet".into(), "Copy some text in any app and it appears here.".into())
                }
                ClipFilter::All => (
                    "Nothing here yet".into(),
                    "Save snippets with +. Turn on capture in the menu to remember copied text.".into(),
                ),
                _ => ("Nothing here".into(), "No entries of this kind.".into()),
            }
        };
        state.set_empty_title(title.into());
        state.set_empty_detail(detail.into());
        if listing.matched > listing.entries.len() {
            tracing::debug!(shown = listing.entries.len(), matched = listing.matched, "clipboard list truncated");
        }
    }

    fn show_selection(&self, rows: &[ClipRow]) {
        let state = self.island.global::<ClipboardState>();
        let index = self.clipboard.selected.get().and_then(|id| rows.iter().position(|r| r.id as i64 == id));
        state.set_selected_top(index.map_or(0.0, |i| row_top(rows, i)));
        state.set_selected(index.map_or(-1, |i| i as i32));
    }

    fn update_clipboard_status(&self) {
        let settings = self.settings.borrow().clipboard.clone();
        let (total, _) = self.clipboard.counts.get();
        let items = plural(total, "entry", "entries");
        let status = if !settings.capture {
            format!("Capture off · {items}")
        } else if self.clipboard.paused.get() {
            format!("Paused · {items}")
        } else {
            format!("Remembering copied text · {items}")
        };
        self.island.global::<ClipboardState>().set_status(status.into());
    }

    /// A short message in place of the status line.
    fn clipboard_notice(&self, text: &str) {
        let state = self.island.global::<ClipboardState>();
        state.set_notice(SharedString::from(text));
        let island = self.island.as_weak();
        self.clipboard.notice_timer.start(slint::TimerMode::SingleShot, NOTICE_TIME, move || {
            if let Some(island) = island.upgrade() {
                island.global::<ClipboardState>().set_notice("".into());
            }
        });
    }

    /// From the settings window.
    pub(super) fn clear_clipboard(app: &Rc<Self>, everything: bool) {
        App::start_clipboard(app);
        app.clipboard.send(if everything { Command::DeleteAll } else { Command::ClearHistory });
    }

    /// Fills the settings window's storage line, starting the worker if needed.
    pub(super) fn refresh_clipboard_usage(app: &Rc<Self>) {
        App::start_clipboard(app);
        app.request_listing();
    }

    pub(super) fn reset_clipboard_ui(&self) {
        let state = self.island.global::<ClipboardState>();
        state.set_editing(false);
        self.clipboard.selected.set(None);
        state.set_selected(-1);
    }
}

pub(super) fn clipboard_config(settings: &crate::settings::Clipboard) -> Config {
    Config {
        policy: capture::Policy {
            excluded_apps: settings.excluded_apps.clone(),
            skip_unknown_sources: settings.skip_unknown_sources,
        },
        skip_secrets: settings.skip_secrets,
        retention: settings.retention.retention(),
        max_items: settings.max_items as usize,
    }
}

fn same_config(a: &Config, b: &Config) -> bool {
    a.policy.excluded_apps == b.policy.excluded_apps
        && a.policy.skip_unknown_sources == b.policy.skip_unknown_sources
        && a.skip_secrets == b.skip_secrets
        && a.retention == b.retention
        && a.max_items == b.max_items
}

/// "42 entries · 1.2 MB on disk".
fn usage_text(total: usize, bytes: u64) -> String {
    let size = if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} KB", bytes.div_ceil(1024))
    };
    format!("{} · {size} on disk", plural(total, "entry", "entries"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::Kind;

    fn summary(id: i64, pinned: bool, manual: bool) -> Summary {
        Summary {
            id,
            preview: format!("entry {id}"),
            title: String::new(),
            description: String::new(),
            kind: Kind::Text,
            created_at: 0,
            used_at: 0,
            source: None,
            pinned,
            manual,
            chars: 7,
            lines: 1,
        }
    }

    #[test]
    fn groups_pinned_snippets_and_recent() {
        let entries = [summary(1, true, false), summary(2, false, true), summary(3, false, false)];
        let rows = build_rows(&entries, true, 0);
        let shape: Vec<(ClipRowKind, String)> = rows.iter().map(|r| (r.kind, r.title.to_string())).collect();
        assert_eq!(rows.len(), 6);
        assert_eq!(shape[0], (ClipRowKind::Header, "Pinned".into()));
        assert_eq!(shape[2], (ClipRowKind::Header, "Snippets".into()));
        assert_eq!(shape[4], (ClipRowKind::Header, "Recent".into()));
        assert_eq!(build_rows(&entries, false, 0).len(), 3);
    }

    #[test]
    fn keyboard_steps_skip_headers_and_stop_at_the_ends() {
        let entries = [summary(1, true, false), summary(2, false, false), summary(3, false, false)];
        let rows = build_rows(&entries, true, 0);
        // Rows: header, 1, header, 2, 3.
        assert_eq!(step(&rows, None, 1), Some(1));
        assert_eq!(step(&rows, Some(1), 1), Some(3));
        assert_eq!(step(&rows, Some(3), 1), Some(4));
        assert_eq!(step(&rows, Some(4), 1), Some(4));
        assert_eq!(step(&rows, Some(3), -1), Some(1));
        assert_eq!(step(&rows, Some(1), -1), Some(1));
        assert_eq!(step(&[], None, 1), None);
        assert_eq!(row_top(&rows, 3), HEADER_HEIGHT * 2.0 + ENTRY_HEIGHT);
    }

    #[test]
    fn metadata_never_invents_a_source() {
        let mut entry = summary(1, false, false);
        entry.used_at = 0;
        assert_eq!(meta(&entry, 30), "Unknown app · just now");
        entry.source = Some("msedge.exe".into());
        entry.kind = Kind::Url;
        assert_eq!(meta(&entry, 30), "Microsoft Edge · just now · Link");
        let mut snippet = summary(2, false, true);
        snippet.description = "Email sign-off".into();
        snippet.lines = 4;
        assert_eq!(meta(&snippet, 30), "Email sign-off · 4 lines");
    }

    #[test]
    fn storage_is_described_in_familiar_units() {
        assert_eq!(usage_text(1, 4096), "1 entry · 4 KB on disk");
        assert_eq!(usage_text(250, 3 * 1_048_576), "250 entries · 3.0 MB on disk");
    }
}
