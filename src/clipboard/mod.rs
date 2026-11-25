//! The clipboard shelf: copied text Lilt remembers on this PC, and snippets saved by hand.
//!
//! Capture is opt-in and off by default. When it is on, Windows tells Lilt each time the
//! clipboard changes (`AddClipboardFormatListener`, no polling); a worker thread then
//! checks, in this order, whether the copying app asked for its content to be left alone
//! (`ExcludeClipboardContentFromMonitorProcessing`, `CanIncludeInClipboardHistory`),
//! whether the app is excluded, and whether the clipboard holds text at all, before it
//! reads anything. Text that looks like a password or key is not kept. Entries are stored
//! in SQLite under `%LOCALAPPDATA%\Lilt` and never leave the PC: they are not logged,
//! uploaded or sent anywhere.

pub mod capture;
pub mod service;
pub mod store;

/// What a piece of copied text looks like, for filtering and display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    Multiline,
    Url,
    Code,
    Json,
    Command,
    Image,
}

impl Kind {
    pub fn to_i64(self) -> i64 {
        match self {
            Kind::Text => 0,
            Kind::Multiline => 1,
            Kind::Url => 2,
            Kind::Code => 3,
            Kind::Json => 4,
            Kind::Command => 5,
            Kind::Image => 6,
        }
    }

    pub fn from_i64(value: i64) -> Self {
        match value {
            1 => Kind::Multiline,
            2 => Kind::Url,
            3 => Kind::Code,
            4 => Kind::Json,
            5 => Kind::Command,
            6 => Kind::Image,
            _ => Kind::Text,
        }
    }

    /// Shown in a monospaced face.
    pub fn is_technical(self) -> bool {
        matches!(self, Kind::Code | Kind::Json | Kind::Command)
    }

    pub fn label(self) -> &'static str {
        match self {
            Kind::Text => "Text",
            Kind::Multiline => "Text",
            Kind::Url => "Link",
            Kind::Code => "Code",
            Kind::Json => "JSON",
            Kind::Command => "Command",
            Kind::Image => "Image",
        }
    }
}

/// The list filters of the shelf.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    Pinned,
    /// Captured text, newest first.
    Recent,
    /// Snippets saved by hand.
    Manual,
    Urls,
    /// Code, JSON and commands.
    Code,
    /// Captured images.
    Images,
    /// Plain and multi-line text.
    Other,
}

impl Filter {
    pub fn matches(self, kind: Kind, pinned: bool, manual: bool) -> bool {
        match self {
            Filter::All => true,
            Filter::Pinned => pinned,
            Filter::Recent => !manual,
            Filter::Manual => manual,
            Filter::Urls => kind == Kind::Url,
            Filter::Code => kind.is_technical(),
            Filter::Images => kind == Kind::Image,
            Filter::Other => matches!(kind, Kind::Text | Kind::Multiline),
        }
    }
}

/// How long captured text is kept. Pinned entries and snippets are kept until deleted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Retention {
    Days(u32),
    Forever,
}

impl Retention {
    pub fn seconds(self) -> Option<i64> {
        match self {
            Retention::Days(days) => Some(days as i64 * 86_400),
            Retention::Forever => None,
        }
    }
}

/// One entry as listed in the shelf: everything but the full text.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    pub id: i64,
    /// The start of the text, at most [`PREVIEW_CHARS`] characters.
    pub preview: String,
    pub title: String,
    pub description: String,
    pub kind: Kind,
    pub created_at: i64,
    /// Last captured, copied back or edited.
    pub used_at: i64,
    /// The copying app's executable name, when Windows reported one.
    pub source: Option<String>,
    pub pinned: bool,
    pub manual: bool,
    /// Characters and lines of the full text.
    pub chars: usize,
    pub lines: usize,
    /// Whether the entry has a stored image thumbnail.
    pub has_thumbnail: bool,
    /// Compressed thumbnail PNG bytes if available.
    pub thumbnail: Option<Vec<u8>>,
}

