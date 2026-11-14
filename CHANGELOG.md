# Changelog

All notable changes to Lilt are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org).

## [Unreleased]

### Added

- A second appearance, **Smoked Glass** (first released as Liquid Glass): dark smoked glass with a lit rim and a soft
  reflection, over a live blur of the desktop drawn by Windows' compositor. It applies to
  the island, the music player, dashboard tiles, track banners and the settings window,
  and falls back to frosted surfaces when Windows' transparency effects, battery saver
  or low-power mode rule out the blur, or when *Reduce transparency* is on.
- *Settings → Appearance* shows the appearances as cards with previews drawn from their
  own design tokens. Switching is immediate and needs no restart.
- A third appearance, **Liquid Glass**: clear glass lit along its upper and lower edges.
  Its tint follows the brightness of the screen behind the island, light over dark
  content and only as dense as WCAG AA text contrast requires over bright windows.
- The "now playing" bars in the compact island move like an equalizer while music
  plays. They step about 14 times a second on a timer, only while the compact island is
  on screen, settle into the flat idle bars when playback pauses, and stay still with
  reduced motion.

- **Tasks**: a prioritised task list in its own box beside or below the expanded island,
  with quick entry (N), priorities, due dates and times, labels, a "Focus now" task,
  completion with undo, postponing, your own order within a priority, and a searchable
  archive with restore and permanent deletion. Tasks are stored in SQLite on this PC.
- Copy, cut and paste in Lilt's text fields.

### Changed

- The original appearance is now called **Midnight**. It is unchanged and remains the
  default.
- Appearance and Position are one settings page, *Appearance & position*, with a button
  that resets only those settings.
- The settings file is now version 3. Older files are read as before and keep their
  appearance and every other setting: a version 2 file that chose Liquid Glass, which
  was the smoked look, now opens in Smoked Glass. An unknown appearance name falls back
  to Midnight.

### Fixed

- When a browser plays in one tab while another is paused, or replaces its media
  session (as Edge does when another tab starts playing), Lilt now follows the session
  that is playing instead of keeping the paused or stale one, so the title is current and
  the playing bars move.
- Lilt's memory use is down from about 7.6–8.3 MB to 3.0–3.6 MB (Task Manager's
  "Memory"). CPU, memory and network readings now come straight from Windows instead of
  the `sysinfo` crate, whose CPU reading loaded the Performance Data Helper's counter
  tables (about 4 MB).

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
