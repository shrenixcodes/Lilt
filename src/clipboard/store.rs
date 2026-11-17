//! Clipboard storage: SQLite in `%LOCALAPPDATA%\Lilt\clipboard.sqlite3`.
//!
//! Versioned like the task store (`PRAGMA user_version`, numbered migrations). Deleted
//! text is overwritten inside the database file (`secure_delete`) and the write-ahead
//! log is truncated after deletions. That cannot reach copies Windows may keep elsewhere,
//! such as backups, restore points or blocks an SSD has remapped.
//!
//! Search reads the stored text and compares it in Rust, so it matches any part of a
//! word, a URL or a line of code, ignoring case in every language. With the entry limit
//! (at most 5 000 captured entries) a search takes a few milliseconds on the worker
//! thread; a full-text index (FTS5) would add a second copy of the text on disk and only
//! match whole words, so it is not used.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use rusqlite::{Connection, OptionalExtension, params};

use super::{Filter, Kind, Retention, Snippet, Summary};
use crate::paths;

const MIGRATIONS: &[&str] = &[
    // 1: entries
    "CREATE TABLE entries (
        id INTEGER PRIMARY KEY,
        text TEXT NOT NULL,
        title TEXT NOT NULL DEFAULT '',
        description TEXT NOT NULL DEFAULT '',
        kind INTEGER NOT NULL DEFAULT 0,
        created_at INTEGER NOT NULL,
        updated_at INTEGER NOT NULL,
        source TEXT,
        pinned INTEGER NOT NULL DEFAULT 0,
        manual INTEGER NOT NULL DEFAULT 0,
        fingerprint INTEGER NOT NULL,
        chars INTEGER NOT NULL,
        lines INTEGER NOT NULL
    );
    CREATE INDEX entries_fingerprint ON entries(fingerprint);
    CREATE INDEX entries_order ON entries(pinned, manual, updated_at);",
    // 2: image thumbnails
    "ALTER TABLE entries ADD COLUMN thumbnail BLOB;",
];

/// Columns of a [`Summary`]; the text is cut to the preview in SQL.
const SUMMARY: &str = "id, substr(text, 1, 400), title, description, kind, created_at, updated_at, source, pinned, manual, chars, lines, thumbnail IS NOT NULL, thumbnail";

#[derive(Debug)]
pub enum StoreError {
    Database(rusqlite::Error),
    /// A snippet needs some text.
    Empty,
    NewerVersion(i64),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Database(err) => write!(f, "clipboard database error: {err}"),
            Self::Empty => f.write_str("a snippet needs some text"),
            Self::NewerVersion(v) => write!(f, "the clipboard database is from a newer version of Lilt (schema {v})"),
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
    paths::local_data_dir().join("clipboard.sqlite3")
}

/// A page of the shelf: the entries shown and how many matched in all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Listing {
    pub entries: Vec<Summary>,
    pub matched: usize,
}

pub struct ClipboardStore {
    conn: Connection,
    path: Option<PathBuf>,
}

impl ClipboardStore {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Self::init(Connection::open(path)?, Some(path.to_path_buf()))
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        conn.busy_timeout(Duration::from_secs(2))?;
        conn.execute_batch(
            "PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL; PRAGMA cache_size = -256; \
             PRAGMA temp_store = MEMORY; PRAGMA secure_delete = ON;",
        )?;
        let store = Self { conn, path };
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

    /// Remembers a captured image. Returns the entry's id.
    pub fn capture_image(&self, thumbnail: &[u8], source: Option<&str>, now: i64) -> Result<i64> {
        // Use a hash of the thumbnail bytes for deduplication.
        let fingerprint = {
            let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
            for &byte in thumbnail {
                hash ^= byte as u64;
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
            hash as i64
        };
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM entries WHERE fingerprint = ?1 AND kind = 6 AND manual = 0 ORDER BY updated_at DESC LIMIT 1",
                [fingerprint],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE entries SET updated_at = ?2, source = COALESCE(?3, source) WHERE id = ?1",
                params![id, now, source],
            )?;
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO entries (text, kind, created_at, updated_at, source, fingerprint, chars, lines, thumbnail)
             VALUES ('', 6, ?1, ?1, ?2, ?3, 0, 0, ?4)",
            params![now, source, fingerprint, thumbnail],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// The thumbnail bytes of an image entry.
    pub fn thumbnail(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self.conn.query_row("SELECT thumbnail FROM entries WHERE id = ?1", [id], |r| r.get(0)).optional()?)
    }

