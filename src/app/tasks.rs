//! The tasks box: connects the task store to the UI.
//!
//! The database is opened the first time the box is shown, so people who never use
//! Tasks pay nothing for it. Active tasks are kept in memory and the list is rebuilt
//! from them after each change; the archive is queried on demand.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use super::App;
use crate::calendar::date::{DAY, Date};
use crate::calendar::now_local;
use crate::settings::{TaskLayout, TaskOrder};
use crate::tasks::store::TaskStore;
use crate::tasks::time::{local_midnight_unix, local_minutes, now_unix};
use crate::tasks::{self, Draft, Priority, SortMode, Source, SyncState, Task, View};
use crate::ui::{
    ArchiveRow, ArchiveRowKind, Island, TaskForm, TaskRow, TaskRowKind, TaskView, TasksActions, TasksState,
};

/// "No due date" in the UI's day offsets.
const NO_DATE: i32 = -10_000;
/// Long enough for the tick to play before the task leaves the list.
const COMPLETION_TRANSITION: Duration = Duration::from_millis(420);
const UNDO_TIME: Duration = Duration::from_secs(6);
const ARCHIVE_LIMIT: usize = 300;

/// What the undo button reverts.
#[derive(Clone, Debug)]
enum Undo {
    Completed(i64),
    Due { id: i64, due: Option<Date>, minute: Option<u32> },
}

