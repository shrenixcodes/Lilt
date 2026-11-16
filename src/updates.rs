//! Update checks and installer download/install against the GitHub Releases API.
//!
//! The installed version is `CARGO_PKG_VERSION`. The repository's latest release is
//! read from `/releases/latest`; the installer asset (produced by `installer/lilt.nsi`)
//! and its `.sha256` sidecar are what Lilt downloads. The installer is NSIS, run with
//! `/S` for a silent, non-interactive install; Windows still shows the UAC prompt.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

/// Current semantic version from Cargo.toml.
pub const LOCAL_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Where the user is sent to see what changed if they prefer to install by hand.
pub const RELEASES_URL: &str = "https://github.com/shrenixcodes/Lilt/releases/latest";

const RELEASE_ENDPOINT: &str = "https://api.github.com/repos/shrenixcodes/Lilt/releases/latest";
const USER_AGENT: &str = concat!("Lilt/", env!("CARGO_PKG_VERSION"));
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(120);
/// Rejects an installer larger than this; a real Lilt installer is a few MB.
const MAX_INSTALLER_BYTES: u64 = 80 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub struct Release {
    /// The version alone, e.g. `0.3.2` (leading `v` from the tag stripped).
    pub version: String,
    /// Full URL to the `.exe` asset (produced by `installer/build.ps1`).
    pub installer_url: Option<String>,
    /// Full URL to the matching `.sha256` sidecar.
    pub sha256_url: Option<String>,
    /// SHA-256 published by GitHub for the installer asset itself.
    pub installer_sha256: Option<String>,
    /// Asset byte length when GitHub reported it, for a rough progress indicator.
    pub installer_size: u64,
}

#[derive(Deserialize)]
struct ReleaseJson {
    tag_name: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<AssetJson>,
}

#[derive(Deserialize)]
struct AssetJson {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

fn clean_version(raw: &str) -> String {
    raw.trim().trim_start_matches(['v', 'V']).trim().to_string()
}

/// Returns true when `remote` is a higher semver than `local`. Unknown or malformed
/// versions never claim an update.
pub fn newer_version(local: &str, remote: &str) -> bool {
    let (l, r) = (clean_version(local), clean_version(remote));
    if l.is_empty() || r.is_empty() {
        return false;
    }
    match (parse_semver(&l), parse_semver(&r)) {
        (Some(a), Some(b)) => b > a,
        _ => false,
    }
}

fn parse_semver(text: &str) -> Option<(u32, u32, u32)> {
    // Accept `MAJOR.MINOR.PATCH`, ignore any `-pre` or `+meta`.
    let main = text.split(['-', '+']).next().unwrap_or(text);
    let parts: Vec<&str> = main.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    Some((parts[0].parse().ok()?, parts[1].parse().ok()?, parts[2].parse().ok()?))
}

/// Reads the latest release from GitHub. Drafts and pre-releases are ignored, so a
/// stable installation never auto-upgrades to a preview build.
pub fn latest_release() -> Result<Release, String> {
    let response = ureq::get(RELEASE_ENDPOINT)
        .set("Accept", "application/vnd.github+json")
        .set("User-Agent", USER_AGENT)
        .timeout(REQUEST_TIMEOUT)
        .call()
        .map_err(|error| error.to_string())?;
    let release: ReleaseJson = response.into_json().map_err(|error| error.to_string())?;
    if release.draft || release.prerelease {
        return Err("The latest release is a draft or pre-release.".into());
    }
    let version = clean_version(&release.tag_name);
    if parse_semver(&version).is_none() {
        return Err(format!("Release tag {:?} is not a semantic version.", release.name));
    }
    // Lilt publishes `Lilt-<version>-x64-setup.exe` with a `.sha256` next to it.
    let installer = release.assets.iter().find(|a| a.name.ends_with(".exe") && a.name.contains("x64"));
    let sha = release.assets.iter().find(|a| a.name.ends_with(".sha256"));
    Ok(Release {
        version,
        installer_url: installer.map(|a| a.browser_download_url.clone()),
        installer_size: installer.map(|a| a.size).unwrap_or(0),
        sha256_url: sha.map(|a| a.browser_download_url.clone()),
        installer_sha256: installer.and_then(|a| a.digest.as_deref()).and_then(sha256_from_digest),
    })
}

fn sha256_from_digest(digest: &str) -> Option<String> {
    let hash = digest.trim().strip_prefix("sha256:")?;
    (hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())).then(|| hash.to_ascii_lowercase())
}

