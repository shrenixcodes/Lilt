use std::path::{Path, PathBuf};

fn main() {
    embed_windows_resources();
}

/// Embeds the application icon, version information and the application manifest
/// (per-monitor DPI awareness, supported OS versions, `asInvoker`).
fn embed_windows_resources() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("set by Cargo"));
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("set by Cargo"));
    let icon = root.join("assets/icons/lilt.ico");
    let manifest = root.join("assets/windows/lilt.manifest");
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed={}", manifest.display());

    let version = std::env::var("CARGO_PKG_VERSION").expect("set by Cargo");
    let numbers: Vec<u16> = version.split(['.', '-', '+']).take(3).map(|part| part.parse().unwrap_or(0)).collect();
    let [major, minor, patch] = [numbers[0], numbers[1], numbers[2]];

    // Generated so the version strings come from Cargo.toml. Numeric constants are used
    // instead of <winver.h> so the file compiles with both rc.exe and windres.
    let script = format!(
        r#"1 ICON "{icon}"
1 24 "{manifest}"

1 VERSIONINFO
FILEVERSION {major},{minor},{patch},0
PRODUCTVERSION {major},{minor},{patch},0
FILEFLAGSMASK 0x3F
FILEFLAGS 0x0
FILEOS 0x40004
FILETYPE 0x1
FILESUBTYPE 0x0
BEGIN
    BLOCK "StringFileInfo"
    BEGIN
        BLOCK "040904B0"
        BEGIN
            VALUE "CompanyName", "Shreyan Bag"
            VALUE "FileDescription", "Lilt"
            VALUE "FileVersion", "{version}"
            VALUE "InternalName", "lilt"
            VALUE "LegalCopyright", "Copyright (c) 2026 Shreyan Bag"
            VALUE "OriginalFilename", "lilt.exe"
            VALUE "ProductName", "Lilt"
            VALUE "ProductVersion", "{version}"
        END
    END
    BLOCK "VarFileInfo"
    BEGIN
        VALUE "Translation", 0x409, 1200
    END
END
"#,
        icon = rc_path(&icon),
        manifest = rc_path(&manifest),
    );
    let rc = out.join("lilt.rc");
    std::fs::write(&rc, script).expect("write resource script");
    embed_resource::compile(&rc, embed_resource::NONE).manifest_required().expect("compile Windows resources");
}

fn rc_path(path: &Path) -> String {
    path.display().to_string().replace('\\', "/")
}