/// A snippet as edited in the shelf.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snippet {
    /// `None` for a new snippet.
    pub id: Option<i64>,
    pub title: String,
    pub text: String,
    pub description: String,
}

/// Longest text that is remembered, in UTF-16 code units (about 400 KB). Longer copies
/// are skipped without being read in full.
pub const MAX_CHARS: usize = 200_000;
/// Characters of each entry kept for the list.
pub const PREVIEW_CHARS: usize = 280;

/// Text as compared for duplicates: Windows and Unix line endings and trailing
/// whitespace do not make two copies different.
pub fn normalize(text: &str) -> String {
    text.replace("\r\n", "\n").trim_end().to_owned()
}

/// A 64-bit FNV-1a hash of the normalised text, to find a repeated copy without
/// comparing every stored entry. Not a security measure.
pub fn fingerprint(text: &str) -> i64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in normalize(text).bytes() {
        hash ^= byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash as i64
}

/// The start of `text` for the list, with line breaks kept (the list shows two lines).
pub fn preview(text: &str) -> String {
    let trimmed = text.trim_start_matches(['\r', '\n']);
    let mut preview: String = trimmed.chars().take(PREVIEW_CHARS).collect();
    preview = preview.replace("\r\n", "\n").replace('\t', "    ");
    preview
}

/// Classifies copied text. Conservative: anything unclear is plain text.
pub fn classify(text: &str) -> Kind {
    let trimmed = text.trim();
    let single_line = !trimmed.contains('\n');
    if single_line && is_url(trimmed) {
        return Kind::Url;
    }
    if (trimmed.starts_with('{') && trimmed.ends_with('}') || trimmed.starts_with('[') && trimmed.ends_with(']'))
        && trimmed.len() >= 2
        && serde_json::from_str::<serde_json::Value>(trimmed).is_ok_and(|v| v.is_object() || v.is_array())
    {
        return Kind::Json;
    }
    if single_line && is_command(trimmed) {
        return Kind::Command;
    }
    if looks_like_code(trimmed) {
        return Kind::Code;
    }
    if single_line { Kind::Text } else { Kind::Multiline }
}

fn is_url(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    let rest = ["https://", "http://", "ftp://", "mailto:", "file:///"].iter().find_map(|p| lower.strip_prefix(p));
    let rest = rest.or_else(|| lower.strip_prefix("www."));
    rest.is_some_and(|r| !r.is_empty() && !r.contains(char::is_whitespace) && r.contains(['.', '/', '@', ':']))
}

/// Commands people copy into a terminal: a known tool followed by arguments.
fn is_command(text: &str) -> bool {
    const TOOLS: &[&str] = &[
        "git",
        "cargo",
        "rustup",
        "npm",
        "npx",
        "pnpm",
        "yarn",
        "node",
        "python",
        "python3",
        "py",
        "pip",
        "pip3",
        "winget",
        "choco",
        "scoop",
        "docker",
        "kubectl",
        "helm",
        "ssh",
        "scp",
        "curl",
        "wget",
        "dotnet",
        "go",
        "make",
        "cmake",
        "gh",
        "az",
        "aws",
        "gcloud",
        "terraform",
        "sudo",
        "apt",
        "apt-get",
        "brew",
        "cd",
        "ls",
        "dir",
        "mkdir",
        "rm",
        "cp",
        "mv",
        "cat",
        "echo",
        "set",
        "setx",
        "powershell",
        "pwsh",
        "cmd",
        "wsl",
        "ping",
        "ipconfig",
        "nslookup",
        "netsh",
        "sfc",
        "dism",
        "code",
        "deno",
        "bun",
        "java",
        "mvn",
        "gradle",
    ];
    let text = text.strip_prefix("$ ").or_else(|| text.strip_prefix("> ")).unwrap_or(text);
    let first = text.split_whitespace().next().unwrap_or_default();
    if first.len() > 2 && first.contains('-') && first.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        // PowerShell cmdlets: Get-ChildItem, Set-Location, …
        let (verb, noun) = first.split_once('-').unwrap_or_default();
        return verb.chars().all(|c| c.is_ascii_alphabetic())
            && noun.chars().next().is_some_and(|c| c.is_ascii_uppercase());
    }
    TOOLS.contains(&first.to_ascii_lowercase().as_str()) && text.split_whitespace().count() >= 2
}

