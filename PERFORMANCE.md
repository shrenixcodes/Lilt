# Performance

Lilt is meant to be almost invisible to the computer while idle. This document records
what was measured, how, and what has not been verified yet.

## Summary

| Target | Goal | Measured (release build) |
| --- | --- | --- |
| Idle memory | 60–100 MB | **6.3 MB** private working set (27 MB working set including shared system DLLs) |
| Active memory | below 130 MB | **6.4–6.6 MB** private working set while expanded or changing tracks |
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

## Results

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

The results above were measured with Midnight, before Liquid Glass existed. Midnight
still renders exactly as it did: its design tokens have their original values, the UI
tests confirm every Midnight screen is pixel-identical, and it creates no extra windows
or compositor objects.

Liquid Glass adds no loop or timer of its own. What it costs depends on its material:

| Material | Extra work in Lilt | Extra work elsewhere |
| --- | --- | --- |
| Frosted (transparency effects off, battery saver, low-power mode or *Reduce transparency*) | Translucent colours, plus a sheen and rim drawn only when the island redraws | None |
| Live blur | One hidden window and one composition visual; the clip is updated only while the island's outline changes | DWM blurs the desktop behind the island on the GPU, as it does for the Start menu |

In an isolated prototype on the test machine, creating the compositor, the window and
the visual added 1.2 MB of private working set and used no CPU while idle. The compositor
is created only when the live blur is first needed.

A side-by-side benchmark of both appearances has not been recorded yet. Two attempts
were disturbed because the test PC was in use during the sampling window (input over the
island), so their numbers are not published here. DWM's GPU cost for the blur could not
be measured on the test machine either: Windows' transparency effects are off there, so
DWM draws no blur.

## Sampling policy

| Data | Compact | Expanded | Low-power mode |
| --- | --- | --- | --- |
| Clock | every minute | every minute | every minute |
| CPU | not sampled | every 2 s | every 5 s |
| Memory | not sampled | every 5 s | every 10 s |
| Network | not sampled | every 1 s | every 5 s |
| Playback position | not updated | every 1 s while playing | every 1 s while playing |
| Playing bars | about 14 steps/s while playing and on screen | not drawn | about 14 steps/s while playing |
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
- Midnight and Liquid Glass side by side in every scenario, and DWM's GPU cost for the
  live blur on a PC with transparency effects on.
- Windows Performance Recorder traces (wake-up sources, frame timing) and behaviour on
  the reference low-end configurations above.
- Interaction with an unlocked desktop (pointer hover, real clicks) and alongside a
  running game; measured CPU for animations suggests the impact is negligible, but this
  has not been verified with a game running.