/// Parses a time of day such as `17:30`, `9.05`, `0930`, `5pm` or `5:30 pm`.
pub fn parse_time(text: &str) -> Option<u32> {
    let text = text.trim().to_ascii_lowercase();
    let (body, suffix) = match text.strip_suffix("am").or_else(|| text.strip_suffix('a')) {
        Some(body) => (body.trim(), Some(false)),
        None => match text.strip_suffix("pm").or_else(|| text.strip_suffix('p')) {
            Some(body) => (body.trim(), Some(true)),
            None => (text.as_str(), None),
        },
    };
    let (hour, minute) = match body.split_once([':', '.']) {
        Some((h, m)) if m.len() == 2 => (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?),
        Some(_) => return None,
        None if body.len() == 4 && suffix.is_none() => (body[..2].parse().ok()?, body[2..].parse().ok()?),
        None if !body.is_empty() && body.len() <= 2 => (body.parse().ok()?, 0),
        None => return None,
    };
    let hour = match suffix {
        Some(pm) => {
            if !(1..=12).contains(&hour) {
                return None;
            }
            (hour % 12) + if pm { 12 } else { 0 }
        }
        None => hour,
    };
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

fn day_offset(day: Option<Date>, today: Date) -> i32 {
    day.map_or(NO_DATE, |d| (d.to_days() - today.to_days()) as i32)
}

fn from_offset(offset: i32, today: Date) -> Option<Date> {
    (offset != NO_DATE).then(|| today.add_days(offset as i64))
}

fn view(state: &TasksState) -> View {
    match state.get_view() {
        TaskView::Today => View::Today,
        TaskView::Upcoming => View::Upcoming,
        _ => View::All,
    }
}

fn row(task: &Task, kind: TaskRowKind, now: i64, h24: bool) -> TaskRow {
    let mut meta: Vec<&str> = Vec::new();
    if !task.category.is_empty() {
        meta.push(&task.category);
    }
    if task.source != Source::Lilt {
        meta.push(task.source.label());
    }
    match task.sync {
        SyncState::Pending => meta.push("Sync pending"),
        SyncState::Conflict => meta.push("Changed in two places"),
        SyncState::LocalOnly => meta.push("Not synced"),
        _ => {}
    }
    if task.origin.is_some() {
        meta.push("From calendar");
    }
    TaskRow {
        kind,
        id: task.id as i32,
        title: task.title.as_str().into(),
        meta: meta.join(" · ").into(),
        due: tasks::due_label(task, now, h24).into(),
        overdue: tasks::urgency(task, now) == tasks::Urgency::Overdue,
        priority: task.priority as i32,
        priority_label: task.priority.label().into(),
        count: 0,
        imported: task.source != Source::Lilt,
        can_move_up: false,
        can_move_down: false,
    }
}

fn header(label: &str, priority: i32, count: usize) -> TaskRow {
    TaskRow {
        kind: TaskRowKind::Header,
        priority,
        priority_label: label.into(),
        count: count as i32,
        ..Default::default()
    }
}

/// The rows of the list: the focus card, then the tasks grouped by priority (or by
/// date in the Upcoming view).
pub fn build_rows(
    tasks: &[Task],
    view: View,
    filter: Option<Priority>,
    mode: SortMode,
    now: i64,
    h24: bool,
) -> Vec<TaskRow> {
    let arranged = tasks::arrange(tasks, view, filter, mode, now);
    let mut rows = Vec::new();
    let today = tasks::today(now);
    if view == View::Upcoming {
        let mut index = 0;
        while index < arranged.len() {
            let day = arranged[index].due;
            let group: Vec<&Task> = arranged[index..].iter().take_while(|t| t.due == day).copied().collect();
            let label = day.map(|d| tasks::day_label(d, today)).unwrap_or_default();
            rows.push(header(&label, -1, group.len()));
            rows.extend(group.iter().map(|t| row(t, TaskRowKind::Task, now, h24)));
            index += group.len();
        }
        return rows;
    }
    let focus = arranged.first().filter(|_| filter.is_none()).map(|t| t.id);
    for priority in Priority::DESCENDING {
        let group: Vec<&Task> = arranged.iter().filter(|t| t.priority == priority).copied().collect();
        let movable = |i: usize| (i > 0, i + 1 < group.len());
        let shown: Vec<(usize, &&Task)> = group.iter().enumerate().filter(|(_, t)| Some(t.id) != focus).collect();
        if let Some(position) = group.iter().position(|t| Some(t.id) == focus) {
            let mut card = row(group[position], TaskRowKind::Focus, now, h24);
            (card.can_move_up, card.can_move_down) = movable(position);
            rows.insert(0, card);
        }
        if shown.is_empty() {
            continue;
        }
        rows.push(header(priority.label(), priority as i32, shown.len()));
        for (i, task) in shown {
            let mut r = row(task, TaskRowKind::Task, now, h24);
            (r.can_move_up, r.can_move_down) = movable(i);
            rows.push(r);
        }
    }
    rows
}

fn completion_label(task: &Task, now: i64, h24: bool) -> String {
    let Some(done) = task.completed_at.map(local_minutes) else { return String::new() };
    let minute = done.rem_euclid(DAY) as u32;
    let time = crate::clock::format_time(minute / 60, minute % 60, h24);
    let day = tasks::today(done);
    let mut parts = vec![match (tasks::today(now).to_days() - day.to_days()) as i32 {
        0 | 1 => format!("Completed {time}"),
        _ => format!("Completed {}, {time}", tasks::day_label(day, tasks::today(now))),
    }];
    if task.priority != Priority::None {
        parts.push(task.priority.label().to_owned());
    }
    if let Some(due) = task.due {
        parts.push(format!("was due {}", tasks::day_label(due, tasks::today(now))));
    }
    if task.source != Source::Lilt {
        parts.push(task.source.label().to_owned());
    }
    parts.join(" · ")
}

/// Completed tasks with a header for each day or period.
pub fn build_archive(completed: &[Task], now: i64, h24: bool) -> Vec<ArchiveRow> {
    let mut rows = Vec::new();
    let mut section = String::new();
    for task in completed {
        let this = tasks::archive_section(local_minutes(task.completed_at.unwrap_or_default()), now);
        if this != section {
            rows.push(ArchiveRow { kind: ArchiveRowKind::Header, title: this.as_str().into(), ..Default::default() });
            section = this;
        }
        rows.push(ArchiveRow {
            kind: ArchiveRowKind::Task,
            id: task.id as i32,
            title: task.title.as_str().into(),
            detail: completion_label(task, now, h24).into(),
            notes: task.notes.as_str().into(),
            priority: task.priority as i32,
        });
    }
    rows
}

/// Callbacks into the application for what the tasks box does not own.
pub struct Hooks {
    /// A field of the box is being edited (the island must not collapse by itself).
    pub busy: Box<dyn Fn(bool)>,
    /// The user chose another sort order, to be saved.
    pub order_changed: Box<dyn Fn(SortMode)>,
    /// The user moved the box beside or below the island.
    pub layout_toggled: Box<dyn Fn()>,
}

/// The tasks box's controller: the store, the active tasks in memory and the UI state.
pub struct TasksController {
    island: Island,
    path: PathBuf,
    /// 24-hour clock, kept in step by the application.
    pub h24: Cell<bool>,
    order: Cell<SortMode>,
    store: RefCell<Option<TaskStore>>,
    tasks: RefCell<Vec<Task>>,
    undo: RefCell<Option<Undo>>,
    undo_timer: slint::Timer,
    sweep_timer: slint::Timer,
    search_timer: slint::Timer,
    /// A field of the box is being edited.
    pub busy: Cell<bool>,
    /// A task was ever created (tasks exist or the archive is not empty).
    used: Cell<bool>,
}

impl TasksController {
    pub fn new(island: &Island, path: PathBuf, h24: bool, order: SortMode) -> Rc<Self> {
        Rc::new(Self {
            island: island.clone_strong(),
            path,
            h24: Cell::new(h24),
            order: Cell::new(order),
            store: RefCell::default(),
            tasks: RefCell::default(),
            undo: RefCell::default(),
            undo_timer: slint::Timer::default(),
            sweep_timer: slint::Timer::default(),
            search_timer: slint::Timer::default(),
            busy: Cell::new(false),
            used: Cell::new(false),
        })
    }

    fn state(&self) -> TasksState<'_> {
        self.island.global::<TasksState>()
    }

    /// Connects the UI's task actions to this controller.
    pub fn connect(self: &Rc<Self>, hooks: Hooks) {
        let actions = self.island.global::<TasksActions>();
        let weak = Rc::downgrade(self);
        let Hooks { busy, order_changed, layout_toggled } = hooks;
        macro_rules! on {
            ($setter:ident, |$c:ident $(, $arg:ident)*| $body:expr) => {{
                let weak = weak.clone();
                actions.$setter(move |$($arg),*| {
                    if let Some($c) = weak.upgrade() {
                        $body;
                    }
                });
            }};
        }
        on!(on_opened, |c| c.open());
        on!(on_view_changed, |c| c.refresh());
        on!(on_refresh, |c| c.refresh());
        on!(on_undo, |c| c.undo_last());
        let w = weak.clone();
        actions.on_toggle_order(move || {
            if let Some(c) = w.upgrade() {
                let order = match c.order.get() {
                    SortMode::Priority => SortMode::Custom,
                    SortMode::Custom => SortMode::Priority,
                };
                c.order.set(order);
                order_changed(order);
                c.refresh();
            }
        });
        actions.on_toggle_layout(layout_toggled);
        let w = weak.clone();
        actions.on_busy(move |now_busy| {
            if let Some(c) = w.upgrade() {
                c.busy.set(now_busy);
                busy(now_busy);
            }
        });
        on!(on_quick_add, |c, title, priority, due, time, category| c
            .quick_add(&title, priority, due, &time, &category));
        on!(on_complete, |c, id| TasksController::complete(&c, id as i64));
        on!(on_edit, |c, id| c.edit(id as i64));
        on!(on_save, |c, form| c.save(form));
        on!(on_delete, |c, id| {
            c.state().set_editing(false);
            c.change(|store, _| store.delete(id as i64));
        });
        on!(on_defer, |c, id| TasksController::defer(&c, id as i64));
        on!(on_move, |c, id, direction| c.move_task(id as i64, direction));
        on!(on_restore, |c, id| c.change(|store, now| store.reopen(id as i64, now)));
        on!(on_delete_completed, |c, id| c.change(|store, _| store.delete(id as i64)));
        on!(on_clear_completed, |c| c.change(|store, _| store.clear_completed_local().map(drop)));
        on!(on_search_changed, |c, _text| {
            let weak = Rc::downgrade(&c);
            c.search_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(150), move || {
                if let Some(c) = weak.upgrade() {
                    c.refresh_archive();
                }
            });
        });
        actions.on_day_label(|offset| {
            if offset == NO_DATE {
                return "No date".into();
            }
            let today = tasks::today(now_local());
            tasks::day_label(today.add_days(offset as i64), today).into()
        });
        actions.on_time_valid(|text| parse_time(&text).is_some());
    }

    /// Applies the saved sort order.
    pub fn set_order(&self, order: SortMode) {
        if self.order.replace(order) != order {
            self.refresh();
        }
    }

    /// Opens the database on first use and fills the box.
    pub fn open(&self) {
        let state = self.state();
        if self.store.borrow().is_none() {
            match TaskStore::open(&self.path) {
                Ok(store) => {
                    self.store.replace(Some(store));
                    state.set_error("".into());
                }
                Err(err) => {
                    tracing::error!(%err, "task database unavailable");
                    state.set_ready(false);
                    state.set_error(format!("{err}.").into());
                    return;
                }
            }
        }
        state.set_ready(true);
        self.reload();
    }

    /// Re-reads the active tasks from the database and rebuilds the box.
    fn reload(&self) {
        let loaded = self.store.borrow().as_ref().map(|s| (s.active(), s.completed("", 1)));
        if let Some((Ok(active), completed)) = loaded {
            self.used.set(!active.is_empty() || completed.is_ok_and(|c| !c.is_empty()));
            self.tasks.replace(active);
        }
        self.refresh();
    }

    /// Counts, dates and progress, from memory and one count query.
    fn refresh_counts(&self) -> (i32, i32) {
        let state = self.state();
        let now = now_local();
        let today = tasks::today(now);
        let mode = self.order.get();
        let tasks = self.tasks.borrow();
        let count = |view| tasks::arrange(&tasks, view, None, mode, now).len() as i32;
        let (open, due_today, upcoming) = (count(View::All), count(View::Today), count(View::Upcoming));
        state.set_open_count(open);
        state.set_due_today_count(due_today);
        state.set_upcoming_count(upcoming);
        state.set_custom_order(mode == SortMode::Custom);
        state.set_date_label(tasks::date_label(today).into());
        let since = local_midnight_unix(now_unix());
        let done_today = self.store.borrow().as_ref().and_then(|s| s.completed_since(since).ok()).unwrap_or(0);
        state.set_done_today(done_today as i32);
        state.set_left_today(due_today);
        (open, due_today)
    }

    /// Rebuilds the list (or the archive), counts and empty states from memory.
    pub fn refresh(&self) {
        if self.store.borrow().is_none() {
            return;
        }
        let (open, _) = self.refresh_counts();
        let state = self.state();
        let now = now_local();
        let mode = self.order.get();
        let filter =
            (state.get_priority_filter() >= 0).then(|| Priority::from_index(state.get_priority_filter() as i64));
        let view = view(&state);
        let rows = build_rows(&self.tasks.borrow(), view, filter, mode, now, self.h24.get());
        let (title, detail) = if open == 0 {
            (
                "Nothing competing for your attention.",
                if self.used.get() {
                    "Add a task when something comes up."
                } else {
                    "Add a task to get started: press N or the + button."
                },
            )
        } else if filter.is_some() {
            ("No tasks with this priority here.", "Choose another priority, or Any priority.")
        } else if view == View::Today {
            ("Nothing due today.", "Tasks with later dates are under Upcoming.")
        } else {
            ("Nothing scheduled ahead.", "Give a task a due date to see it here.")
        };
        state.set_empty_title(title.into());
        state.set_empty_detail(detail.into());
        state.set_nothing_open(open == 0);
        state.set_first_use(!self.used.get());
        state.set_rows(ModelRc::new(VecModel::from(rows)));
        if state.get_view() == TaskView::Done {
            self.refresh_archive();
        }
    }

    fn refresh_archive(&self) {
        let state = self.state();
        let store = self.store.borrow();
        let Some(store) = store.as_ref() else { return };
        match store.completed(&state.get_search(), ARCHIVE_LIMIT) {
            Ok(completed) => {
                let rows = build_archive(&completed, now_local(), self.h24.get());
                state.set_archive(ModelRc::new(VecModel::from(rows)));
            }
            Err(err) => tracing::warn!(%err, "archive unavailable"),
        }
    }

    /// Called every minute while the box is visible: due labels and overdue states follow
    /// the clock.
    pub fn tick(&self) {
        if !self.state().get_editing() {
            self.refresh();
        }
    }

    /// Runs a change against the database, then reloads.
    fn change(&self, change: impl FnOnce(&TaskStore, i64) -> crate::tasks::store::Result<()>) {
        let result = self.store.borrow().as_ref().map(|store| change(store, now_unix()));
        if let Some(Err(err)) = result {
            tracing::warn!(%err, "task change failed");
        }
        self.reload();
    }

    fn quick_add(&self, title: &str, priority: i32, due: i32, time: &str, category: &str) {
        let state = self.state();
        let today = tasks::today(now_local());
        let due = from_offset(due, today);
        let minute = if due.is_some() && !time.trim().is_empty() {
            match parse_time(time) {
                Some(minute) => Some(minute),
                None => {
                    state.set_draft_error("Use a time such as 17:30 or 5:30 pm".into());
                    return;
                }
            }
        } else {
            None
        };
        let draft = Draft {
            title: title.into(),
            priority: Some(Priority::from_index(priority as i64)),
            due,
            due_minute: minute,
            category: category.into(),
            ..Default::default()
        };
        if draft.clean_title().is_none() {
            state.set_draft_error("Type a title first".into());
            return;
        }
        let created = self.store.borrow().as_ref().map(|store| store.create(&draft, now_unix()));
        match created {
            Some(Ok(_)) => {
                state.set_draft_error("".into());
                state.set_draft("".into());
                state.set_draft_time("".into());
                state.set_draft_priority(0);
                state.set_draft_due(NO_DATE);
                state.set_draft_category("".into());
                self.reload();
            }
            Some(Err(err)) => state.set_draft_error(format!("Could not save: {err}").into()),
            None => {}
        }
    }

    fn complete(this: &Rc<Self>, id: i64) {
        let now = now_unix();
        let title = this.tasks.borrow().iter().find(|t| t.id == id).map(|t| t.title.clone());
        let Some(title) = title else { return };
        let result = this.store.borrow().as_ref().map(|store| store.complete(id, now));
        if let Some(Err(err)) = result {
            tracing::warn!(%err, "could not complete task");
            return;
        }
        // The task is done now and the counts say so; it leaves the list once its tick
        // has played.
        if let Some(task) = this.tasks.borrow_mut().iter_mut().find(|t| t.id == id) {
            task.completed_at = Some(now);
        }
        this.used.set(true);
        this.refresh_counts();
        let weak = Rc::downgrade(this);
        this.sweep_timer.start(slint::TimerMode::SingleShot, COMPLETION_TRANSITION, move || {
            if let Some(this) = weak.upgrade() {
                this.reload();
            }
        });
        TasksController::offer_undo(this, Undo::Completed(id), format!("Completed \u{b7} {title}"));
    }

    fn defer(this: &Rc<Self>, id: i64) {
        let task = this.tasks.borrow().iter().find(|t| t.id == id).cloned();
        let Some(task) = task else { return };
        let tomorrow = tasks::today(now_local()).add_days(1);
        let due = match task.due {
            Some(due) if due >= tomorrow => due.add_days(1),
            _ => tomorrow,
        };
        this.change(|store, now| store.set_due(id, Some(due), task.due_minute, now));
        let label = tasks::day_label(due, tasks::today(now_local()));
        TasksController::offer_undo(
            this,
            Undo::Due { id, due: task.due, minute: task.due_minute },
            format!("Moved to {} \u{b7} {}", label.to_lowercase(), task.title),
        );
    }

    fn offer_undo(this: &Rc<Self>, undo: Undo, text: String) {
        let state = this.state();
        this.undo.replace(Some(undo));
        state.set_undo_text(text.into());
        state.set_undo_visible(true);
        let weak = Rc::downgrade(this);
        this.undo_timer.start(slint::TimerMode::SingleShot, UNDO_TIME, move || {
            if let Some(this) = weak.upgrade() {
                this.undo.replace(None);
                this.state().set_undo_visible(false);
            }
        });
    }

    fn undo_last(&self) {
        self.undo_timer.stop();
        self.state().set_undo_visible(false);
        match self.undo.take() {
            Some(Undo::Completed(id)) => {
                self.sweep_timer.stop();
                self.change(|store, now| store.reopen(id, now));
            }
            Some(Undo::Due { id, due, minute }) => self.change(|store, now| store.set_due(id, due, minute, now)),
            None => {}
        }
    }

    fn edit(&self, id: i64) {
        let task = self.store.borrow().as_ref().and_then(|s| s.get(id).ok().flatten());
        let Some(task) = task else { return };
        let today = tasks::today(now_local());
        let mut source = match task.source {
            Source::Lilt => "Lilt task, stored on this PC".to_owned(),
            other => other.label().to_owned(),
        };
        if let Some(list) = &task.external_list {
            source.push_str(" \u{b7} ");
            source.push_str(list);
        }
        let sync_note = match (task.source, task.sync) {
            (Source::Lilt, _) => "",
            (_, SyncState::LocalOnly) => "Changes stay in Lilt: Google allows read-only access",
            (_, SyncState::Pending) => "Changes will be sent to Google",
            (_, SyncState::Conflict) => "Changed in Lilt and in Google since the last sync",
            _ => "Title, notes, due date and completion sync with Google; priority, label and time stay in Lilt",
        };
        let state = self.state();
        state.set_form(TaskForm {
            id: task.id as i32,
            title: task.title.into(),
            notes: task.notes.into(),
            priority: task.priority as i32,
            due_offset: day_offset(task.due, today),
            time: task.due_minute.map(|m| crate::clock::format_time(m / 60, m % 60, true)).unwrap_or_default().into(),
            category: task.category.into(),
            source: source.into(),
            sync_note: sync_note.into(),
            origin: task.origin.unwrap_or_default().into(),
            time_allowed: true,
            error: SharedString::new(),
        });
        state.set_adding(false);
        state.set_editing(true);
    }

    fn save(&self, form: TaskForm) {
        let state = self.state();
        let fail = |message: &str| {
            let mut form = form.clone();
            form.error = message.into();
            state.set_form(form);
        };
        let today = tasks::today(now_local());
        let due = from_offset(form.due_offset, today);
        let minute = match (due, form.time.trim()) {
            (Some(_), time) if !time.is_empty() => match parse_time(time) {
                Some(minute) => Some(minute),
                None => return fail("Use a time such as 17:30 or 5:30 pm."),
            },
            _ => None,
        };
        let draft = Draft {
            title: form.title.to_string(),
            notes: form.notes.to_string(),
            priority: Some(Priority::from_index(form.priority as i64)),
            due,
            due_minute: minute,
            category: form.category.to_string(),
            origin: None,
        };
        if draft.clean_title().is_none() {
            return fail("A task needs a title.");
        }
        let result = self.store.borrow().as_ref().map(|store| store.update(form.id as i64, &draft, now_unix()));
        match result {
            Some(Err(err)) => fail(&format!("Could not save: {err}")),
            _ => {
                state.set_editing(false);
                self.reload();
            }
        }
    }

    /// Moves a task one place within its priority in the custom order.
    fn move_task(&self, id: i64, direction: i32) {
        let tasks = self.tasks.borrow().clone();
        let arranged = tasks::arrange(&tasks, View::All, None, SortMode::Custom, now_local());
        let Some(task) = arranged.iter().find(|t| t.id == id) else { return };
        let group: Vec<&&Task> = arranged.iter().filter(|t| t.priority == task.priority).collect();
        let Some(index) = group.iter().position(|t| t.id == id) else { return };
        let Some(other) = index.checked_add_signed(direction as isize).and_then(|i| group.get(i)) else { return };
        let (a, b) = (task.position, other.position);
        let (a, b) = if a == b { (b + direction as f64 * 0.5, a) } else { (b, a) };
        let (other_id, task_id) = (other.id, task.id);
        self.change(|store, now| {
            store.set_position(task_id, a, now)?;
            store.set_position(other_id, b, now)
        });
    }
}

