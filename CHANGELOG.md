# Changelog

All notable changes to Lilt are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org).

## [Unreleased]

### Added

- A second appearance, **Liquid Glass**: dark smoked glass with a lit rim and a soft
  reflection, over a live blur of the desktop drawn by Windows' compositor. It applies to
  the island, the music player, dashboard tiles, track banners and the settings window,
  and falls back to frosted surfaces when Windows' transparency effects, battery saver
  or low-power mode rule out the blur, or when *Reduce transparency* is on.
- *Settings → Appearance* shows both appearances as cards with previews drawn from their
  own design tokens. Switching is immediate and needs no restart.

### Changed

- The original appearance is now called **Midnight**. It is unchanged and remains the
  default.
- The settings file is now version 2. Version 1 files are read as before and keep their
  appearance and every other setting; an unknown appearance name falls back to
  Midnight.

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
