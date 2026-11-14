//! Persistent task storage: SQLite in `%LOCALAPPDATA%\Lilt\tasks.sqlite3`.
//!
//! The schema is versioned with `PRAGMA user_version` and upgraded by numbered
//! migrations, each in its own transaction. A database written by a newer Lilt is
//! refused rather than modified. Credentials are never stored here.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, Row, params};

use super::{Draft, Priority, Source, SyncState, Task};
use crate::calendar::date::Date;
use crate::paths;

/// Schema steps; the database's `user_version` is the number applied so far.
const MIGRATIONS: &[&str] = &[
    // 1: tasks
    "CREATE TABLE tasks (
        id INTEGER PRIMARY KEY,
        title TEXT NOT NULL,
        notes TEXT NOT NULL DEFAULT '',
        priority INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        due_day INTEGER,
        due_minute INTEGER,
        completed_at INTEGER,
        category TEXT NOT NULL DEFAULT '',
        source INTEGER NOT NULL DEFAULT 0,
        external_list TEXT,
        external_id TEXT,
        sync_state INTEGER NOT NULL DEFAULT 0,
        position REAL NOT NULL DEFAULT 0,
        origin TEXT
    );
    CREATE UNIQUE INDEX tasks_external ON tasks(source, external_list, external_id) WHERE external_id IS NOT NULL;
    CREATE INDEX tasks_completed ON tasks(completed_at);",
];

const COLUMNS: &str = "id, title, notes, priority, created_at, updated_at, due_day, due_minute, completed_at, \
                       category, source, external_list, external_id, sync_state, position, origin";

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    /// A task needs a title.
    EmptyTitle,
    /// The database was written by a newer Lilt.
    NewerVersion(i64),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => write!(f, "task database error: {err}"),
            Self::EmptyTitle => f.write_str("a task needs a title"),
            Self::NewerVersion(v) => write!(f, "the task database is from a newer version of Lilt (schema {v})"),
        }
    }
}

impl std::error::Error for StoreError {}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Database(err)
    }
}

pub type Result<T> = std::result::Result<T, StoreError>;

pub fn default_path() -> PathBuf {
    paths::local_data_dir().join("tasks.sqlite3")
}

pub struct TaskStore {
    conn: Connection,
}

