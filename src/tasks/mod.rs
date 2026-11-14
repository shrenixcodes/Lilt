//! Tasks: the data model, priority ordering, views and labels.
//!
//! Everything here is pure so the ordering rules can be tested exactly. Times are Unix
//! seconds (UTC) for timestamps and local calendar dates for due dates; "now" is passed
//! in as local wall-clock minutes since the epoch (see `calendar::now_local`).

pub mod store;
pub mod time;

use crate::calendar::date::{DAY, Date};
use crate::clock;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    None = 0,
    Low = 1,
    Medium = 2,
    High = 3,
    Critical = 4,
}

impl Priority {
    /// Most important first: the order of the groups in the task list.
    pub const DESCENDING: [Priority; 5] =
        [Priority::Critical, Priority::High, Priority::Medium, Priority::Low, Priority::None];

    pub fn from_index(value: i64) -> Self {
        match value {
            4 => Self::Critical,
            3 => Self::High,
            2 => Self::Medium,
            1 => Self::Low,
            _ => Self::None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Critical => "Critical",
            Self::High => "High",
            Self::Medium => "Medium",
            Self::Low => "Low",
            Self::None => "No priority",
        }
    }
}

/// Where a task came from, which decides how it synchronises.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Lilt = 0,
    GoogleTasks = 1,
}

impl Source {
    pub fn from_index(value: i64) -> Self {
        if value == 1 { Self::GoogleTasks } else { Self::Lilt }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Lilt => "Lilt",
            Self::GoogleTasks => "Google Tasks",
        }
    }
}

/// Synchronisation state of a task with its source.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncState {
    /// A Lilt task: nothing to synchronise.
    Local = 0,
    /// Matches the last version seen from the source.
    Synced = 1,
    /// Changed in Lilt; the change has not reached the source yet.
    Pending = 2,
    /// Changed both in Lilt and at the source since the last synchronisation.
    Conflict = 3,
    /// Changed in Lilt, but the source cannot accept it (for example read-only access).
    LocalOnly = 4,
}

