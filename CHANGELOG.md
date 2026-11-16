# Changelog

All notable changes to Lilt are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org).

## [Unreleased]

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
