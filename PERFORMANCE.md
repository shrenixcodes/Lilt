# Performance

Lilt is meant to be almost invisible to the computer while idle. This document records
what was measured, how, and what has not been verified yet.

## Summary

| Target | Goal | Measured (release build) |
| --- | --- | --- |
| Idle memory | below 8 MB | **3.0 MB** private working set in Midnight, 3.2 MB in Liquid Glass (24–25 MB working set including shared system DLLs) |
| Active memory | below 8 MB | **3.5–3.6 MB** private working set while expanded with the tasks box, or while music plays |
| Idle CPU | below 0.5 % | **0.000 %** over 120 s |
| Interaction CPU | below 2 % | **0.021 %** of all cores (0.25 % of one core) while expanding and collapsing every 2 s |
| Rendering loop | none while idle | 0.1 context switches per second while idle |
| GPU | no heavy use | **0 %** in every scenario; Lilt renders on the CPU |
| Network | none | **No connections** in any scenario |
| Disk | minimal | **0 KB** read or written during every sampling window |
| Start-up | under 3 s | **0.38–0.51 s** to a visible island |

## Environment

| | |
| --- | --- |
| Machine | Laptop, Intel Core i5-12450H (8 cores, 12 threads), 7.7 GB RAM |
| Graphics | Intel UHD Graphics and NVIDIA GeForce GTX 1650 (unused by Lilt) |
| Display | 1920 × 1080 at 100 % |
| OS | Windows 11 Home, build 26200 |
| Build | `cargo build --release` (LTO, one codegen unit), Rust 1.98, GNU toolchain |
| Date | November 2026 |

The Windows session was locked while these runs took place, so no pointer or keyboard
input reached Lilt. Expansion was triggered by posting Lilt's global-shortcut message.
Other applications (a browser, a chat client) were running.

## Method

[`benchmarks/measure.ps1`](benchmarks/measure.ps1) starts the executable, records the
time to its first visible window, waits 10 s and then samples the whole process tree
every 2 s. [`benchmarks/scenarios.ps1`](benchmarks/scenarios.ps1) drives the scenarios:

| Scenario | What happens | Length |
| --- | --- | --- |
| idle | Compact island showing the clock | 120 s |
| expanded | Expanded dashboard: clock, agenda, system readings (CPU every 2 s, memory every 5 s, network every 1 s) | 120 s |
| toggle | Expanded and collapsed every 2 s (60 size animations) | 120 s |
| media | `tools/media-session` changes track and artwork every 5 s | 120 s |
| soak | Compact island, sampled every 10 s | 30 min |