    /// Remembers copied text. A repeat of captured text moves that entry to the top
    /// instead of adding another. Returns the entry's id.
    pub fn capture(&self, text: &str, kind: Kind, source: Option<&str>, now: i64) -> Result<i64> {
        let fingerprint = super::fingerprint(text);
        let existing: Option<i64> = self
            .conn
            .query_row(
                "SELECT id FROM entries WHERE fingerprint = ?1 AND manual = 0 ORDER BY updated_at DESC LIMIT 1",
                [fingerprint],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            self.conn.execute(
                "UPDATE entries SET updated_at = ?2, source = COALESCE(?3, source) WHERE id = ?1",
                params![id, now, source],
            )?;
            return Ok(id);
        }
        self.conn.execute(
            "INSERT INTO entries (text, kind, created_at, updated_at, source, fingerprint, chars, lines)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7)",
            params![
                text,
                kind.to_i64(),
                now,
                source,
                fingerprint,
                text.chars().count() as i64,
                text.lines().count().max(1) as i64
            ],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Entries matching `search` (any part of the text, title, description or app, without
    /// case) and `filter`: pinned first, then snippets, then captured text, newest first.
    pub fn list(&self, search: &str, filter: Filter, limit: usize) -> Result<Listing> {
        let query = search.trim().to_lowercase();
        let mut listing = Listing::default();
        if query.is_empty() {
            let mut statement = self.conn.prepare_cached(&format!(
                "SELECT {SUMMARY} FROM entries ORDER BY pinned DESC, manual DESC, updated_at DESC"
            ))?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let summary = summary(row)?;
                if filter.matches(summary.kind, summary.pinned, summary.manual) {
                    listing.matched += 1;
                    if listing.entries.len() < limit {
                        listing.entries.push(summary);
                    }
                }
            }
            return Ok(listing);
        }
        let mut statement = self.conn.prepare_cached(&format!(
            "SELECT {SUMMARY}, text FROM entries ORDER BY pinned DESC, manual DESC, updated_at DESC"
        ))?;
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let summary = summary(row)?;
            if !filter.matches(summary.kind, summary.pinned, summary.manual) {
                continue;
            }
            let text: String = row.get(14)?;
            let app = summary.source.as_deref().map(super::app_name).unwrap_or_default();
            let found = [text.as_str(), &summary.title, &summary.description, &app]
                .iter()
                .any(|field| field.to_lowercase().contains(&query));
            if found {
                listing.matched += 1;
                if listing.entries.len() < limit {
                    listing.entries.push(summary);
                }
            }
        }
        Ok(listing)
    }

    /// Number of entries: (all, captured history that is neither pinned nor a snippet).
    pub fn counts(&self) -> Result<(usize, usize)> {
        Ok(self.conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(pinned = 0 AND manual = 0), 0) FROM entries",
            [],
            |r| Ok((r.get::<_, i64>(0)? as usize, r.get::<_, i64>(1)? as usize)),
        )?)
    }

    /// The full text of an entry.
    pub fn text(&self, id: i64) -> Result<Option<String>> {
        Ok(self.conn.query_row("SELECT text FROM entries WHERE id = ?1", [id], |r| r.get(0)).optional()?)
    }

    pub fn snippet(&self, id: i64) -> Result<Option<Snippet>> {
        Ok(self
            .conn
            .query_row("SELECT id, title, text, description FROM entries WHERE id = ?1", [id], |r| {
                Ok(Snippet { id: Some(r.get(0)?), title: r.get(1)?, text: r.get(2)?, description: r.get(3)? })
            })
            .optional()?)
    }

    /// Marks an entry as just used (copied back), moving it to the top of its section.
    pub fn touch(&self, id: i64, now: i64) -> Result<()> {
        self.conn.execute("UPDATE entries SET updated_at = ?2 WHERE id = ?1", params![id, now])?;
        Ok(())
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        self.conn.execute("UPDATE entries SET pinned = ?2 WHERE id = ?1", params![id, pinned])?;
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM entries WHERE id = ?1", [id])?;
        self.truncate_log();
        Ok(())
    }

    /// Saves a snippet: a new one, or an entry edited by hand, which becomes a snippet
    /// and is then kept until deleted.
    pub fn save_snippet(&self, snippet: &Snippet, now: i64) -> Result<i64> {
        if snippet.text.trim().is_empty() {
            return Err(StoreError::Empty);
        }
        let title = snippet.title.trim();
        let description = snippet.description.trim();
        let text = snippet.text.as_str();
        let (fingerprint, kind) = (super::fingerprint(text), super::classify(text));
        let (chars, lines) = (text.chars().count() as i64, text.lines().count().max(1) as i64);
        match snippet.id {
            Some(id) => {
                self.conn.execute(
                    "UPDATE entries SET text = ?2, title = ?3, description = ?4, kind = ?5, updated_at = ?6,
                     manual = 1, fingerprint = ?7, chars = ?8, lines = ?9 WHERE id = ?1",
                    params![id, text, title, description, kind.to_i64(), now, fingerprint, chars, lines],
                )?;
                Ok(id)
            }
            None => {
                self.conn.execute(
                    "INSERT INTO entries (text, title, description, kind, created_at, updated_at, manual, fingerprint,
                     chars, lines) VALUES (?1, ?2, ?3, ?4, ?5, ?5, 1, ?6, ?7, ?8)",
                    params![text, title, description, kind.to_i64(), now, fingerprint, chars, lines],
                )?;
                Ok(self.conn.last_insert_rowid())
            }
        }
    }

    /// Applies the retention period and the entry limit to captured history; pinned
    /// entries and snippets are kept. Returns how many entries were removed.
    pub fn prune(&self, retention: Retention, max_items: usize, now: i64) -> Result<usize> {
        let mut removed = 0;
        if let Some(seconds) = retention.seconds() {
            removed += self
                .conn
                .execute("DELETE FROM entries WHERE pinned = 0 AND manual = 0 AND updated_at < ?1", [now - seconds])?;
        }
        removed += self.conn.execute(
            "DELETE FROM entries WHERE id IN (SELECT id FROM entries WHERE pinned = 0 AND manual = 0
             ORDER BY updated_at DESC LIMIT -1 OFFSET ?1)",
            [max_items as i64],
        )?;
        if removed > 0 {
            self.truncate_log();
        }
        Ok(removed)
    }

    /// Deletes captured history, keeping pinned entries and snippets.
    pub fn clear_history(&self) -> Result<usize> {
        let removed = self.conn.execute("DELETE FROM entries WHERE pinned = 0 AND manual = 0", [])?;
        self.compact();
        Ok(removed)
    }

    /// Deletes every entry, pinned entries and snippets included.
    pub fn delete_all(&self) -> Result<usize> {
        let removed = self.conn.execute("DELETE FROM entries", [])?;
        self.compact();
        Ok(removed)
    }

    /// Bytes used on disk by the database and its log.
    pub fn disk_usage(&self) -> u64 {
        let Some(path) = &self.path else {
            let pages: i64 = self.conn.query_row("PRAGMA page_count", [], |r| r.get(0)).unwrap_or(0);
            let size: i64 = self.conn.query_row("PRAGMA page_size", [], |r| r.get(0)).unwrap_or(0);
            return (pages * size) as u64;
        };
        ["", "-wal", "-shm"]
            .iter()
            .filter_map(|suffix| {
                let mut name = path.clone().into_os_string();
                name.push(suffix);
                std::fs::metadata(PathBuf::from(name)).ok()
            })
            .map(|m| m.len())
            .sum()
    }

    /// Moves the log into the database and empties it, so deleted text does not linger
    /// in the log file.
    fn truncate_log(&self) {
        let _ = self.conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()));
    }

    /// After a large deletion: also gives the freed space back to Windows.
    fn compact(&self) {
        self.truncate_log();
        let _ = self.conn.execute_batch("VACUUM");
        self.truncate_log();
    }
}