fn looks_like_code(text: &str) -> bool {
    const MARKERS: &[&str] = &[
        "fn ",
        "let ",
        "const ",
        "function ",
        "def ",
        "class ",
        "import ",
        "#include",
        "public ",
        "private ",
        "return ",
        "=>",
        "->",
        "::",
        "#!/",
        "<?php",
        "</",
        "/>",
        "SELECT ",
        "INSERT ",
        "UPDATE ",
        "CREATE ",
        "var ",
        "using ",
        "package ",
        "struct ",
        "impl ",
        "async ",
        "await ",
    ];
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    if lines.is_empty() {
        return false;
    }
    let punctuated = lines
        .iter()
        .filter(|l| {
            let l = l.trim_end();
            l.ends_with(';') || l.ends_with('{') || l.ends_with('}') || l.ends_with(')') || l.ends_with(':')
        })
        .count();
    let indented = lines.iter().filter(|l| l.starts_with("    ") || l.starts_with('\t')).count();
    let markers = MARKERS.iter().filter(|m| text.contains(**m)).count();
    if lines.len() == 1 {
        // One line needs strong evidence: a marker and code punctuation.
        return markers >= 1 && punctuated == 1 && text.contains(['(', '{', '=', ';']);
    }
    markers >= 1 && (punctuated * 3 >= lines.len() || indented * 3 >= lines.len()) || markers >= 3
}

/// Why copied text was not kept. Only these reasons are logged, never the text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Skip {
    /// The copying app asked clipboard managers to ignore it.
    ExcludedByApp,
    /// The app is on the user's exclusion list.
    ExcludedApp,
    /// Windows did not say which app copied, and unknown sources are skipped.
    UnknownSource,
    NotText,
    Empty,
    TooLarge,
    /// It looks like a password, key or token.
    LooksSecret,
    /// Lilt put it there itself.
    OwnCopy,
    /// The clipboard was busy or could not be read.
    Unavailable,
}

/// Conservative checks for passwords, keys and tokens. Each pattern is specific enough
/// that ordinary text rarely matches; a miss is possible, so password managers should
/// still be excluded (they are by default).
pub fn looks_secret(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.contains("-----BEGIN") && trimmed.contains("PRIVATE KEY") {
        return true;
    }
    if trimmed.contains("-----BEGIN PGP PRIVATE KEY BLOCK-----") {
        return true;
    }
    for token in trimmed.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | ',' | ';' | '=' | ':')) {
        if is_token(token) {
            return true;
        }
    }
    has_secret_assignment(trimmed)
}