fn absolute_url(url: &str) -> Result<(), String> {
    let lower = url.to_ascii_lowercase();
    if !(lower.starts_with("https://github.com/") || lower.starts_with("https://objects.githubusercontent.com/")) {
        return Err("Refusing to download an update from an unexpected host.".into());
    }
    Ok(())
}

/// Downloads `url` to `dest` with a strict size cap. The destination directory is
/// created if it doesn't exist; a partial file is removed on error.
pub fn download(url: &str, dest: &Path) -> Result<(), String> {
    absolute_url(url)?;
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let response = ureq::get(url)
        .set("User-Agent", USER_AGENT)
        .set("Accept", "application/octet-stream")
        .timeout(DOWNLOAD_TIMEOUT)
        .call()
        .map_err(|error| error.to_string())?;
    let length: u64 = response.header("content-length").and_then(|s| s.parse().ok()).unwrap_or(0);
    if length > MAX_INSTALLER_BYTES {
        return Err(format!("Update is larger than expected ({length} bytes)."));
    }
    let mut reader = response.into_reader().take(MAX_INSTALLER_BYTES);
    let mut file = std::fs::File::create(dest).map_err(|e| e.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(|e| {
            let _ = std::fs::remove_file(dest);
            e.to_string()
        })?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read]).map_err(|e| {
            let _ = std::fs::remove_file(dest);
            e.to_string()
        })?;
    }
    file.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// Computes a hex SHA-256 digest of a file.
pub fn sha256_of(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut state = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        state.update(&buffer[..n]);
    }
    Ok(state.hex())
}

/// Parses `<hex>  <filename>` or `<hex>` from `.sha256` text.
pub fn sha256_from_text(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let token = trimmed.split_whitespace().next()?;
        if token.len() == 64 && token.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(token.to_ascii_lowercase());
        }
    }
    None
}

/// Where downloaded installers live — the OS temp dir.
pub fn installer_target(version: &str) -> PathBuf {
    let safe: String = version.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
    std::env::temp_dir().join(format!("Lilt-{safe}-x64-setup.exe"))
}

/// Launches the installer silently; Windows still shows the UAC prompt before any file
/// is replaced.
pub fn run_installer(path: &Path) -> Result<(), String> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, PCWSTR, w};
    let target = HSTRING::from(path.as_os_str());
    // `/S` is NSIS's silent-install switch.
    let args = HSTRING::from("/S");
    let result =
        unsafe { ShellExecuteW(None, w!("runas"), &target, PCWSTR(args.as_ptr()), PCWSTR::null(), SW_SHOWNORMAL) };
    if result.0 as isize > 32 { Ok(()) } else { Err(format!("ShellExecuteW returned {}", result.0 as isize)) }
}

