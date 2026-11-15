use std::time::Duration;

use serde::Deserialize;

pub const BUILT_REVISION: &str = env!("LILT_BUILD_SHA");
pub const BRANCH_URL: &str = "https://github.com/shrenixcodes/Lilt/commits/main";

#[derive(Deserialize)]
struct Commit {
    sha: String,
}

pub fn update_available(local: &str, remote: &str) -> bool {
    !local.is_empty() && !remote.is_empty() && !local.eq_ignore_ascii_case(remote)
}

pub fn branch_revision() -> Result<String, String> {
    let response = ureq::get("https://api.github.com/repos/shrenixcodes/Lilt/commits/main")
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", concat!("Lilt/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(8))
        .call()
        .map_err(|error| error.to_string())?;
    let commit: Commit = response.into_json().map_err(|error| error.to_string())?;
    if commit.sha.len() != 40 || !commit.sha.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("GitHub returned an invalid commit revision".into());
    }
    Ok(commit.sha)
}

#[cfg(test)]
mod tests {
    use super::update_available;

    #[test]
    fn newer_main_commit_is_available() {
        assert!(update_available("abcdef123456", "123456abcdef"));
    }

    #[test]
    fn same_commit_is_current() {
        assert!(!update_available("ABCDEF123456", "abcdef123456"));
    }

    #[test]
    fn unknown_build_revision_never_claims_an_update() {
        assert!(!update_available("", "abcdef123456"));
        assert!(!update_available("abcdef123456", ""));
    }
}