/// API keys and tokens with well-known prefixes, and JSON Web Tokens.
fn is_token(token: &str) -> bool {
    let body_ok = |body: &str, min: usize| {
        body.len() >= min && body.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    };
    const PREFIXED: &[(&str, usize)] = &[
        ("ghp_", 30),
        ("gho_", 30),
        ("ghu_", 30),
        ("ghs_", 30),
        ("ghr_", 30),
        ("github_pat_", 40),
        ("glpat-", 20),
        ("xoxb-", 20),
        ("xoxp-", 20),
        ("xoxa-", 20),
        ("xoxs-", 20),
        ("sk_live_", 16),
        ("rk_live_", 16),
        ("sk_test_", 16),
        ("sk-ant-", 30),
        ("sk-proj-", 30),
        ("sk-", 40),
        ("AIza", 35),
        ("ya29.", 30),
        ("npm_", 30),
        ("pypi-", 40),
        ("SG.", 40),
    ];
    for (prefix, min) in PREFIXED {
        if let Some(body) = token.strip_prefix(prefix)
            && body_ok(&body.replace('.', ""), *min)
        {
            return true;
        }
    }
    // AWS access key IDs.
    if token.len() == 20
        && (token.starts_with("AKIA") || token.starts_with("ASIA"))
        && token.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return true;
    }
    // JSON Web Tokens: three base64url parts, the first two starting with `{"`.
    let parts: Vec<&str> = token.split('.').collect();
    parts.len() == 3
        && parts[0].starts_with("eyJ")
        && parts[1].starts_with("eyJ")
        && parts.iter().all(|p| p.len() >= 8 && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

/// `password = hunter22`, `"api_key": "…"`, `Password=…;` in a connection string.
fn has_secret_assignment(text: &str) -> bool {
    const NAMES: &[&str] = &[
        "password",
        "passwd",
        "pwd",
        "passphrase",
        "secret",
        "client_secret",
        "api_key",
        "apikey",
        "api-key",
        "access_token",
        "auth_token",
        "refresh_token",
        "private_key",
        "secret_key",
        "aws_secret_access_key",
    ];
    let lower = text.to_ascii_lowercase();
    for name in NAMES {
        let mut from = 0;
        while let Some(found) = lower[from..].find(name) {
            let start = from + found;
            from = start + name.len();
            // A whole word: not part of a longer identifier such as `password_hint`.
            let before = lower[..start].chars().next_back();
            if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
                continue;
            }
            let rest = &text[from..];
            let rest = rest.trim_start_matches(['"', '\'']);
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix([':', '=']) else { continue };
            let value = rest.trim_start().trim_start_matches(['"', '\'']);
            let value: String =
                value.chars().take_while(|c| !c.is_whitespace() && !matches!(c, '"' | '\'' | ';' | ',')).collect();
            if value.chars().count() >= 6 && !is_placeholder(&value) {
                return true;
            }
        }
    }
    false
}

/// Values in documentation and templates, which are not secrets.
fn is_placeholder(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with('<')
        || lower.starts_with("${")
        || lower.starts_with("{{")
        || lower.starts_with('$')
        || lower.starts_with("%")
        || lower.chars().all(|c| c == '*' || c == 'x' || c == '.')
        || ["your", "example", "placeholder", "changeme", "redacted", "process.env", "os.environ", "env("]
            .iter()
            .any(|p| lower.contains(p))
}

/// Executables excluded by default: password managers, which put passwords on the
/// clipboard. Most also mark their copies as private, which Lilt honours as well.
pub const DEFAULT_EXCLUDED_APPS: &[&str] =
    &["1Password.exe", "Bitwarden.exe", "KeePass.exe", "KeePassXC.exe", "Enpass.exe", "Dashlane.exe"];

/// Whether `source` (an executable name) is on the exclusion list. Names compare
/// without case and with or without `.exe`.
pub fn is_excluded(source: &str, excluded: &[String]) -> bool {
    let stem = |s: &str| {
        let s = s.trim().to_ascii_lowercase();
        s.strip_suffix(".exe").map(str::to_owned).unwrap_or(s)
    };
    let source = stem(source);
    excluded.iter().any(|e| stem(e) == source)
}

/// A friendlier name for well-known executables; others show as their file name.
pub fn app_name(exe: &str) -> String {
    let stem = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
    let known = match stem.to_ascii_lowercase().as_str() {
        "msedge" => "Microsoft Edge",
        "chrome" => "Google Chrome",
        "firefox" => "Firefox",
        "brave" => "Brave",
        "opera" => "Opera",
        "code" => "Visual Studio Code",
        "devenv" => "Visual Studio",
        "windowsterminal" => "Terminal",
        "explorer" => "File Explorer",
        "notepad" => "Notepad",
        "winword" => "Word",
        "excel" => "Excel",
        "powerpnt" => "PowerPoint",
        "outlook" | "olk" => "Outlook",
        "ms-teams" | "teams" => "Teams",
        "slack" => "Slack",
        "discord" => "Discord",
        "obsidian" => "Obsidian",
        "notion" => "Notion",
        "spotify" => "Spotify",
        "lilt" => "Lilt",
        _ => stem,
    };
    known.to_owned()
}

