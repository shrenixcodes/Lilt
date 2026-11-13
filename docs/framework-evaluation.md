# UI framework evaluation

Lilt is a small always-visible overlay, so the cost of the UI stack is paid continuously
rather than only while someone uses the app. Before committing to a framework, four
candidates were built as the same minimal floating window and measured as release
builds on Windows.

**Decision:** Slint's software renderer driven by a small custom Win32 platform layer
(a layered, top-most, non-activating window), with native Windows features accessed
through `windows-rs`.

## Candidates

| Candidate | What was built |
| --- | --- |
| Slint 1.18 + winit, femtovg (OpenGL) renderer | Slint's default desktop backend |
| Slint 1.18 + winit, software renderer | Same backend, CPU rendering through `softbuffer` |
| Slint 1.18 software renderer + custom Win32 window | Slint renders into a DIB section presented with `UpdateLayeredWindow` |
| egui / eframe 0.37 with glow (OpenGL) | Immediate-mode UI, repainting only on input or request |
| Tauri 2.12 + WebView2 | Plain HTML/CSS/JS front end (no framework, so a lower bound for a React build) |

A native Win32/WinRT UI written directly against Direct2D/DirectWrite/DirectComposition
was assessed on paper only: it would have the smallest possible footprint but requires
building layout, text, animation and widgets from scratch. The custom-platform Slint
variant keeps most of that advantage (one process, no GPU, a native window) while
reusing a maintained declarative UI toolkit.

Every proof of concept implemented the same workload: a 420 × 220 logical-pixel
transparent, borderless, always-on-top window at the top centre of the primary display,
hidden from the taskbar, containing a near-black pill with a clock that updates once a
minute and a hover state. A test switch expanded and collapsed the pill every two
seconds with a 320 ms size animation.

## Test environment

| | |
| --- | --- |
| CPU | Intel Core i5-12450H (8 cores, 12 threads) |
| Memory | 7.7 GB |
| Graphics | Intel UHD Graphics (driver 31.0.101.4502) and NVIDIA GeForce GTX 1650 |
| Display | 1920 × 1080, 100 % scaling, single monitor |
| OS | Windows 11 Home, build 26200 |
| WebView2 runtime | 157.0.4341.46 |
| Toolchain | Rust 1.98.0, `x86_64-pc-windows-gnu`, MinGW-w64 GCC 16.1 |
| Profile | `--release`, thin LTO, symbols stripped |

