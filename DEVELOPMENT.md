# Development

## Prerequisites

- Windows 10 22H2 or Windows 11, x64.
- [Rust](https://rustup.rs) stable (1.88 or newer) with `rustfmt` and `clippy`.
- A C toolchain for the Windows resource compiler:
  - **Recommended:** the MSVC toolchain (`x86_64-pc-windows-msvc`) with the
    "Desktop development with C++" workload of Visual Studio Build Tools. This is what CI
    and release builds use.
  - **Also works:** the GNU toolchain (`x86_64-pc-windows-gnu`) with a 64-bit
    [MinGW-w64](https://www.mingw-w64.org) (for example WinLibs) first on `PATH`. The
    linker prints `.rsrc merge failure: multiple non-default manifests` because MinGW adds
    its own default manifest; Lilt's manifest is kept and the warning is harmless. If
    `windres` fails with "can't popen", check that the `COMSPEC` environment variable
    points to `C:\Windows\System32\cmd.exe`.
- [NSIS](https://nsis.sourceforge.io) 3.x, only to build the installer.

No Node.js, Python or other runtime is needed to build or run Lilt. Python with Pillow
is only used to regenerate the icons (`assets/icons/render_icons.py`).

## Build and run

```powershell
cargo run                    # debug build, with a console for log output
cargo build --release        # optimised build: target\release\lilt.exe
```

### Local development install

The `D:\Lilt Dev` worktree is the local test branch. After each update, run
`powershell -ExecutionPolicy Bypass -File .\tools\install-dev.ps1` there. The script
checks formatting, runs Clippy and tests, builds the complete release executable,
copies it to `%LOCALAPPDATA%\Programs\Lilt Dev`, and starts it. It creates a
**Lilt Dev** Start menu shortcut. The installed production Lilt remains separate.

Development settings, task and clipboard databases, and logs use `Lilt Dev`
folders under `%APPDATA%` and `%LOCALAPPDATA%`. The development build also has
its own single-instance guard and startup registry value. The two islands can
overlap on screen; quit either app from its notification-area icon if needed.
Shared Windows hotkeys can be held by only one running copy at a time.

When testing is complete, merge `dev/local-testing` into `main` only when the
user requests it. Normal release builds without `dev-build` keep the production
identity.

Only one instance runs per Windows session; starting another brings the running
island back and exits. Quit from the notification-area icon.

Logs are written to `%LOCALAPPDATA%\Lilt\logs\lilt.log`. Set `LILT_LOG=debug` (or
`trace`) before starting for more detail, for example which media session is followed.

Lilt reads its folders from `APPDATA` and `LOCALAPPDATA`, so pointing both at a scratch
directory runs it with separate settings and logs. Set `LILT_GLASS=frosted` or
`LILT_GLASS=blur` to force the glass appearances' material regardless of the Windows settings,
for example to exercise the live-blur window on a PC with transparency effects off
(the blur then renders black, which is what Windows draws in that case).

## Checks

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

`cargo test` includes headless UI tests (`src/snapshots.rs`) that render the real Slint
components with the software renderer. The images are written to
`target/ui-snapshots/`; look at them after changing the UI.

## Testing media integration

`tools/media-session` publishes a silent media session with metadata and coloured
artwork, so play/pause, track changes, seeking, artwork and ambient colours can be tested
without touching real media:

```powershell
cd tools\media-session
cargo run --release -- 30 4    # four 30-second tracks, repeating
```

It appears in Lilt (and in the Windows media flyout) as "media-session". Stop it with
Ctrl+C.

## Benchmarks

See [benchmarks/README.md](benchmarks/README.md). To measure the main states of a release
build:

```powershell
cargo build --release
.\benchmarks\scenarios.ps1 -Scenarios idle,expanded,toggle,media
```

Results are recorded in [PERFORMANCE.md](PERFORMANCE.md).

## Project layout

[ARCHITECTURE.md](ARCHITECTURE.md) describes the modules, threads and data flow. In short,
native Windows work lives in Rust under `src/`, all UI is Slint markup under `ui/`, and
the two communicate through the `AppState`, `AppActions` and `Theme` globals.

## Conventions

- Format with `rustfmt` (configuration in `rustfmt.toml`); keep `clippy` clean.
- Keep work off the UI thread only when it can block (media sessions, artwork). Do not
  add timers that run while their output is not visible.
- Prefer Windows APIs and the standard library over new dependencies; justify any new
  crate in the pull request.
- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org),
  for example `feat(media): …`, `fix(window): …`, `perf(ui): …`.