/// "just now", "5 min ago", "3 h ago", "yesterday", "4 days ago", or a date.
pub fn relative_time(then: i64, now: i64) -> String {
    let seconds = (now - then).max(0);
    match seconds {
        0..60 => "just now".into(),
        60..3_600 => format!("{} min ago", seconds / 60),
        3_600..86_400 => format!("{} h ago", seconds / 3_600),
        86_400..172_800 => "yesterday".into(),
        _ if seconds < 30 * 86_400 => format!("{} days ago", seconds / 86_400),
        _ => {
            const MONTHS: [&str; 12] =
                ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
            let date = crate::calendar::date::Date::from_days(then.div_euclid(86_400));
            format!("{} {} {}", date.day, MONTHS[date.month as usize - 1], date.year)
        }
    }
}

/// Maximum size of a compressed thumbnail stored in the database, in bytes.
/// Keeps the database lean even with many captured screenshots.
pub const MAX_THUMBNAIL_BYTES: usize = 512 * 1024;

/// Maximum size of a full-resolution captured PNG image stored in the database.
pub const MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

/// Maximum dimensions for a thumbnail. Larger images are scaled down.
pub const THUMBNAIL_MAX_SIDE: u32 = 256;

/// Extracts a CSS hex colour from copied text, if the whole text is one colour.
/// Accepts `#RGB`, `#RRGGBB` and `#RRGGBBAA`.
pub fn detect_hex_color(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hex = &trimmed[1..];
    let valid_len = matches!(hex.len(), 3 | 4 | 6 | 8);
    if valid_len && hex.chars().all(|c| c.is_ascii_hexdigit()) { Some(trimmed) } else { None }
}

/// Expands a 3 or 4-digit hex colour to 6 digits (without alpha). Returns the
/// six hex digits as `RRGGBB` (no `#` prefix), suitable for rendering a swatch.
pub fn normalize_hex_color(hex: &str) -> Option<String> {
    let hex = hex.strip_prefix('#').unwrap_or(hex);
    match hex.len() {
        3 | 4 => {
            let mut out = String::with_capacity(6);
            for c in hex.chars().take(3) {
                out.push(c);
                out.push(c);
            }
            Some(out)
        }
        6 | 8 => Some(hex[..6].to_owned()),
        _ => None,
    }
}