fn summary(row: &rusqlite::Row) -> rusqlite::Result<Summary> {
    let start: String = row.get(1)?;
    Ok(Summary {
        id: row.get(0)?,
        preview: super::preview(&start),
        title: row.get(2)?,
        description: row.get(3)?,
        kind: Kind::from_i64(row.get(4)?),
        created_at: row.get(5)?,
        used_at: row.get(6)?,
        source: row.get(7)?,
        pinned: row.get(8)?,
        manual: row.get(9)?,
        chars: row.get::<_, i64>(10)? as usize,
        lines: row.get::<_, i64>(11)? as usize,
        has_thumbnail: row.get(12)?,
        thumbnail: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> ClipboardStore {
        ClipboardStore::open_in_memory().unwrap()
    }

    fn ids(listing: &Listing) -> Vec<i64> {
        listing.entries.iter().map(|e| e.id).collect()
    }

    #[test]
    fn repeated_copies_move_to_the_top_instead_of_duplicating() {
        let s = store();
        let a = s.capture("first", Kind::Text, Some("notepad.exe"), 100).unwrap();
        let b = s.capture("second", Kind::Text, None, 200).unwrap();
        assert_eq!(ids(&s.list("", Filter::All, 50).unwrap()), [b, a]);
        // The same text with Windows line endings and a trailing space.
        assert_eq!(s.capture("first \r\n", Kind::Text, None, 300).unwrap(), a);
        assert_eq!(ids(&s.list("", Filter::All, 50).unwrap()), [a, b]);
        assert_eq!(s.counts().unwrap(), (2, 2));
        // The source is kept when the repeat's source is unknown.
        assert_eq!(s.list("", Filter::All, 50).unwrap().entries[0].source.as_deref(), Some("notepad.exe"));
    }

    #[test]
    fn unicode_and_multiline_text_round_trips_exactly() {
        let s = store();
        let text = "Grüße, 世界! 👋\r\n\tindented line\r\n";
        let id = s.capture(text, Kind::Multiline, None, 1).unwrap();
        assert_eq!(s.text(id).unwrap().as_deref(), Some(text));
        let entry = &s.list("", Filter::All, 10).unwrap().entries[0];
        assert_eq!(entry.lines, 2);
        assert_eq!(entry.chars, text.chars().count());
    }

    #[test]
    fn search_matches_any_part_ignoring_case_in_any_language() {
        let s = store();
        let url = s.capture("https://example.com/Reports/2026", Kind::Url, None, 1).unwrap();
        let german = s.capture("ÜBERSICHT der Straße", Kind::Text, Some("WINWORD.EXE"), 2).unwrap();
        let snippet = s
            .save_snippet(
                &Snippet {
                    id: None,
                    title: "Signature".into(),
                    text: "Best,\nShreyan".into(),
                    description: "Email".into(),
                },
                3,
            )
            .unwrap();
        assert_eq!(ids(&s.list("reports/20", Filter::All, 10).unwrap()), [url]);
        assert_eq!(ids(&s.list("übersicht", Filter::All, 10).unwrap()), [german]);
        assert_eq!(ids(&s.list("word", Filter::All, 10).unwrap()), [german], "by app name");
        assert_eq!(ids(&s.list("signature", Filter::All, 10).unwrap()), [snippet], "by title");
        assert_eq!(ids(&s.list("email", Filter::Manual, 10).unwrap()), [snippet], "by description");
        assert!(s.list("absent", Filter::All, 10).unwrap().entries.is_empty());
    }

    #[test]
    fn pinned_then_snippets_then_history() {
        let s = store();
        let old = s.capture("old", Kind::Text, None, 1).unwrap();
        let snippet = s.save_snippet(&Snippet { text: "snip".into(), ..Default::default() }, 2).unwrap();
        let new = s.capture("new", Kind::Text, None, 3).unwrap();
        s.set_pinned(old, true).unwrap();
        assert_eq!(ids(&s.list("", Filter::All, 10).unwrap()), [old, snippet, new]);
        assert_eq!(ids(&s.list("", Filter::Pinned, 10).unwrap()), [old]);
        assert_eq!(ids(&s.list("", Filter::Manual, 10).unwrap()), [snippet]);
        assert_eq!(ids(&s.list("", Filter::Recent, 10).unwrap()), [old, new]);
        let limited = s.list("", Filter::All, 2).unwrap();
        assert_eq!((limited.entries.len(), limited.matched), (2, 3));
    }

    #[test]
    fn retention_and_limits_spare_pinned_entries_and_snippets() {
        let s = store();
        let day = 86_400;
        let pinned = s.capture("pinned", Kind::Text, None, 0).unwrap();
        s.set_pinned(pinned, true).unwrap();
        let snippet = s.save_snippet(&Snippet { text: "kept".into(), ..Default::default() }, 0).unwrap();
        s.capture("old", Kind::Text, None, 0).unwrap();
        let recent: Vec<i64> =
            (0..5).map(|i| s.capture(&format!("recent {i}"), Kind::Text, None, 40 * day + i).unwrap()).collect();
        assert_eq!(s.prune(Retention::Days(30), 1000, 41 * day).unwrap(), 1);
        assert_eq!(s.prune(Retention::Forever, 3, 41 * day).unwrap(), 2);
        let left = ids(&s.list("", Filter::All, 50).unwrap());
        assert_eq!(left, [pinned, snippet, recent[4], recent[3], recent[2]]);
        assert_eq!(s.clear_history().unwrap(), 3);
        assert_eq!(ids(&s.list("", Filter::All, 50).unwrap()), [pinned, snippet]);
        assert_eq!(s.delete_all().unwrap(), 2);
        assert_eq!(s.counts().unwrap(), (0, 0));
    }

    #[test]
    fn editing_an_entry_makes_it_a_snippet() {
        let s = store();
        let id = s.capture("ssh deploy@example.com", Kind::Command, None, 1).unwrap();
        let mut snippet = s.snippet(id).unwrap().unwrap();
        snippet.title = "Deploy host".into();
        assert_eq!(s.save_snippet(&snippet, 2).unwrap(), id);
        let entry = &s.list("", Filter::Manual, 10).unwrap().entries[0];
        assert_eq!((entry.id, entry.title.as_str(), entry.manual), (id, "Deploy host", true));
        assert!(matches!(
            s.save_snippet(&Snippet { text: "  ".into(), ..Default::default() }, 3),
            Err(StoreError::Empty)
        ));
        // A copy of a snippet's text is still captured as history of its own.
        assert_ne!(s.capture("ssh deploy@example.com", Kind::Command, None, 4).unwrap(), id);
    }

    #[test]
    fn deleted_text_is_overwritten_in_the_file() {
        let dir = std::env::temp_dir().join(format!("lilt-clipboard-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("clipboard.sqlite3");
        let marker = "ZQX-unique-marker-7c1f";
        {
            let s = ClipboardStore::open(&path).unwrap();
            let id = s.capture(&format!("{marker} {}", "padding ".repeat(50)), Kind::Text, None, 1).unwrap();
            s.capture("other", Kind::Text, None, 2).unwrap();
            s.delete(id).unwrap();
            assert!(s.disk_usage() > 0);
        }
        let mut bytes = Vec::new();
        for suffix in ["", "-wal"] {
            let mut name = path.clone().into_os_string();
            name.push(suffix);
            bytes.extend(std::fs::read(PathBuf::from(name)).unwrap_or_default());
        }
        let found = bytes.windows(marker.len()).any(|w| w == marker.as_bytes());
        let _ = std::fs::remove_dir_all(&dir);
        assert!(!found, "deleted text is still in the database files");
    }

    #[test]
    fn searching_five_thousand_entries_is_quick() {
        let s = store();
        let tx = s.conn.unchecked_transaction().unwrap();
        for i in 0..5_000 {
            let text = format!("Entry {i}: {}", "lorem ipsum dolor sit amet ".repeat(20));
            s.capture(&text, Kind::Text, Some("notepad.exe"), i).unwrap();
        }
        tx.commit().unwrap();
        let start = std::time::Instant::now();
        let listing = s.list("entry 4999:", Filter::All, 200).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(listing.matched, 1);
        // About 3 MB of text compared in Rust; generous for slow CI machines.
        assert!(elapsed < Duration::from_millis(500), "search took {elapsed:?}");
    }
}