impl App {
    /// Connects the tasks box to the application's collapse timer and settings.
    pub(super) fn connect_tasks(app: &Rc<Self>) {
        let weak = Rc::downgrade(app);
        let busy = weak.clone();
        let order = weak.clone();
        let layout = weak;
        app.tasks.connect(Hooks {
            busy: Box::new(move |busy_now| {
                let Some(app) = busy.upgrade() else { return };
                if busy_now {
                    app.collapse_timer.stop();
                } else if app.expanded() && !app.pointer_inside.get() {
                    App::schedule_collapse(&app);
                }
            }),
            order_changed: Box::new(move |mode| {
                let Some(app) = order.upgrade() else { return };
                app.settings.borrow_mut().tasks.order = match mode {
                    SortMode::Priority => TaskOrder::Priority,
                    SortMode::Custom => TaskOrder::Custom,
                };
                App::schedule_save(&app);
            }),
            layout_toggled: Box::new(move || {
                let Some(app) = layout.upgrade() else { return };
                {
                    let mut settings = app.settings.borrow_mut();
                    settings.tasks.layout = match settings.tasks.layout {
                        TaskLayout::Beside => TaskLayout::Below,
                        TaskLayout::Below => TaskLayout::Beside,
                    };
                }
                App::apply_settings(&app);
                App::schedule_save(&app);
                app.refresh_settings_window();
            }),
        });
    }

