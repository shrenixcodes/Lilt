# Releasing

Lilt is distributed as a per-user NSIS installer attached to GitHub Releases.

## Versioning

Versions follow [Semantic Versioning](https://semver.org). The version in `Cargo.toml`
is the single source: it is embedded in the executable, the installer and the uninstall
entry.

## Steps

1. Update `version` in `Cargo.toml` and run `cargo build` so `Cargo.lock` follows.
2. Move the entries under `[Unreleased]` in `CHANGELOG.md` into a new `## [x.y.z]`
   section (a date may follow: `## [x.y.z] - YYYY-MM-DD`).
3. Check the build locally:

   ```powershell
   cargo fmt --check
   cargo clippy --all-targets -- -D warnings
   cargo test
   cargo build --release
   .\installer\build.ps1
   ```

   Install the result on a clean Windows user account (or a virtual machine), start
   it from the Start menu, try the media controls with `tools/media-session`, then
   uninstall and confirm the program, shortcut and start-up entry are gone.
4. Commit, then tag and push:

   ```powershell
   git tag -a vx.y.z -m "Lilt x.y.z"
   git push origin main vx.y.z
   ```

5. The **Release** workflow checks that the tag matches `Cargo.toml`, runs lints and
   tests, builds the installer and creates a **draft** release. The installer is its only
   file; the notes are install instructions with the installer's SHA-256 checksum,
   followed by the changelog section. (GitHub adds source-code archives to every release
   by itself; they cannot be removed.)
6. Review the draft, download the installer from it, and publish it.

If the workflow fails after the tag is pushed (for example on the changelog section),
fix the problem on `main` and run **Release** by hand from the Actions tab, giving the
existing tag. The code is always checked out from the tag, so the installer still
matches the tagged commit; tags are never moved.

Nothing is published automatically: the workflow only creates drafts.

## Installer behaviour

- Installs to `%LOCALAPPDATA%\Programs\Lilt` by default without administrator rights;
  the folder can be changed.
- Optional components: a Start menu shortcut (on by default) and start at sign-in (off
  by default).
- Upgrades in place: a running copy is closed first, and an existing start-up entry is
  pointed at the new location.
- Registers an uninstaller in *Settings → Apps*. Uninstalling removes the program,
  shortcut and start-up entry, and offers to remove settings and logs.
- Requires 64-bit Windows 10 or later. No other runtime is needed: the C runtime is
  linked statically and the UI does not use a web view.

## Code signing

Builds are currently unsigned, so Windows SmartScreen may show a warning the first
time the installer runs. Signing can be added to the release workflow with a
certificate stored as an encrypted repository secret; certificates and keys must never
be committed. Lilt checks GitHub Releases for updates and verifies the downloaded installer against
the published SHA-256 checksum. This integrity check does not provide publisher
authentication; installers are currently unsigned.
