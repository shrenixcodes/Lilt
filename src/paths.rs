//! Locations of Lilt's files. Nothing is written anywhere else.

use std::path::{Path, PathBuf};

fn known_dir(variable: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)))
        .unwrap_or_default()
        .join(crate::identity::DATA_DIR)
}

/// Roaming user settings for this build's app identity.
pub fn roaming_data_dir() -> PathBuf {
    known_dir("APPDATA")
}

/// Machine-specific data such as logs for this build's app identity.
pub fn local_data_dir() -> PathBuf {
    known_dir("LOCALAPPDATA")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_are_named_after_the_app() {
        #[cfg(feature = "dev-build")]
        let expected = "Lilt Dev";
        #[cfg(not(feature = "dev-build"))]
        let expected = "Lilt";
        assert!(roaming_data_dir().ends_with(expected));
        assert!(local_data_dir().ends_with(expected));
    }
}
