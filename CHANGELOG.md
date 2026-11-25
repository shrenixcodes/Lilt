# Changelog

All notable changes to Lilt are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.6.2] - 2026-11-25

### Changed

- Reordered Settings sections so `Clock & weather` sits before `About`, wrapped clipboard filter chips onto two rows, and allowed vertical scrolling in the expanded Calendar agenda.
- Toggling the Brightness sun icon switches between dimmed (`15 %`) and previous brightness levels, and media progress bars support continuous drag-to-seek scrubbing.

### Fixed

- Preserved `1 px` glass borders when UI hue gradient is disabled and scaled live backdrop compositing with window fade opacity to eliminate opaque fade pops.
- Enforced the Compact App Launcher toggle across middle-clicks, wheel scrolling, and launches; resolved `Code.exe` and quoted launch targets cleanly; smoothed precision touchpad launcher scrolling; and allowed dragging across the compact pill without inner targets swallowing gestures.
- Fixed Settings drag-and-drop shortcut reordering breaking mid-drag, blocked `Alt`/`F10` system-key activation during shortcut capture alongside `F13..F24` and punctuation support, aligned Settings label text contrast tokens, and made native file pickers modal to Settings.
- Preserved full-resolution PNG captures for clipboard image re-pasting, cached decoded thumbnails, corrected `CF_DIBV5` pixel data offsets, prevented snippet editor activation on images, debounced clipboard search filtering, and updated clipboard privacy copy in Settings, intro, and `SECURITY.md`.
- Split audio commands onto a dedicated worker thread so slow Bluetooth/Wi-Fi/WMI scans never stall volume or mute updates, cached mixer app icons, resolved indirect audio endpoint/session names, preserved in-progress Wi-Fi passwords across network refreshes, and cleaned up superseded temporary Wi-Fi profiles.
- Fixed expanded view padding clicks collapsing the island, Escape/arrow/Enter key hijacking across inputs and overlays, click-outside dismissal on Tasks priority filters, Archive search/volume box focus tracking, and compact progress bar edge clearance.
- Scoped full-screen hiding to Lilt's active monitor and ignored null-foreground busy states, aligned auto-hide wake zones and tuck travel with top-docked taskbars, paused off-screen visualizers, marquees, and disabled System monitor polling, kept ended media visible during grace in Expanded View, cycled individual browser tab media sessions, fixed cross-timezone ICS event durations and midnight task counters, removed unnecessary elevation on per-user auto-updates, and refused updates when checksum download or parsing fails.

## [0.5.5] - 2026-11-18

### Added

- Drag grip handles and reorder controls (move up and down) for collapsed island shortcuts in Settings.
- Responsive layout constraint (`max-width: 840px`) for Settings cards to prevent excessive stretching on widescreen and full-screen displays.
- Streamlined two-tier shortcut addition UI in Settings with an expansive path input bar and quick browse button.

### Changed

- Refined CPU and RAM telemetry meters into slender 4px rounded capsule tracks with matching subtle fills and clean typography.
- Vertically centered the compact island app launcher item and removed the redundant click-to-launch hint.
- Protected Settings row labels with guaranteed minimum widths to eliminate text squeezing on narrow viewports.

### Fixed

- Resolved jagged alignment and floating center-cluster in the collapsed island shortcuts settings list.

## [0.5.4] - 2026-11-18

### Added

- Native file browse dialog for adding custom application shortcuts (`.exe`, `.lnk`, `.url`, `.cmd`, `.bat`) with automatic display name and icon extraction.
- Clear and reset controls for the application shortcuts list in Settings, allowing default detected shortcuts to be removed or cleared completely.
- Return-to-resting loop in the collapsed island launcher: scrolling cycles through all shortcuts and seamlessly returns to the resting music/idle view before continuing the loop.
- Descriptive instructions for compact island scroll navigation across the Settings widgets and shortcuts pages.

### Fixed

- Left indentation disparity in the clipboard shelf where items with custom titles or short preview text were centered instead of aligning to the start.

## [0.5.2] - 2026-11-18

### Fixed