/// Tiny streaming SHA-256 that keeps `windows` as the only runtime dependency growth.
/// Correctness checked against the standard test vectors in the tests below.
struct Sha256 {
    state: [u32; 8],
    buffer: [u8; 64],
    buffer_len: usize,
    bytes: u64,
}

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19],
            buffer: [0u8; 64],
            buffer_len: 0,
            bytes: 0,
        }
    }

    fn update(&mut self, mut data: &[u8]) {
        self.bytes += data.len() as u64;
        if self.buffer_len > 0 {
            let needed = 64 - self.buffer_len;
            let take = needed.min(data.len());
            self.buffer[self.buffer_len..self.buffer_len + take].copy_from_slice(&data[..take]);
            self.buffer_len += take;
            data = &data[take..];
            if self.buffer_len == 64 {
                let buffer = self.buffer;
                self.compress(&buffer);
                self.buffer_len = 0;
            }
        }
        while data.len() >= 64 {
            let mut block = [0u8; 64];
            block.copy_from_slice(&data[..64]);
            self.compress(&block);
            data = &data[64..];
        }
        if !data.is_empty() {
            self.buffer[..data.len()].copy_from_slice(data);
            self.buffer_len = data.len();
        }
    }

    fn compress(&mut self, block: &[u8; 64]) {
        const K: [u32; 64] = [
            0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5, 0xd807aa98,
            0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786,
            0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8,
            0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13,
            0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819,
            0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a,
            0x5b9cca4f, 0x682e6ff3, 0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
            0xc67178f2,
        ];
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.state;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = h.wrapping_add(s1).wrapping_add(ch).wrapping_add(K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        self.state[0] = self.state[0].wrapping_add(a);
        self.state[1] = self.state[1].wrapping_add(b);
        self.state[2] = self.state[2].wrapping_add(c);
        self.state[3] = self.state[3].wrapping_add(d);
        self.state[4] = self.state[4].wrapping_add(e);
        self.state[5] = self.state[5].wrapping_add(f);
        self.state[6] = self.state[6].wrapping_add(g);
        self.state[7] = self.state[7].wrapping_add(h);
    }

    fn finalise(mut self) -> [u8; 32] {
        let bit_count = self.bytes.wrapping_mul(8);
        // Pad: 1 bit, zeros, then 64-bit length.
        self.update(&[0x80]);
        while self.buffer_len != 56 {
            self.update(&[0]);
        }
        let length = bit_count.to_be_bytes();
        self.update(&length);
        let mut out = [0u8; 32];
        for (i, value) in self.state.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&value.to_be_bytes());
        }
        out
    }

    fn hex(self) -> String {
        let bytes = self.finalise();
        let mut s = String::with_capacity(64);
        for b in bytes {
            s.push_str(&format!("{b:02x}"));
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_remote_semver_triggers_update() {
        assert!(newer_version("0.3.1", "0.3.2"));
        assert!(newer_version("v0.3.1", "V0.3.2"));
        assert!(newer_version("0.3.9", "0.4.0"));
        assert!(!newer_version("0.3.2", "0.3.2"));
        assert!(!newer_version("0.4.0", "0.3.9"), "older remote never upgrades");
        assert!(!newer_version("", "0.4.0"));
        assert!(!newer_version("0.4.0", ""));
        assert!(!newer_version("garbage", "0.4.0"));
    }

    #[test]
    fn sha256_text_strips_filename_suffix() {
        assert_eq!(
            sha256_from_text("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789  foo.exe\n"),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into())
        );
        assert_eq!(sha256_from_text("# comment\n"), None);
        assert_eq!(sha256_from_text(""), None);
    }

    #[test]
    fn github_asset_digest_is_accepted_only_when_it_is_sha256() {
        assert_eq!(
            sha256_from_digest("sha256:abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"),
            Some("abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789".into())
        );
        assert_eq!(sha256_from_digest("sha1:abcdef"), None);
    }

    #[test]
    fn sha256_matches_known_vectors() {
        let hash = |bytes: &[u8]| {
            let mut s = Sha256::new();
            s.update(bytes);
            s.hex()
        };
        assert_eq!(hash(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(hash(b"abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!(
            hash(b"The quick brown fox jumps over the lazy dog"),
            "d7a8fbb307d7809469ca9abcb0082e4f8d5651e46d3cdb762d02d0bf37c9e592"
        );
    }

    #[test]
    fn installer_target_is_under_temp() {
        let path = installer_target("0.3.2");
        assert!(path.starts_with(std::env::temp_dir()));
        assert!(path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(".exe")));
    }

    #[test]
    fn download_refuses_an_unexpected_host() {
        let tmp = std::env::temp_dir().join("lilt-update-test.bin");
        let err = download("https://evil.example.com/setup.exe", &tmp).unwrap_err();
        assert!(err.contains("unexpected host"));
    }
}