/// Parses a hex colour string into (r, g, b) components, 0–255.
pub fn parse_hex_rgb(hex: &str) -> Option<(u8, u8, u8)> {
    let norm = normalize_hex_color(hex)?;
    let r = u8::from_str_radix(&norm[0..2], 16).ok()?;
    let g = u8::from_str_radix(&norm[2..4], 16).ok()?;
    let b = u8::from_str_radix(&norm[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Extracts the domain from a URL for the link preview badge.
/// `https://docs.google.com/spreadsheets/d/…` → `docs.google.com`.
pub fn extract_domain(text: &str) -> Option<String> {
    let trimmed = text.trim();
    let lower = trimmed.to_ascii_lowercase();
    let after_scheme = ["https://", "http://", "ftp://"].iter().find_map(|p| lower.strip_prefix(p)).or_else(|| {
        lower.strip_prefix("www.").map(|r| {
            // "www." without a scheme: return everything after "www."
            let _ = r;
            &trimmed[4..]
        })
    })?;
    // `after_scheme` is lowercase; use the original text's offset for the domain.
    let scheme_len = trimmed.len() - after_scheme.len();
    let rest = &trimmed[scheme_len..];
    let host = rest.split(&['/', '?', '#', ':'][..]).next().unwrap_or(rest);
    if host.contains('.') && !host.is_empty() { Some(host.to_ascii_lowercase()) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_copies() {
        assert_eq!(classify("https://example.com/a?b=c"), Kind::Url);
        assert_eq!(classify("www.rust-lang.org"), Kind::Url);
        assert_eq!(classify("see https://example.com for details"), Kind::Text);
        assert_eq!(classify("{\"name\": \"Lilt\", \"tags\": [1, 2]}"), Kind::Json);
        assert_eq!(classify("[1, 2, 3]"), Kind::Json);
        assert_eq!(classify("{not json}"), Kind::Text);
        assert_eq!(classify("git commit -m \"fix\""), Kind::Command);
        assert_eq!(classify("$ cargo test --release"), Kind::Command);
        assert_eq!(classify("Get-ChildItem -Recurse"), Kind::Command);
        assert_eq!(classify("cd"), Kind::Text);
        assert_eq!(classify("fn main() {\n    println!(\"hi\");\n}"), Kind::Code);
        assert_eq!(classify("def add(a, b):\n    return a + b"), Kind::Code);
        assert_eq!(classify("let total = price * count;"), Kind::Code);
        assert_eq!(classify("Meeting moved to Thursday."), Kind::Text);
        assert_eq!(classify("Dear Priya,\n\nThanks for the notes.\nSee you soon."), Kind::Multiline);
        assert_eq!(classify("Returns: the total.\nUse it wisely."), Kind::Multiline);
    }

    #[test]
    fn finds_keys_and_tokens() {
        // Made-up samples, put together at run time so that no complete token-shaped
        // string sits in the source, where secret scanners would flag it.
        let token = |prefix: &str, body: &str| format!("{prefix}{body}");
        let secrets = [
            token(
                "-----BEGIN OPENSSH ",
                "PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----",
            ),
            token("-----BEGIN RSA ", "PRIVATE KEY-----"),
            token("ghp", "_0123456789abcdefghijABCDEFGHIJ0123"),
            token("token: github", "_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyzABCDEFGH"),
            token("AKIA", "IOSFODNN7EXAMPLE"),
            token("export OPENAI_API_KEY=sk", "-proj-abcdefghijklmnopqrstuvwxyz0123456789ABCD"),
            token("xox", "b-123456789012-123456789012-abcdefghijklmnop"),
            token("sk_li", "ve_51H8abcdefghijklmnop"),
            token("AIza", "SyA1234567890abcdefghijklmnopqrstuv"),
            token("eyJhbGciOiJIUzI1NiJ9", ".eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U"),
            token("password = ", "hunter22"),
            token("\"api_key\": ", "\"a8f5f167f44f4964e6c998dee827110c\""),
            token("Server=db;Database=app;User Id=sa;", "Password=Tr0ub4dor&3;"),
        ];
        for secret in &secrets {
            assert!(looks_secret(secret), "missed {secret:?}");
        }
    }

    #[test]
    fn ordinary_text_is_not_secret() {
        let plain = [
            "The password reset link expires in 24 hours.",
            "password: <your password>",
            "api_key = ${API_KEY}",
            "password=changeme",
            "Set password_hint to something memorable.",
            "sk-short",
            "https://github.com/settings/tokens",
            "Ask in #secrets: what's the plan?",
            "git commit -m \"Rotate the API key\"",
            "The token count is 4096.",
        ];
        for text in plain {
            assert!(!looks_secret(text), "flagged {text:?}");
        }
    }

    #[test]
    fn fingerprint_ignores_line_endings_and_trailing_space() {
        assert_eq!(fingerprint("a\r\nb  \n"), fingerprint("a\nb"));
        assert_ne!(fingerprint("a\nb"), fingerprint("a b"));
        assert_ne!(fingerprint("  indented"), fingerprint("indented"));
    }

    #[test]
    fn exclusions_compare_names_loosely() {
        let excluded = vec!["KeePass.exe".to_owned(), "secret-tool".to_owned()];
        assert!(is_excluded("keepass.exe", &excluded));
        assert!(is_excluded("KEEPASS", &excluded));
        assert!(is_excluded("Secret-Tool.exe", &excluded));
        assert!(!is_excluded("KeePassXC.exe", &excluded));
    }

    #[test]
    fn previews_are_bounded() {
        let long = "x".repeat(10_000);
        assert_eq!(preview(&long).chars().count(), PREVIEW_CHARS);
        assert_eq!(preview("\r\n\r\nfirst\r\nsecond\tend"), "first\nsecond    end");
        assert_eq!(app_name("msedge.exe"), "Microsoft Edge");
        assert_eq!(app_name("someapp.exe"), "someapp");
    }

    #[test]
    fn relative_times_read_naturally() {
        let now = 1_000_000_000;
        assert_eq!(relative_time(now - 5, now), "just now");
        assert_eq!(relative_time(now - 300, now), "5 min ago");
        assert_eq!(relative_time(now - 7_200, now), "2 h ago");
        assert_eq!(relative_time(now - 90_000, now), "yesterday");
        assert_eq!(relative_time(now - 4 * 86_400, now), "4 days ago");
    }

    #[test]
    fn filters_select_the_right_entries() {
        assert!(Filter::Code.matches(Kind::Json, false, false));
        assert!(Filter::Code.matches(Kind::Command, false, true));
        assert!(!Filter::Code.matches(Kind::Url, false, false));
        assert!(Filter::Other.matches(Kind::Multiline, false, false));
        assert!(Filter::Recent.matches(Kind::Url, true, false));
        assert!(!Filter::Recent.matches(Kind::Url, false, true));
        assert!(Filter::Manual.matches(Kind::Text, false, true));
        assert!(Filter::Pinned.matches(Kind::Text, true, true));
        assert!(Filter::Images.matches(Kind::Image, false, false));
        assert!(!Filter::Images.matches(Kind::Text, false, false));
    }

    #[test]
    fn hex_color_detection() {
        assert_eq!(detect_hex_color("#6366F1"), Some("#6366F1"));
        assert_eq!(detect_hex_color("  #fff  "), Some("#fff"));
        assert_eq!(detect_hex_color("#AABBCCDD"), Some("#AABBCCDD"));
        assert_eq!(detect_hex_color("#abc"), Some("#abc"));
        assert_eq!(detect_hex_color("#GGG"), None);
        assert_eq!(detect_hex_color("not a color"), None);
        assert_eq!(detect_hex_color("#12"), None);
        assert_eq!(detect_hex_color("#12345"), None);
    }

    #[test]
    fn hex_color_normalization() {
        assert_eq!(normalize_hex_color("#fff"), Some("ffffff".to_owned()));
        assert_eq!(normalize_hex_color("#6366F1"), Some("6366F1".to_owned()));
        assert_eq!(normalize_hex_color("#abcd"), Some("aabbcc".to_owned()));
        assert_eq!(normalize_hex_color("#aabbccdd"), Some("aabbcc".to_owned()));
    }

    #[test]
    fn hex_rgb_parsing() {
        assert_eq!(parse_hex_rgb("#ff0000"), Some((255, 0, 0)));
        assert_eq!(parse_hex_rgb("#00ff00"), Some((0, 255, 0)));
        assert_eq!(parse_hex_rgb("#fff"), Some((255, 255, 255)));
    }

    #[test]
    fn domain_extraction() {
        assert_eq!(extract_domain("https://docs.google.com/spreadsheets/d/abc"), Some("docs.google.com".to_owned()));
        assert_eq!(extract_domain("http://example.com/path?q=1"), Some("example.com".to_owned()));
        assert_eq!(extract_domain("https://github.com"), Some("github.com".to_owned()));
        assert_eq!(extract_domain("www.rust-lang.org"), Some("rust-lang.org".to_owned()));
        assert_eq!(extract_domain("not a url"), None);
    }
}