impl SyncState {
    pub fn from_index(value: i64) -> Self {
        match value {
            1 => Self::Synced,
            2 => Self::Pending,
            3 => Self::Conflict,
            4 => Self::LocalOnly,
            _ => Self::Local,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub notes: String,
    pub priority: Priority,
    pub created_at: i64,
    pub updated_at: i64,
    pub due: Option<Date>,
    /// Minute of the day; only Lilt tasks can have a due time.
    pub due_minute: Option<u32>,
    /// Set once the task is completed.
    pub completed_at: Option<i64>,
    pub category: String,
    pub source: Source,
    /// Task list (Google) the task belongs to.
    pub external_list: Option<String>,
    pub external_id: Option<String>,
    pub sync: SyncState,
    /// Manual order within the task's priority group.
    pub position: f64,
    /// Where the task was created from, e.g. a calendar event, for display.
    pub origin: Option<String>,
}

impl Task {
    pub fn is_completed(&self) -> bool {
        self.completed_at.is_some()
    }
}

/// The fields a user enters when creating or editing a task.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Draft {
    pub title: String,
    pub notes: String,
    pub priority: Option<Priority>,
    pub due: Option<Date>,
    pub due_minute: Option<u32>,
    pub category: String,
    pub origin: Option<String>,
}

impl Draft {
    /// The title with surrounding whitespace removed; a task needs one.
    pub fn clean_title(&self) -> Option<String> {
        let title = self.title.trim();
        (!title.is_empty()).then(|| title.chars().take(500).collect())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortMode {
    /// Priority, then urgency of the due date.
    #[default]
    Priority,
    /// Priority, then the user's own order within each priority.
    Custom,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    All,
    /// Overdue and due today.
    Today,
    /// Due after today, soonest first.
    Upcoming,
}

/// How pressing a task's due date is right now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Urgency {
    Overdue = 0,
    Today = 1,
    Upcoming = 2,
    Undated = 3,
}

pub fn today(now: i64) -> Date {
    Date::from_days(now.div_euclid(DAY))
}

pub fn urgency(task: &Task, now: i64) -> Urgency {
    let Some(due) = task.due else { return Urgency::Undated };
    let today = today(now);
    if due < today {
        return Urgency::Overdue;
    }
    if due > today {
        return Urgency::Upcoming;
    }
    match task.due_minute {
        Some(minute) if (minute as i64) < now.rem_euclid(DAY) => Urgency::Overdue,
        _ => Urgency::Today,
    }
}

/// The tasks to show for a view, in display order.
///
/// Priority always comes first: a manual order or a due date never lifts a task above
/// one of higher priority. Within a priority, the Priority mode shows overdue tasks,
/// then tasks due today, then the nearest upcoming dates and finally undated tasks;
/// the Custom mode uses the user's order. The Upcoming view lists by date instead.
pub fn arrange(tasks: &[Task], view: View, filter: Option<Priority>, mode: SortMode, now: i64) -> Vec<&Task> {
    let mut shown: Vec<&Task> = tasks
        .iter()
        .filter(|t| !t.is_completed())
        .filter(|t| filter.is_none_or(|p| t.priority == p))
        .filter(|t| match view {
            View::All => true,
            View::Today => matches!(urgency(t, now), Urgency::Overdue | Urgency::Today),
            View::Upcoming => urgency(t, now) == Urgency::Upcoming,
        })
        .collect();
    let due_key = |t: &Task| (t.due.map(Date::to_days).unwrap_or(i64::MAX), t.due_minute.unwrap_or(u32::MAX));
    shown.sort_by(|a, b| {
        let tie = || a.position.total_cmp(&b.position).then(a.created_at.cmp(&b.created_at)).then(a.id.cmp(&b.id));
        if view == View::Upcoming {
            return due_key(a).cmp(&due_key(b)).then(b.priority.cmp(&a.priority)).then_with(tie);
        }
        let by_priority = b.priority.cmp(&a.priority);
        match mode {
            SortMode::Priority => {
                by_priority.then(urgency(a, now).cmp(&urgency(b, now))).then(due_key(a).cmp(&due_key(b))).then_with(tie)
            }
            SortMode::Custom => by_priority.then_with(tie),
        }
    });
    shown
}

/// Short due label, e.g. "Today 17:30", "Tomorrow", "Mon 17", "12 Dec", "3 Jan 2027".
pub fn due_label(task: &Task, now: i64, h24: bool) -> String {
    let Some(due) = task.due else { return String::new() };
    let mut label = day_label(due, today(now));
    if let Some(minute) = task.due_minute {
        label.push(' ');
        label.push_str(&clock::format_time(minute / 60, minute % 60, h24));
    }
    label
}

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTH_NAMES: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

/// "Today", "Tomorrow", "Yesterday", a weekday within the coming week, else a date.
pub fn day_label(day: Date, today: Date) -> String {
    let delta = day.to_days() - today.to_days();
    match delta {
        0 => "Today".into(),
        1 => "Tomorrow".into(),
        -1 => "Yesterday".into(),
        2..=6 => format!("{} {}", WEEKDAYS[day.weekday() as usize], day.day),
        _ if day.year == today.year => format!("{} {}", day.day, MONTHS[day.month as usize - 1]),
        _ => format!("{} {} {}", day.day, MONTHS[day.month as usize - 1], day.year),
    }
}

/// "Sat 14 Nov".
pub fn date_label(day: Date) -> String {
    format!("{} {} {}", WEEKDAYS[day.weekday() as usize], day.day, MONTHS[day.month as usize - 1])
}

/// Section of the archive a completion falls in, given local minutes for both times.
pub fn archive_section(completed: i64, now: i64) -> String {
    let (day, today) = (today(completed), today(now));
    let delta = today.to_days() - day.to_days();
    if delta <= 0 {
        return "Today".into();
    }
    if delta == 1 {
        return "Yesterday".into();
    }
    // Monday-based weeks, matching `Date::weekday`.
    let week_start = today.to_days() - today.weekday() as i64;
    if day.to_days() >= week_start {
        return "Earlier this week".into();
    }
    format!("{} {}", MONTH_NAMES[day.month as usize - 1], day.year)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// 2026-11-14 (a Saturday) at 14:00, local minutes.
    pub const NOW: i64 = 20_771 * DAY + 14 * 60;

    pub fn task(id: i64, priority: Priority, due: Option<i64>) -> Task {
        Task {
            id,
            title: format!("task {id}"),
            notes: String::new(),
            priority,
            created_at: id,
            updated_at: id,
            due: due.map(|offset| Date::from_days(NOW.div_euclid(DAY) + offset)),
            due_minute: None,
            completed_at: None,
            category: String::new(),
            source: Source::Lilt,
            external_list: None,
            external_id: None,
            sync: SyncState::Local,
            position: id as f64,
            origin: None,
        }
    }

    fn ids(tasks: Vec<&Task>) -> Vec<i64> {
        tasks.into_iter().map(|t| t.id).collect()
    }

    #[test]
    fn now_is_the_expected_day() {
        assert_eq!(today(NOW), Date::new(2026, 11, 14).unwrap());
    }

    #[test]
    fn priorities_come_first_in_order() {
        let tasks = vec![
            task(1, Priority::Low, None),
            task(2, Priority::None, Some(-3)),
            task(3, Priority::Critical, None),
            task(4, Priority::Medium, Some(0)),
            task(5, Priority::High, Some(5)),
        ];
        assert_eq!(ids(arrange(&tasks, View::All, None, SortMode::Priority, NOW)), [3, 5, 4, 1, 2]);
        // A manual order never crosses priorities either.
        assert_eq!(ids(arrange(&tasks, View::All, None, SortMode::Custom, NOW)), [3, 5, 4, 1, 2]);
    }

    #[test]
    fn within_a_priority_overdue_then_today_then_nearest_then_undated() {
        let mut timed_past = task(6, Priority::High, Some(0));
        timed_past.due_minute = Some(9 * 60); // 09:00 today, before 14:00: overdue
        let mut timed_later = task(7, Priority::High, Some(0));
        timed_later.due_minute = Some(17 * 60);
        let tasks = vec![
            task(1, Priority::High, None),
            task(2, Priority::High, Some(4)),
            task(3, Priority::High, Some(0)),
            task(4, Priority::High, Some(-1)),
            task(5, Priority::High, Some(1)),
            task(8, Priority::High, Some(-5)),
            timed_past,
            timed_later,
        ];
        // Overdue (oldest first, then earlier today), today (timed before all-day), upcoming, undated.
        assert_eq!(ids(arrange(&tasks, View::All, None, SortMode::Priority, NOW)), [8, 4, 6, 7, 3, 5, 2, 1]);
    }

    #[test]
    fn custom_order_applies_within_each_priority() {
        let mut a = task(1, Priority::High, Some(-2));
        let mut b = task(2, Priority::High, None);
        let c = task(3, Priority::Low, Some(-9));
        a.position = 10.0;
        b.position = 5.0;
        let tasks = vec![a, b, c];
        assert_eq!(ids(arrange(&tasks, View::All, None, SortMode::Priority, NOW)), [1, 2, 3]);
        assert_eq!(ids(arrange(&tasks, View::All, None, SortMode::Custom, NOW)), [2, 1, 3]);
    }

    #[test]
    fn views_and_filters() {
        let mut done = task(9, Priority::Critical, Some(0));
        done.completed_at = Some(1);
        let tasks = vec![
            task(1, Priority::Low, Some(0)),
            task(2, Priority::High, Some(-1)),
            task(3, Priority::Critical, Some(3)),
            task(4, Priority::Medium, Some(1)),
            task(5, Priority::None, None),
            done,
        ];
        assert_eq!(ids(arrange(&tasks, View::Today, None, SortMode::Priority, NOW)), [2, 1]);
        // Upcoming is by date: tomorrow's medium task before the critical one in 3 days.
        assert_eq!(ids(arrange(&tasks, View::Upcoming, None, SortMode::Priority, NOW)), [4, 3]);
        assert_eq!(ids(arrange(&tasks, View::All, Some(Priority::High), SortMode::Priority, NOW)), [2]);
        assert_eq!(arrange(&tasks, View::All, None, SortMode::Priority, NOW).len(), 5, "completed tasks are hidden");
    }

    #[test]
    fn due_labels() {
        let mut t = task(1, Priority::None, Some(0));
        assert_eq!(due_label(&t, NOW, true), "Today");
        t.due_minute = Some(17 * 60 + 30);
        assert_eq!(due_label(&t, NOW, true), "Today 17:30");
        assert_eq!(due_label(&task(1, Priority::None, Some(1)), NOW, true), "Tomorrow");
        assert_eq!(due_label(&task(1, Priority::None, Some(2)), NOW, true), "Mon 16");
        assert_eq!(due_label(&task(1, Priority::None, Some(-1)), NOW, true), "Yesterday");
        assert_eq!(due_label(&task(1, Priority::None, Some(-10)), NOW, true), "4 Nov");
        assert_eq!(due_label(&task(1, Priority::None, Some(60)), NOW, true), "13 Jan 2027");
        assert_eq!(due_label(&task(1, Priority::None, None), NOW, true), "");
    }

    #[test]
    fn archive_sections() {
        let at = |days: i64| NOW - days * DAY;
        assert_eq!(archive_section(at(0), NOW), "Today");
        assert_eq!(archive_section(at(1), NOW), "Yesterday");
        // Saturday: Monday to Thursday of this week.
        assert_eq!(archive_section(at(4), NOW), "Earlier this week");
        assert_eq!(archive_section(at(6), NOW), "November 2026");
        assert_eq!(archive_section(at(40), NOW), "October 2026");
    }

    #[test]
    fn date_labels() {
        assert_eq!(date_label(today(NOW)), "Sat 14 Nov");
    }

    #[test]
    fn titles_are_required() {
        assert_eq!(Draft { title: "  ".into(), ..Default::default() }.clean_title(), None);
        assert_eq!(
            Draft { title: " Call Sam ".into(), ..Default::default() }.clean_title().as_deref(),
            Some("Call Sam")
        );
    }
}