- Automated GitHub Actions release packaging to directly publish releases with the NSIS installer attached (`Lilt-0.5.2-x64-setup.exe`).
- Cleaned up changelog history structure and updated release build metadata.

## [0.5.1] - 2026-11-18

### Added

- Collapsed island quick application launcher with mouse wheel scroll selection and animated item transitions.
- Application shortcut management in Settings with automatic icon extraction (up to 7 items).
- Dynamic rotating sun icon with 16 vector frames that spins smoothly while dragging the brightness slider, with a live percentage readout.
- Configurable "UI hue gradient" slider under Settings > Appearance (0–100 %, default 0 %) that restores subtle edge sheen and radial rim disc glow without coupling to transparency.
- Clipboard image thumbnail previews, hex color swatches, and web link domain badges in the clipboard history shelf.

### Changed

- Expanded dashboard control deck combines CPU and RAM telemetry meters, up/down network rates, and the 8-button quick actions strip into a unified tile.
- Peek auto-hide toggle is repositioned to the quick actions strip with open/closed eyelash eye icons.
- CPU and RAM telemetry meter bars stretch seamlessly across the telemetry deck and dynamically adapt their accent fill color to the playing music artwork.

### Fixed

- Resolved meter bar layout collapsing when horizontal layout alignment was centered.
- Corrected peek auto-hide icon state inversion to accurately reflect island visibility.
- Prevented CPU/RAM meter bar default color from showing purple, aligning with the default UI theme accent.

## [0.4.13] - 2026-11-17

### Added

- Sound output and microphone device selectors, microphone volume and mute controls.
- Configurable 12/24-hour time and date formats, with an idle clock/date carousel
  and independently adjustable clock and item durations.
- Named-place weather search and optional current weather in the idle carousel.

### Changed

- The Sound mixer uses aligned rows and pages for additional application sessions.
- Expanded media uses larger artwork and scrolling long titles and artist names.
- Paused media yields to the idle display after a grace period.

### Fixed

- Development builds no longer automatically install production updates at startup.
- Center snapping retains the drag anchor so the island can be dragged away again.
- Native dragging starts outside Slint pointer and timer callbacks.
- Invalid weather coordinates and idle timing values are sanitized; unavailable
  weather falls back to clock and date.


## [0.4.12] - 2026-11-17

### Changed

- Liquid Glass transparency has been redesigned to be distinctly clearer and set apart
  from Smoked Glass, with a 30 % baseline opacity (down from 36 %) and a crisper 11 px
  liquid blur that keeps background shapes and colors recognizable.
- Dashboard tiles in Liquid Glass use a lighter, airy 5 % translucent fill framed by a
  fine 1 px specular glass border, allowing wallpaper to flow seamlessly through them.
- Liquid Glass edge reflections have been brightened with a more radiant upper rim and
  clearer caustic lower glow.
- Liquid Glass text luminance has been tuned to ensure WCAG AA contrast across any
  desktop wallpaper without turning the surface into an opaque dark slab.

### Fixed

- Development install script now detects non-standard Cargo home paths and ensures the
  MinGW compiler tools are prioritized in PATH.

### Added

- The brightness control's sun turns as the slider is dragged.

### Changed

- The glass appearances' lit edge is a soft radial glow that eases around the rounded
  corners and fades inward, and it tracks the transparency slider: brighter and more
  defined as the glass clears, softer as it frosts.

### Fixed

- Auto-hide no longer leaves a blurred patch at the top of the screen; the live blur now
  lifts away with the island.
- A tucked island only reappears when the pointer reaches the very top edge of the screen.
- The Appearance settings page keeps its controls in place when the appearance is changed,
  rather than shifting the layout.
- The Brightness quick action shows a sun instead of a monitor, and the skip-track buttons
  show their bar so they no longer look like a play button.
- The clipboard entries show their Pin and Delete buttons again.
- The Tasks and Clipboard quick action closes the shelf when its button is tapped a second
  time.

## [0.4.8] - 2026-11-17

### Added

