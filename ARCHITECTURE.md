# Architecture

Lilt is a single Windows process with one UI thread and one worker thread. The UI is
written in [Slint](https://slint.dev) and drawn by Slint's software renderer into memory
that Lilt presents with plain Win32 calls; native features are reached through
[`windows-rs`](https://github.com/microsoft/windows-rs). The reasoning behind these
choices, with measurements, is in [docs/framework-evaluation.md](docs/framework-evaluation.md).

## Threads and data flow

```mermaid
flowchart LR
    subgraph OS[Windows]
        GSMTC[Media sessions<br/>GSMTC]
        Shell[Shell notifications<br/>full-screen, hot key,<br/>display changes]
        Input[Mouse and keyboard]
    end

    subgraph Worker[Media thread]
        Tracker[Session tracker] --> Artwork[Artwork decode<br/>and cache]
        Artwork --> Palette[Accent palette]
    end

    subgraph UI[UI thread]
        Loop[Win32 message loop] --> App[App controller]
        App --> State[(AppState and Theme<br/>Slint globals)]
        State --> Render[Software renderer]
        Render --> Layered[Layered island window]
        App --> Sysinfo[System monitor<br/>sysinfo]
        App --> Calendar[Calendar file<br/>and agenda]
        App --> Settings[(settings.json)]
    end

    GSMTC -- change events --> Tracker
    Tracker -- updates --> Loop
    App -- commands --> Tracker
    Shell --> Loop
    Input --> Loop
```

**UI thread.** Runs the message loop, every Slint timer, rendering, the system monitor,
calendar parsing and settings. It sleeps in `MsgWaitForMultipleObjectsEx` until input
arrives, a timer is due, another thread posts work, or an animation needs its next
frame. Nothing ticks while the island is idle.

**Media thread.** Owns the Global System Media Transport Controls session manager.
WinRT raises change events on thread-pool threads; their handlers only send a message
to this thread, which reads the new state, decodes artwork, extracts its palette and
posts the result to the UI thread. Between events it blocks on a channel.

There is no async runtime: the few WinRT operations are awaited synchronously on the
media thread.

## Source layout

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | Logging, single-instance guard, start-up |
| `src/window/` | The Slint platform: event loop, Win32 windows, input translation, monitor placement |
| `src/app/` | The controller connecting services, settings and UI (`media`, `system`, `calendar`, `preferences`) |
| `src/media/` | Media session tracking, session selection policy and transport commands |
| `src/artwork/` | Artwork decoding, cropping, rounding and caching |
| `src/ambient/` | Colour science (OKLab/OKLCH, contrast) and palette extraction |
| `src/system/` | CPU, memory and network readings and the sampling policy |
| `src/calendar/` | ICS parsing, recurrence expansion, time zones and the agenda |
| `src/settings/` | Versioned settings document, validation and atomic saving |
| `src/clock.rs`, `src/hotkey.rs`, `src/startup.rs`, `src/actions.rs`, `src/dialogs.rs` | Clock text, global shortcut, start-with-Windows, quick actions, file dialog |
| `src/snapshots.rs` | Headless UI tests |
| `ui/` | Slint markup: design tokens, island, views, settings window, tray |
| `benchmarks/` | Process-tree resource measurement |
| `tools/media-session/` | A silent test media session for development |

## The window platform

Slint normally renders through a windowing backend. Lilt instead implements Slint's
`Platform` and `WindowAdapter` traits directly (`src/window/`):

- **Island.** A `WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE`
  popup. Slint renders premultiplied BGRA pixels straight into a DIB section, which is
  presented with `UpdateLayeredWindowIndirect`, passing only the changed rectangle. Fully
  transparent pixels pass mouse input through to the windows underneath, so the window
  can be sized for the largest state without blocking anything. Clicking does not take
  focus; expanding the island takes it explicitly so Escape and outside clicks work, and
  collapsing returns it.
- **Settings window.** An ordinary top-level window that blits the same kind of buffer
  in `WM_PAINT`, with a dark title bar. It is created when opened and destroyed when
  closed.
- **Control window.** A hidden top-level window that receives cross-thread wake-ups,
  the global hot key, the shell's full-screen notifications, display and setting
  changes, and the "show yourself" message from a second instance.
- **DPI.** The process is per-monitor DPI aware (manifest, with a runtime fallback).
  `WM_DPICHANGED` updates Slint's scale factor and the window size.

## Placement

`src/window/placement.rs` enumerates monitors with their work areas and effective DPI.
The island's position is stored as a monitor device name, a horizontal offset from the
work area's centre and a top margin, all in logical pixels, so it survives resolution
and scaling changes. It is recomputed on display, work-area and DPI changes. Dragging
uses the native move loop with the top edge held fixed.

## Update policies

| Data | Mechanism |
| --- | --- |
| Clock | One timer aimed at the next minute boundary; also refreshed on time, time-zone and regional-format changes and on resume |
| Media metadata and playback state | WinRT change events, coalesced for 150 ms; commands run immediately |
| Artwork | Read only when the track changes (or, if it had none, when its properties change); decoded once and cached by content hash |
| Accent palette | Computed once per new artwork on the media thread |
| Playback progress | Extrapolated from the session timeline; refreshed once a second only while expanded and playing |
| CPU / memory / network | 2 s / 5 s / 1 s, only while the expanded island is visible; 5 s / 10 s / 5 s in low-power mode; off otherwise |
| Calendar | Rebuilt when the island expands and each minute while expanded; the file is re-read only if its modification time changed |
| Full-screen state | Shell app-bar notification, plus one query at start-up |

## Media session selection

When several applications expose sessions, Lilt keeps following the session it shows
until it closes, the user picks another source, or another session *starts* playing.
It never switches because the followed session was paused, which would make the next
click on the same button control a different application. The policy is a pure function
(`media::session::choose`) with unit tests.

## Ambient colour

Artwork is reduced to 32 × 32 pixels and grouped with median cut. Candidates are scored
by how vivid and how common they are, de-emphasising near-black and near-white areas.
The winning hue is kept while lightness and chroma are clamped in OKLCH, and the accent
is lightened until it reaches 4.5:1 contrast with the island surface. The UI uses it for
a radial glow behind the artwork, a faint tint of the expanded surface and accents, and
cross-fades to new colours over 700 ms; otherwise nothing moves.

The software renderer draws gradients and clipped children as rectangles, ignoring
corner radii. Effects are therefore kept inside the rounded shape: the glow fades out
before the panel's edge, the tint is a solid colour of the panel itself, and album
artwork has its rounded corners applied to the pixels when it is decoded.

## Design tokens

All colours, sizes, radii, type sizes and durations come from the `Theme` global in
`ui/theme.slint`. Ambient colours and the motion factor are the only tokens changed at
run time. Additional appearances can be added by providing another set of token values
without changing the views.

## Persistence and files

| File | Location | Written |
| --- | --- | --- |
| Settings | `%APPDATA%\Lilt\settings.json` | Shortly after a change, atomically |
| Log | `%LOCALAPPDATA%\Lilt\logs\lilt.log` | On notable events; rotated at 512 KB |
| Start with Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\Lilt` | When toggled |

## Testing

- Unit tests cover parsing, colour extraction, date arithmetic, time-zone conversion,
  settings validation and migration, placement geometry, shortcut handling and the
  session selection policy.
- `src/snapshots.rs` renders the real UI components headlessly with a controllable clock
  and checks layout, artwork, ambient colours and scaling; images are written to
  `target/ui-snapshots/`.
- `tools/media-session` provides a deterministic media session for manual and benchmark
  runs, and `benchmarks/` measures the running application.