Reported values: private working set (Task Manager's "Memory" column), working set,
private bytes, CPU time as a share of all 12 logical processors and of one, the peak
2-second interval, context switches per second (wake-ups), GPU engine utilisation, I/O
and remote TCP endpoints.

## Current results

Release build of November 2026, after the system readings stopped using the Performance
Data Helper (see [Memory](#memory)). Each scenario ran for 30 s after a 10 s settle,
with separate settings and data, the tasks box enabled and nothing else playing media.

| Scenario | Private WS | Working set | Private bytes | CPU (all cores) | CPU (one core) | Wake-ups/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Midnight, compact, idle | 3.0 MB | 23.6 MB | 4.0 MB | 0.000 % | 0.000 % | 0.0 |
| Liquid Glass, compact, idle | 3.2 MB | 25.3 MB | 4.1 MB | 0.000 % | 0.000 % | 0.1 |
| Liquid Glass, compact, music playing (bars moving) | 3.5 MB | 26.5 MB | 4.6 MB | 0.041 % | 0.49 % | 24.6 |
| Midnight, expanded with the tasks box, readings live | 3.5 MB | 26.0 MB | 4.4 MB | 0.022 % | 0.26 % | 58.2 |
| Liquid Glass, expanded with the tasks box, readings live | 3.6 MB | 29.7 MB | 4.6 MB | 0.047 % | 0.56 % | 55.8 |

GPU utilisation and network connections were zero in every row.

These rows used the earlier composition blur for Liquid Glass. With the backdrop drawn
by Lilt (see [Appearances](#appearances)), Liquid Glass measured 3.1 MB compact and idle
and 3.6–3.8 MB expanded with the tasks box at the default 50 % transparency (0.08–0.2 %
CPU while its readings are live). At 100 % (clear glass, a full-resolution copy of the
screen behind the window) it measured 3.9 MB compact and 4.5 MB expanded.
Those runs were taken while the PC was in use; runs in which the pointer crossed the
island (visible as hundreds of wake-ups per second from hover redraws) are not reported.

### Memory

Task Manager's "Memory" column is the private working set. Measuring it after each
start-up step showed where it went before this change:

| Step | Private working set |
| --- | ---: |
| Process start, logging, COM, the window platform, island and tray created | 1.5 MB |
| System readings created (`sysinfo`, whose CPU usage uses the Performance Data Helper) | **+4.1 MB** |
| Island shown: its 890 × 340 drawing surface (1.2 MB) and, in the glass appearances, the blur | +1.3–1.5 MB |
| Expanded with the tasks box (SQLite, rows, readings) | +0.5–0.7 MB |

Lilt's own Rust heap was only about 0.5 MB of this. The readings now come straight from
Windows (`GetSystemTimes`, `GlobalMemoryStatusEx`, `GetIfTable2`), which removed the
4.1 MB and three crates. Before the change the same scenarios measured 7.6–8.3 MB.

## Earlier results

Measured before Tasks, the glass appearances and the change above, over 120 s per
scenario.

| Scenario | Private WS | Working set | Private bytes | CPU (all cores) | CPU (one core) | Peak | Wake-ups/s |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| idle | 6.3 MB | 27.1 MB | 7.6 MB | 0.000 % | 0.000 % | 0.000 % | 0.1 |
| expanded | 6.4 MB | 27.7 MB | 8.3 MB | 0.001 % | 0.013 % | 0.043 % | 2.2 |
| toggle every 2 s | 6.5 MB | 27.3 MB | 8.0 MB | 0.021 % | 0.246 % | 0.167 % | 30.9 |
| track change every 5 s | 6.6 MB | 27.4 MB | 8.3 MB | 0.004 % | 0.051 % | 0.064 % | 6.2 |
| soak, 30 min | 2.0 MB (max 6.2 MB) | 14.3 MB | 8.0 MB | 0.000 % | 0.000 % | 0.000 % | 0.0 |

GPU utilisation, disk I/O and network connections were zero in every row.

Over the 30-minute soak, committed memory (private bytes) stayed at about 8 MB, the same
as in the two-minute runs, so nothing accumulates while idle. The lower average working
set reflects Windows trimming the memory of a process that is not doing anything.

For comparison, the same measurements of minimal prototypes in other UI stacks are in
[docs/framework-evaluation.md](docs/framework-evaluation.md): about 70 MB for OpenGL-based
toolkits and about 90 MB across seven processes for a WebView2 (Tauri) window.

## Why it is small

- **No web view and no GPU context.** Slint's software renderer draws into a bitmap that
  a layered window presents. Only the changed rectangle is redrawn and uploaded.
- **No idle loop.** The message loop sleeps until input, a timer, posted work or an
  animation frame is due. Animations run at up to 60 frames per second only while
  they last.
- **Event-driven media.** Windows media session changes arrive as events on a worker
  thread that otherwise blocks. Artwork is read and decoded once per track and cached.
- **Timers only for visible data.** The clock fires once a minute. CPU, memory and
  network are sampled only while the expanded island is visible, and playback progress
  updates once a second only while expanded and playing.
- **Lazy views.** The expanded dashboard, detail pages and settings window are created
  when shown and released when hidden.
- **Hidden during full-screen apps.** Games and videos get the whole machine.

## Appearances

Midnight still renders exactly as it did: its design tokens have their original values,
the UI tests confirm every Midnight screen is pixel-identical, and it creates no extra
windows or compositor objects.

The glass appearances read the screen behind the island (at one eighth of its
resolution) when it opens, closes or moves, when another window comes to the front,
moves or scrolls, and every 3 s while the island is open; nothing while it sits compact
with no window activity. What the glass costs depends on its material:

| Material | Extra work in Lilt | Extra work elsewhere |
| --- | --- | --- |
| Frosted (transparency effects off, battery saver, low-power mode or *Reduce transparency*) | Translucent colours, plus a sheen and rim drawn only when the island redraws | None |
| Blur | A screen read and a blur of a small image on the events above; blending it beneath the glass surfaces in the pixels of each redrawn region | None |

## Sampling policy

| Data | Compact | Expanded | Low-power mode |
| --- | --- | --- | --- |
| Clock | every minute | every minute | every minute |
| CPU | not sampled | every 2 s | every 5 s |
| Memory | not sampled | every 5 s | every 10 s |
| Network | not sampled | every 1 s | every 5 s |
| Playback position | not updated | every 1 s while playing | every 1 s while playing |
| Playing bars | about 14 steps/s while playing and on screen | not drawn | about 14 steps/s while playing |
| Glass backdrop | on window changes | on window changes and every 3 s | not read (frosted) |
| Media metadata | Windows events | Windows events | Windows events |
| Calendar | not refreshed | on expand and every minute (file re-read only if changed) | same |

The compact island shows no system readings, so none are collected while it is compact,
hidden or behind a full-screen application.

## Low-end compatibility

| Environment | Status |
| --- | --- |
| Windows 11 x64 | Tested (build 26200) |
| Windows 10 22H2 x64 | Not yet tested; APIs used are available since Windows 10 1903 |
| 4 GB RAM | Not tested; Lilt's footprint is under 10 MB of private memory |
| Dual-core CPU | Not tested; Lilt's work is single-threaded except media events |
| Integrated graphics | No GPU use in any scenario |
| Low-resolution display | Placement is resolution independent; not tested below 1920 × 1080 |
| High-DPI display | Per-monitor DPI aware; UI tests render at 100 % and 150 % |
| Multiple monitors | Placement logic is unit tested; not tested on a multi-monitor setup |

## Not yet measured

- The tasks box: memory with a large list and archive. The database is opened only when
  the box is first shown, so people who do not use Tasks pay nothing for it.
- The glass appearances with music playing and while toggling, measured on an idle PC,
  the cost of each screen read, and Smoked Glass in every scenario.
- The settings window: it is created only while open and released when closed.
- Windows Performance Recorder traces (wake-up sources, frame timing) and behaviour on
  the reference low-end configurations above.
- Interaction with an unlocked desktop (pointer hover, real clicks) and alongside a
  running game; measured CPU for animations suggests the impact is negligible, but this
  has not been verified with a game running.