- An auto-hide option that tucks the island away above the top edge and reveals it when
  the pointer reaches the top centre of the screen, with a smooth slide and fade.

### Changed

- Each appearance keeps its own heading face, chosen to stay legible: Midnight uses Space
  Grotesk, Smoked Glass uses Newsreader italic and Liquid Glass uses Unbounded. Settings
  section headers use a clean tracked label so the display face is reserved for prominent
  titles.

### Fixed

- The settings window no longer resizes or overlaps its content when the Appearance page
  opens; it stays wide enough for the appearance cards.

## [0.4.7] - 2026-11-16

### Added

- The Sound panel shows application icons, vertical volume sliders, editable percentage
  fields and mute buttons for active audio sessions.
- Volume percentages can be entered directly for the main output and each application.

### Changed

- The quick action gear opens Lilt Settings. The Sound panel has more room for the
  application mixer and keeps its controls stable while session values refresh.

## [0.4.6] - 2026-11-16

### Changed

- The compact media progress is a thin 1 px line with no gray track; the island's
  lit lower edge now shows on either side of the accent fill.
- Smoked Glass gains a subtle lower rim so its lit perimeter matches the upper
  edge. Liquid Glass's lower rim remains noticeably brighter.
- Clipboard pin and delete buttons stay clearly visible on every entry; pinned
  entries show their pin in the accent colour.

## [0.4.5] - 2026-11-16

### Added

- Lilt now downloads, verifies and installs a newer stable GitHub release at startup.
  Manual and periodic checks remain informational.

### Fixed

- Glass appearances no longer draw a pixelated bright border around the rounded island
  perimeter; their highlight is now inset from the curved corners.

## [0.4.1] - 2026-11-16

### Added

- Per-application volume controls in the Sound quick panel, including mute.
- A Tasks + Clipboard quick action, which opens the side panel without changing the
  corresponding Settings switches.
- Release-aware in-app updates: Lilt can download, verify and install the latest
  signed GitHub release installer.

### Fixed

- The compact media progress line could extend outside the rounded island. It now stays
  within the pill's curved ends.
- Liquid Glass no longer shows stepped corners while the island animates.
- The side Tasks/Clipboard box aligns with the expanded island, and long clipboard
  previews no longer overlap their metadata.
- Late-arriving media artwork now refreshes instead of remaining from the prior track.

## [0.4.0] - 2026-11-16

### Added

- Media controls in the compact island for pausing or resuming playback and skipping to
  the next track.
- An update check in the expanded island that compares the installed build with the
  latest commit on `main` and opens GitHub when an update is available.

## [0.3.0] - 2026-11-15

### Added

- **Quick controls.** The Volume, Brightness, Bluetooth and Wi-Fi buttons open pop-ups in
  the island instead of Windows Settings: volume and mute, screen brightness, Bluetooth
  and Wi-Fi on/off, paired Bluetooth devices (with connect and disconnect for headphones
  and speakers whose audio driver Windows lists), and Wi-Fi networks with connect (including a password for new networks)
  and disconnect. Each pop-up links to the matching Settings page.
- **Clipboard shelf**, in the box next to the expanded island (switch between Tasks and
  Clipboard in its title). Search copied text and snippets, filter by pinned, recent,
  snippets, links, code or other text, click to copy back, pin, edit and delete, and
  save snippets by hand. Remembering copied text is opt-in and off by default, can be
  paused or turned off at any time and keeps only text: copies marked private by their
  app (as password managers do), copies from excluded apps and text that looks like a
  password, key or token are skipped. Stored only on this PC, with a retention period
  (30 days by default), an entry limit and clear-history controls in
  *Settings → Clipboard*, plus an optional global shortcut.
- **Typography for each appearance.** Interface text is now DM Sans everywhere; headings
  use Space Grotesk in Midnight, Instrument Serif in Smoked Glass and the script Allura
  in Liquid Glass (short headings only; longer titles stay in DM Sans). Code uses
  JetBrains Mono. The fonts are bundled under the SIL Open Font License; nothing is
  downloaded. Each appearance card in Settings shows its name in its own face.

### Fixed