The machine was in normal use during measurement (a browser playing video, a chat
client and this repository's tooling were running). Results are therefore comparative
rather than absolute, and per-process metrics were used so that other applications do
not contribute to the numbers.

## Method

Measurements were taken with [`benchmarks/measure.ps1`](../benchmarks/measure.ps1),
which launches an executable, follows its entire process tree (including WebView2 helper
processes), waits 10 s for start-up work to settle and then samples for 60 s:

- **Private working set** - the Task Manager "Memory" column, summed over the tree.
- **Working set** - includes shared pages such as system DLLs and graphics drivers.
- **Private bytes** - committed private memory.
- **CPU** - CPU time consumed during the window as a share of all 12 logical processors;
  peak is the highest 2-second interval.
- **Context switches per second** - a proxy for wake-ups.
- **GPU** - sum of the `GPU Engine` utilisation counters for the tree's processes.
- **Disk I/O** - bytes read and written during the window.
- **Network** - remote TCP endpoints owned by the tree at any sample.

Each idle scenario was run three times. The pointer was in use near the top of the
screen during some runs, which wakes windows that receive mouse input; the lowest
idle CPU of the three runs is reported alongside the first run's memory figures.

## Results

### Idle (compact pill, clock only)

| Variant | Processes | Private WS | Working set | Private bytes | CPU (best of 3) | Start-up to window | Disk written / min | Network |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| **Slint software + Win32 layered** | 1 | **1.4 MB** | **10.8 MB** | **1.8 MB** | **0.006 %** | 136-187 ms | 0 | none |
| Slint software + winit | 1 | 3.0 MB | 20.7 MB | 3.8 MB | 0.000 % | 147-172 ms | 0-1 KB | none |
| Slint femtovg + winit | 1 | 69-90 MB | 122-143 MB | 93-111 MB | 0.016 % | 267-311 ms | 0 | none |
| egui glow | 1 | 69-70 MB | 119-121 MB | 91-92 MB | 0.057 % | 151-209 ms | 0 | none |
| Tauri / WebView2 | 7 | 90-94 MB | 343-350 MB | 154-159 MB | 0.022 % | 205-238 ms | 79 KB - 8.9 MB | 1-3 Microsoft endpoints per run |

### Expanding and collapsing every two seconds

| Variant | Private WS | CPU (of all cores) | CPU (of one core) | Peak | GPU |
| --- | ---: | ---: | ---: | ---: | ---: |
| **Slint software + Win32 layered** | **1.4 MB** | **0.05 %** | **0.6 %** | 0.44 % | 0 % |
| Slint software + winit | 3.1 MB | 0.15 % | 1.8 % | 0.38 % | 0 % |
| Slint femtovg + winit | 70 MB | 1.48 % | 17.7 % | 2.89 % | 2.1 % |
| egui glow | 76 MB | 1.51 % | 18.1 % | 2.78 % | 1.1 % |
| Tauri / WebView2 | 111 MB | 0.87 % | 10.5 % | 2.25 % | 1.5 % |

### Build footprint

| Variant | Crates compiled for Windows | Executable |
| --- | ---: | ---: |
| Slint software + Win32 layered | 157 | 7.9 MB |
| Slint (femtovg + software) + winit | 240 | 11.6 MB |
| egui glow | 108 | 6.8 MB |
| Tauri (no front-end tooling) | 215 | 6.9 MB + WebView2 runtime |

A Tauri build with React and Vite would additionally require a Node.js toolchain and its
npm dependency tree at build time.

## Observations

- **OpenGL dominates memory.** Both GL-based variants spend 65-85 MB on the graphics
  driver and context alone, before any application data exists. On integrated graphics
  this memory is taken from system RAM.
- **WebView2 is a browser.** The Tauri build runs six `msedgewebview2.exe` helper
  processes, writes to its profile directory while idle, and opened connections to
  Microsoft endpoints in every run even though the page was entirely local. That
  conflicts with Lilt's "no network traffic" requirement unless WebView2 features are
  individually disabled, and the process-tree memory stays near 90 MB regardless.
- **Software rendering is cheap at this size.** The island is a few hundred thousand
  pixels; Slint redraws only the changed region, so animations cost well under 1 % of one
  core and nothing runs between them.
- **Transparent windows must not receive input.** A normal transparent window still
  captures clicks and pointer movement over its invisible area, which both woke the
  winit variants during measurement and would block clicks on browser tabs beneath the
  island. A layered window passes input through fully transparent pixels natively.
- **GPU contention.** The software-rendered variants never touch the GPU, so they cannot
  compete with a game for GPU time or force a discrete GPU to wake up.

## Qualitative comparison

| | Slint + Win32 platform | Slint + winit | egui | Tauri 2 |
| --- | --- | --- | --- | --- |
| Transparency | Per-pixel via layered window | Per-pixel via DWM | Per-pixel via DWM (needed extra work to remove a frame) | Per-pixel via WebView2 |
| Click-through outside the shape | Native | Requires window regions | Requires window regions | Requires window regions |
| Always on top, no activation, no taskbar entry | Native styles | Attributes plus Win32 tweaks | Attributes plus Win32 tweaks | Config options plus Win32 tweaks |
| DPI awareness | Per-monitor v2, handled in Lilt | Handled by winit | Handled by winit | Handled by Tauri |
| Animation model | Declarative, property animations, frames only while animating | Same | Immediate mode, repaints on request | CSS transitions in a browser engine |
| Windows API access | Direct, same thread | Through window handle | Through window handle | Rust side, separate from UI process |
| Accessibility | None in the first release (see trade-offs) | AccessKit / UI Automation | AccessKit | Browser accessibility |
| Licence | GPL-3.0 or Slint Royalty-free 2.0 or commercial | Same | MIT / Apache-2.0 | MIT / Apache-2.0 |
| Latest release checked | 1.18.1 (2026-09-21) | 1.18.1 | 0.37.0 (2026-10-15) | 2.12.5 (2026-11-06) |

## Licensing

Slint is offered under GPL-3.0, the *Slint Royalty-free Desktop, Mobile, and Web
Applications License 2.0*, or a commercial licence. The royalty-free licence permits
distributing Slint as part of a desktop application, modified or not, provided the
application either shows Slint's `AboutSlint` widget in an About screen reachable from
its top-level menu or displays the "Made with Slint" badge on a public page such as the
download page. It does not allow embedded-systems use, distributing Slint on its own, or
exposing Slint's APIs. Lilt is a desktop application, shows `AboutSlint` in
**Settings → About**, and credits Slint in its README, so its own source code can be
published under a permissive licence while it uses Slint under the royalty-free terms.

## Trade-offs of the chosen approach

- **Accessibility.** The custom platform does not yet expose a UI Automation tree, so
  screen readers cannot read the island. The notification-area menu is a native menu and
  remains accessible, and every action in the island is also available from Windows
  itself. Adding UI Automation support is tracked as follow-up work.
- **Platform code to maintain.** About 600 lines of Win32 code replace winit: window
  creation, input translation, DPI changes and the message loop. It is limited to what
  Lilt needs (no IME or text editing; the settings window has no text fields).
- **Cursor shapes.** Slint's public platform API does not report cursor changes, so the
  pointer stays an arrow over buttons.

## Dependency audit

| Dependency | Responsibility | Essential | Licence | Notes |
| --- | --- | --- | --- | --- |
| `slint` (software renderer only) | Declarative UI, layout, text, animation | Yes | GPL-3.0 / Royalty-free 2.0 / commercial | No backend, no GPU renderer enabled |
| `windows` 0.62 | GSMTC media sessions, Win32 windowing, time zones, registry, shell | Yes | MIT / Apache-2.0 | Microsoft-maintained; feature-gated bindings |
| `sysinfo` 0.40 | CPU, memory and network counters | Yes (per requirements) | MIT | Only the `system` and `network` features |
| `image` 0.25 | Decoding JPEG/PNG/BMP album art and resizing it | Yes | MIT / Apache-2.0 | Default features disabled; formats chosen explicitly |
| `serde`, `serde_json` | Versioned settings file | Yes | MIT / Apache-2.0 | |
| `tracing` | Structured diagnostics | Yes | MIT | Writes to a small local log only |
| `tokio` | Async runtime | **No** | - | WinRT operations are awaited synchronously on one worker thread |
| `colorthief` | Dominant colours | **No** | - | First released May 2026, very little use; replaced by a tested ~150-line median-cut extractor |
| `chrono` | Dates and times | **No** | - | Calendar arithmetic is a small tested module; time-zone conversion uses the Windows time-zone database |

Transitive dependency counts and binary size for the finished application are recorded
in [`PERFORMANCE.md`](../PERFORMANCE.md).

## Reproducing

The proof-of-concept sources are not part of the repository; each was a single
`main.rs` implementing the workload above, with environment variables selecting the
renderer and enabling the expand/collapse cycle. The same measurement can be repeated
for any build:

```powershell
.\benchmarks\measure.ps1 -Exe path\to\app.exe -Label idle -SampleSeconds 60
```