impl TaskStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?)
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(2))?;
        // A small page cache: the task list is tiny and is also kept in memory.
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA cache_size = -256; \
             PRAGMA temp_store = MEMORY;",
        )?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<()> {
        let version: i64 = self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > MIGRATIONS.len() as i64 {
            return Err(StoreError::NewerVersion(version));
        }
        for (index, sql) in MIGRATIONS.iter().enumerate().skip(version as usize) {
            let tx = self.conn.unchecked_transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", index as i64 + 1)?;
            tx.commit()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self.conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn task(row: &Row) -> rusqlite::Result<Task> {
        Ok(Task {
            id: row.get(0)?,
            title: row.get(1)?,
            notes: row.get(2)?,
            priority: Priority::from_index(row.get(3)?),
            created_at: row.get(4)?,
            updated_at: row.get(5)?,
            due: row.get::<_, Option<i64>>(6)?.map(Date::from_days),
            due_minute: row.get::<_, Option<i64>>(7)?.map(|m| m.clamp(0, 24 * 60 - 1) as u32),
            completed_at: row.get(8)?,
            category: row.get(9)?,
            source: Source::from_index(row.get(10)?),
            external_list: row.get(11)?,
            external_id: row.get(12)?,
            sync: SyncState::from_index(row.get(13)?),
            position: row.get(14)?,
            origin: row.get(15)?,
        })
    }

    /// Tasks that are not completed.
    pub fn active(&self) -> Result<Vec<Task>> {
        let mut statement =
            self.conn.prepare_cached(&format!("SELECT {COLUMNS} FROM tasks WHERE completed_at IS NULL"))?;
        let rows = statement.query_map([], Self::task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn get(&self, id: i64) -> Result<Option<Task>> {
        let mut statement = self.conn.prepare_cached(&format!("SELECT {COLUMNS} FROM tasks WHERE id = ?1"))?;
        Ok(statement.query_row([id], Self::task).optional()?)
    }

    fn next_position(&self) -> Result<f64> {
        Ok(self.conn.query_row("SELECT COALESCE(MAX(position), 0) + 1 FROM tasks", [], |r| r.get(0))?)
    }

    /// Creates a Lilt task. Only the title is required.
    pub fn create(&self, draft: &Draft, now: i64) -> Result<Task> {
        let title = draft.clean_title().ok_or(StoreError::EmptyTitle)?;
        let position = self.next_position()?;
        self.conn.execute(
            "INSERT INTO tasks (title, notes, priority, created_at, updated_at, due_day, due_minute, category, \
             position, origin) VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                title,
                draft.notes.trim(),
                draft.priority.unwrap_or(Priority::None) as i64,
                now,
                draft.due.map(Date::to_days),
                draft.due.and(draft.due_minute).map(i64::from),
                draft.category.trim(),
                position,
                draft.origin,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get(id)?.ok_or(StoreError::Database(rusqlite::Error::QueryReturnedNoRows))
    }

    /// Saves edited fields. For tasks from Google, a change to a field Google stores
    /// (title, notes, due date) marks the task for synchronisation; priority, category
    /// and due time are kept in Lilt only.
    pub fn update(&self, id: i64, draft: &Draft, now: i64) -> Result<()> {
        let title = draft.clean_title().ok_or(StoreError::EmptyTitle)?;
        let Some(old) = self.get(id)? else { return Ok(()) };
        let synced_fields_changed = old.title != title || old.notes != draft.notes.trim() || old.due != draft.due;
        let sync = Self::after_edit(&old, synced_fields_changed);
        self.conn.execute(
            "UPDATE tasks SET title = ?2, notes = ?3, priority = ?4, due_day = ?5, due_minute = ?6, category = ?7, \
             sync_state = ?8, updated_at = ?9 WHERE id = ?1",
            params![
                id,
                title,
                draft.notes.trim(),
                draft.priority.unwrap_or(old.priority) as i64,
                draft.due.map(Date::to_days),
                draft.due.and(draft.due_minute).map(i64::from),
                draft.category.trim(),
                sync as i64,
                now,
            ],
        )?;
        Ok(())
    }

    fn after_edit(old: &Task, synced_fields_changed: bool) -> SyncState {
        match (old.source, old.sync, synced_fields_changed) {
            (Source::Lilt, ..) => SyncState::Local,
            (_, SyncState::Synced, true) => SyncState::Pending,
            (_, state, _) => state,
        }
    }

    #[cfg(test)]
    pub fn set_priority(&self, id: i64, priority: Priority, now: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE tasks SET priority = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, priority as i64, now],
        )?;
        Ok(())
    }

    pub fn set_due(&self, id: i64, due: Option<Date>, minute: Option<u32>, now: i64) -> Result<()> {
        let Some(mut draft) = self.get(id)?.map(|t| Draft {
            title: t.title,
            notes: t.notes,
            priority: Some(t.priority),
            due: t.due,
            due_minute: t.due_minute,
            category: t.category,
            origin: t.origin,
        }) else {
            return Ok(());
        };
        draft.due = due;
        draft.due_minute = minute;
        self.update(id, &draft, now)
    }

    pub fn set_position(&self, id: i64, position: f64, now: i64) -> Result<()> {
        self.conn
            .execute("UPDATE tasks SET position = ?2, updated_at = ?3 WHERE id = ?1", params![id, position, now])?;
        Ok(())
    }

    /// Marks a task completed at `now`. It stays in the database until deleted.
    pub fn complete(&self, id: i64, now: i64) -> Result<()> {
        self.set_completed(id, Some(now), now)
    }

    /// Returns a completed task to the active list.
    pub fn reopen(&self, id: i64, now: i64) -> Result<()> {
        self.set_completed(id, None, now)
    }

    fn set_completed(&self, id: i64, completed_at: Option<i64>, now: i64) -> Result<()> {
        let Some(old) = self.get(id)? else { return Ok(()) };
        let sync = Self::after_edit(&old, true);
        self.conn.execute(
            "UPDATE tasks SET completed_at = ?2, sync_state = ?3, updated_at = ?4 WHERE id = ?1",
            params![id, completed_at, sync as i64, now],
        )?;
        Ok(())
    }

    /// Permanently removes a task.
    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM tasks WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Completed tasks, most recent first, optionally matching `search` in the title,
    /// notes or category.
    pub fn completed(&self, search: &str, limit: usize) -> Result<Vec<Task>> {
        let pattern = format!("%{}%", search.trim().replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
        let mut statement = self.conn.prepare_cached(&format!(
            "SELECT {COLUMNS} FROM tasks WHERE completed_at IS NOT NULL AND \
             (title LIKE ?1 ESCAPE '\\' OR notes LIKE ?1 ESCAPE '\\' OR category LIKE ?1 ESCAPE '\\') \
             ORDER BY completed_at DESC, id DESC LIMIT ?2"
        ))?;
        let rows = statement.query_map(params![pattern, limit as i64], Self::task)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn completed_since(&self, unix: i64) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM tasks WHERE completed_at IS NOT NULL AND completed_at >= ?1",
            [unix],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }

    /// Permanently removes completed tasks created in Lilt. Tasks from Google stay, so
    /// that clearing local history never affects another service.
    pub fn clear_completed_local(&self) -> Result<usize> {
        Ok(self.conn.execute("DELETE FROM tasks WHERE completed_at IS NOT NULL AND source = 0", [])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::tests::NOW;
    use crate::tasks::{SortMode, View, arrange};

    fn draft(title: &str) -> Draft {
        Draft { title: title.into(), ..Default::default() }
    }

    #[test]
    fn create_with_only_a_title() {
        let store = TaskStore::open_in_memory().unwrap();
        let task = store.create(&draft("  Write report "), 100).unwrap();
        assert_eq!(task.title, "Write report");
        assert_eq!(task.priority, Priority::None);
        assert_eq!((task.created_at, task.updated_at), (100, 100));
        assert!(!task.is_completed());
        assert_eq!(task.source, Source::Lilt);
        assert!(matches!(store.create(&draft("   "), 100), Err(StoreError::EmptyTitle)));
    }

    #[test]
    fn create_with_every_field() {
        let store = TaskStore::open_in_memory().unwrap();
        let due = Date::new(2026, 11, 20);
        let task = store
            .create(
                &Draft {
                    title: "Prepare slides".into(),
                    notes: "For the team meeting".into(),
                    priority: Some(Priority::High),
                    due,
                    due_minute: Some(15 * 60),
                    category: "Work".into(),
                    origin: Some("Google Calendar: Team meeting".into()),
                },
                5,
            )
            .unwrap();
        assert_eq!(store.get(task.id).unwrap().unwrap(), task);
        assert_eq!((task.due, task.due_minute, task.category.as_str()), (due, Some(900), "Work"));
        // A time without a date is meaningless and is not stored.
        let undated = store.create(&Draft { due_minute: Some(60), ..draft("x") }, 5).unwrap();
        assert_eq!(undated.due_minute, None);
    }

    #[test]
    fn edit_priority_due_and_position() {
        let store = TaskStore::open_in_memory().unwrap();
        let task = store.create(&draft("Call Sam"), 1).unwrap();
        store.set_priority(task.id, Priority::Critical, 2).unwrap();
        store.set_due(task.id, Date::new(2026, 12, 1), Some(30), 3).unwrap();
        store.set_position(task.id, -4.5, 4).unwrap();
        let mut edit = draft("Call Sam back");
        edit.notes = "About the invoice".into();
        edit.category = "Admin".into();
        edit.priority = Some(Priority::Critical);
        edit.due = Date::new(2026, 12, 1);
        edit.due_minute = Some(30);
        store.update(task.id, &edit, 5).unwrap();
        let task = store.get(task.id).unwrap().unwrap();
        assert_eq!(task.title, "Call Sam back");
        assert_eq!((task.priority, task.due, task.due_minute), (Priority::Critical, Date::new(2026, 12, 1), Some(30)));
        assert_eq!((task.position, task.category.as_str(), task.updated_at), (-4.5, "Admin", 5));
        assert_eq!(task.sync, SyncState::Local);
    }

    #[test]
    fn complete_undo_restore_and_delete() {
        let store = TaskStore::open_in_memory().unwrap();
        let a = store.create(&draft("a"), 1).unwrap();
        let b = store.create(&draft("b"), 1).unwrap();
        store.complete(a.id, 50).unwrap();
        assert_eq!(store.active().unwrap().iter().map(|t| t.id).collect::<Vec<_>>(), [b.id]);
        assert_eq!(store.get(a.id).unwrap().unwrap().completed_at, Some(50));
        assert_eq!(store.completed_since(40).unwrap(), 1);
        assert_eq!(store.completed_since(60).unwrap(), 0);
        // Undo and restore are the same operation.
        store.reopen(a.id, 51).unwrap();
        assert_eq!(store.active().unwrap().len(), 2);
        assert_eq!(store.get(a.id).unwrap().unwrap().completed_at, None);
        store.complete(a.id, 52).unwrap();
        store.delete(a.id).unwrap();
        assert_eq!(store.get(a.id).unwrap(), None);
    }

    #[test]
    fn archive_search_and_clear() {
        let store = TaskStore::open_in_memory().unwrap();
        for (i, title) in ["Invoice 100%", "Pay rent", "Book flights"].into_iter().enumerate() {
            let t = store.create(&draft(title), 1).unwrap();
            store.complete(t.id, 10 + i as i64).unwrap();
        }
        let notes = store.create(&Draft { notes: "rent receipt".into(), ..draft("Scan") }, 1).unwrap();
        store.complete(notes.id, 20).unwrap();
        store.create(&draft("Still open rent"), 1).unwrap();
        let titles = |q: &str| store.completed(q, 50).unwrap().into_iter().map(|t| t.title).collect::<Vec<_>>();
        assert_eq!(titles(""), ["Scan", "Book flights", "Pay rent", "Invoice 100%"]);
        assert_eq!(titles("RENT"), ["Scan", "Pay rent"]);
        assert_eq!(titles("100%"), ["Invoice 100%"]);
        assert_eq!(titles("%"), ["Invoice 100%"], "wildcards are literal");
        assert_eq!(store.clear_completed_local().unwrap(), 4);
        assert!(titles("").is_empty());
        assert_eq!(store.active().unwrap().len(), 1, "open tasks are kept");
    }

    #[test]
    fn data_survives_reopening_the_file() {
        let dir = std::env::temp_dir().join(format!("lilt-tasks-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("tasks.sqlite3");
        let id = {
            let store = TaskStore::open(&path).unwrap();
            let t = store.create(&Draft { priority: Some(Priority::High), ..draft("Persist me") }, 7).unwrap();
            store.create(&draft("Done"), 7).and_then(|d| store.complete(d.id, 8)).unwrap();
            t.id
        };
        let store = TaskStore::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), MIGRATIONS.len() as i64);
        let task = store.get(id).unwrap().unwrap();
        assert_eq!((task.title.as_str(), task.priority), ("Persist me", Priority::High));
        assert_eq!(store.completed("", 10).unwrap().len(), 1);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn newer_databases_are_left_alone() {
        let store = TaskStore::open_in_memory().unwrap();
        store.conn.pragma_update(None, "user_version", 99).unwrap();
        assert!(matches!(store.migrate(), Err(StoreError::NewerVersion(99))));
    }

    #[test]
    fn stored_tasks_sort_by_priority() {
        let store = TaskStore::open_in_memory().unwrap();
        for (title, priority) in [("low", Priority::Low), ("crit", Priority::Critical), ("none", Priority::None)] {
            store.create(&Draft { priority: Some(priority), ..draft(title) }, 1).unwrap();
        }
        let tasks = store.active().unwrap();
        let order: Vec<_> =
            arrange(&tasks, View::All, None, SortMode::Priority, NOW).into_iter().map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["crit", "low", "none"]);
    }
}