- The "now playing" bars could stay still while music played, for example after a
  full-screen video. Lilt now steps them itself and checks whether they should move
  whenever playback, visibility, expansion or the motion setting changes, and once a
  minute.

## [0.2.1] - 2026-11-15

### Added

- **Glass transparency** in *Settings → Appearance & position*, for Smoked Glass and
  Liquid Glass: from a heavy blur at 0 %, through the designed frosted look at 50 %, to
  clear glass at 100 %, where the desktop shows through sharply. Up to 50 % text keeps
  WCAG AA contrast; above that the glass dims less and less over bright windows.
- Liquid Glass bends the desktop along its edges like a thick lens.

### Fixed

- With Windows display scaling other than 100 % (for example 125 %), clicks on the
  island and in Settings landed on the wrong controls, and the glass's blurred shapes
  were drawn larger than the island. Lilt drew everything at 100 % while converting
  pointer positions at the real scale; it now draws at the display's scale from the
  start.
- The glass appearances showed black, oversized shapes around the island instead of a
  blur. Lilt now draws the backdrop itself inside the island's window, from a copy of
  the screen behind it, so it follows the island's outline exactly. The island leaves
  itself out of that copy.

### Changed

- Releases contain only the installer; its checksum is in the release notes.

## [0.2.0] - 2026-11-15

The first build published on GitHub. It includes everything listed under 0.1.0 in the
changelog, plus the following.

### Added

- **Three appearances**, chosen in *Settings → Appearance & position* and applied at once:
  - **Midnight**, the original minimal near-black look (the default).
  - **Smoked Glass**: dark smoked glass with a lit rim over a live blur of the desktop.
  - **Liquid Glass**: clear glass lit along its upper and lower edges. Its tint follows
    the brightness of the screen behind the island: light over dark content, and only
    as dense as readable text (WCAG AA contrast) requires over bright windows.

  The glass appearances fall back to frosted surfaces when Windows' transparency
  effects, battery saver or low-power mode rule out the blur, or when *Reduce
  transparency* is on, and Settings says why.
- **Tasks**: a prioritised task list in its own box beside or below the expanded island,
  with quick entry (N), priorities, due dates and times, labels, a "Focus now" task,
  completion with undo, postponing, your own order within a priority, and a searchable
  archive with restore and permanent deletion. Tasks are stored in SQLite on this PC.
- The "now playing" bars in the compact island move like an equalizer while music
  plays, about 14 steps a second, only while the compact island is on screen. They
  settle when playback pauses and stay still with reduced motion.
- Copy, cut and paste in Lilt's text fields.

### Changed

- Appearance and Position are one settings page, *Appearance & position*, with a button
  that resets only those settings.
- The settings file is now version 3. Older files are read as before and keep every
  setting; an unknown appearance name falls back to Midnight.
- Lower memory use: about 3.0–3.6 MB in Task Manager. CPU, memory and network readings
  come straight from Windows instead of a library whose CPU reading cost about 4 MB.

### Fixed

- When a browser plays in one tab while another is paused, or replaces its media
  session as tabs start and stop, Lilt follows the session that is playing, so the
  title stays current and the playing bars move.

## [0.1.0]

First release.

### Added

- A floating island at the top of the screen that expands into a dashboard. It stays
  above other windows without taking focus, lets clicks through its transparent edges,
  follows display, scaling and work-area changes, can be dragged along the top edge, and
  hides while a full-screen application is in front.
- Media controls for any application that uses Windows media sessions: title, artist,
  album artwork, play/pause, previous/next, seeking and a source switcher, plus a brief
  banner when the track changes.
- Ambient colours derived from the current album artwork, with adjustable intensity.
- CPU, memory and network readings, collected only while they are on screen.
- A calendar tile and weekly agenda from an imported `.ics` file, with recurring events
  and time-zone support.
- Quick actions for common Windows Settings pages and tools.
- A settings window, a global shortcut (Ctrl+Alt+Space by default), start with Windows,
  reduced motion and a low-power mode.
- A notification-area icon to show, hide, configure and quit Lilt.
