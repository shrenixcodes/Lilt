# Changelog

All notable changes to Lilt are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org).

## [Unreleased]

## [0.2.0] - 2026-11-15

The first build published on GitHub. It includes everything listed under 0.1.0 below,
plus the following.

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