    /// Called every minute: keeps due labels current while the box is visible.
    pub(super) fn tick_tasks(&self) {
        let state = self.island.global::<crate::ui::AppState>();
        self.tasks.h24.set(self.h24.get());
        if state.get_expanded() && state.get_effective_tasks() {
            self.tasks.tick();
        }
    }
}

pub fn sort_mode(order: TaskOrder) -> SortMode {
    match order {
        TaskOrder::Custom => SortMode::Custom,
        TaskOrder::Priority => SortMode::Priority,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::{NOW, task};

    #[test]
    fn times_are_parsed_leniently() {
        assert_eq!(parse_time("17:30"), Some(17 * 60 + 30));
        assert_eq!(parse_time("9.05"), Some(9 * 60 + 5));
        assert_eq!(parse_time("0930"), Some(9 * 60 + 30));
        assert_eq!(parse_time("7"), Some(7 * 60));
        assert_eq!(parse_time("5pm"), Some(17 * 60));
        assert_eq!(parse_time("5:30 PM"), Some(17 * 60 + 30));
        assert_eq!(parse_time("12 am"), Some(0));
        assert_eq!(parse_time("12pm"), Some(12 * 60));
        for bad in ["", "24:00", "7:5", "13pm", "abc", "12345", "9:60"] {
            assert_eq!(parse_time(bad), None, "{bad}");
        }
    }

    #[test]
    fn rows_have_a_focus_card_then_priority_groups() {
        let tasks = vec![
            task(1, Priority::High, Some(0)),
            task(2, Priority::Critical, Some(3)),
            task(3, Priority::High, None),
            task(4, Priority::None, None),
        ];
        let rows = build_rows(&tasks, View::All, None, SortMode::Priority, NOW, true);
        let shape: Vec<(TaskRowKind, i32)> = rows.iter().map(|r| (r.kind, r.id)).collect();
        assert_eq!(
            shape,
            [
                (TaskRowKind::Focus, 2),
                (TaskRowKind::Header, 0),
                (TaskRowKind::Task, 1),
                (TaskRowKind::Task, 3),
                (TaskRowKind::Header, 0),
                (TaskRowKind::Task, 4),
            ]
        );
        assert_eq!((rows[1].priority_label.as_str(), rows[1].count), ("High", 2));
        assert_eq!(rows[2].due, "Today");
        assert_eq!(rows[5].priority_label, "No priority");
    }

    #[test]
    fn rows_offer_moves_within_a_priority() {
        let tasks = vec![task(1, Priority::Low, None), task(2, Priority::Low, None), task(3, Priority::High, None)];
        let rows = build_rows(&tasks, View::All, Some(Priority::Low), SortMode::Custom, NOW, true);
        let moves: Vec<(bool, bool)> =
            rows.iter().filter(|r| r.kind == TaskRowKind::Task).map(|r| (r.can_move_up, r.can_move_down)).collect();
        assert_eq!(moves, [(false, true), (true, false)]);
    }

    #[test]
    fn upcoming_rows_are_grouped_by_day() {
        let tasks =
            vec![task(1, Priority::Low, Some(2)), task(2, Priority::High, Some(1)), task(3, Priority::None, Some(1))];
        let rows = build_rows(&tasks, View::Upcoming, None, SortMode::Priority, NOW, true);
        let labels: Vec<String> = rows.iter().map(|r| r.priority_label.to_string()).collect();
        assert_eq!(labels[0], "Tomorrow");
        assert_eq!(rows.iter().filter(|r| r.kind == TaskRowKind::Header).count(), 2);
        assert_eq!(rows[1].id, 2, "higher priority first within a day");
    }

    #[test]
    fn imported_and_unsynced_tasks_say_so() {
        let mut t = task(1, Priority::None, None);
        t.source = Source::GoogleTasks;
        t.sync = SyncState::LocalOnly;
        t.category = "Work".into();
        let r = row(&t, TaskRowKind::Task, NOW, true);
        assert!(r.imported);
        assert_eq!(r.meta, "Work · Google Tasks · Not synced");
    }

    #[test]
    fn day_offsets_round_trip() {
        let today = tasks::today(NOW);
        assert_eq!(from_offset(NO_DATE, today), None);
        assert_eq!(day_offset(from_offset(-3, today), today), -3);
        assert_eq!(day_offset(None, today), NO_DATE);
    }
}
