//! Diagnostics written to `%LOCALAPPDATA%\Lilt\logs\lilt.log`.
//!
//! Only notable events are logged at the default level, so the file is written rarely.
//! Set `LILT_LOG` to `debug` or `trace` for more detail.

use std::fs::{File, OpenOptions};
use std::path::Path;
use std::sync::Mutex;

use tracing::Level;

/// The log is rotated at start-up once it exceeds this size; one previous file is kept.
const MAX_LOG_BYTES: u64 = 512 * 1024;

pub fn init(dir: &Path) {
    let level = match std::env::var("LILT_LOG").as_deref().map(str::to_ascii_lowercase).as_deref() {
        Ok("trace") => Level::TRACE,
        Ok("debug") => Level::DEBUG,
        Ok("warn") => Level::WARN,
        Ok("error") => Level::ERROR,
        _ => Level::INFO,
    };
    let Some(file) = open(dir) else { return };
    let initialised = tracing_subscriber::fmt()
        .with_writer(Mutex::new(file))
        .with_ansi(false)
        .with_target(false)
        .with_max_level(level)
        .try_init();
    if initialised.is_ok() {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            tracing::error!(%info, "panic");
            previous(info);
        }));
    }
}

fn open(dir: &Path) -> Option<File> {
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join("lilt.log");
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES) {
        let _ = std::fs::rename(&path, dir.join("lilt.old.log"));
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotates_large_logs() {
        let dir = std::env::temp_dir().join(format!("lilt-log-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("lilt.log"), vec![b'x'; MAX_LOG_BYTES as usize + 1]).unwrap();
        let file = open(&dir).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 0);
        assert!(dir.join("lilt.old.log").exists());
    }
}
