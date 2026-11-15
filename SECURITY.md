# Security and privacy

## Reporting a vulnerability

Please report security problems privately through
[GitHub security advisories](https://github.com/shrenixcodes/Lilt/security/advisories/new)
rather than in a public issue. Include the Lilt version, your Windows version and the
steps to reproduce. You should receive a reply within a week.

Only the latest release receives security fixes.

## What Lilt does with your data

Lilt works entirely on your PC.

- **No network access.** Lilt makes no network requests: no telemetry, update checks,
  crash reporting or online calendars.
- **Media information** (titles, artists, artwork, playback position) is read from the
  Windows media session API to display it and is never written to disk or sent anywhere.
  Artwork is held in memory only, at 128 × 128 pixels, for the last eight tracks.
- **System readings** (CPU, memory, network throughput) are read only while the expanded
  island is visible and are not recorded.
- **Calendar files** you import are read from the location you chose; Lilt stores only
  the path. Events are not copied elsewhere.
- **Clipboard text** is read only if you turn on *Remember copied text* (off by
  default), and only while capture is on and not paused. Windows notifies Lilt of each
  clipboard change (`AddClipboardFormatListener`); Lilt does not poll. Before reading
  anything it checks whether the copying app marked the copy as private
  (`ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer Ignore`, or
  `CanIncludeInClipboardHistory` set to zero) and whether the app is excluded; only
  then is the text read. Text that looks like a password, private key or access token
  is discarded. Only plain text is kept, never images or files. Entries are stored in
  `clipboard.sqlite3` and are never logged, uploaded, synced or sent to any service; the
  log records only why a copy was skipped (for example "looks secret"), never its text
  or app. The app's name comes from Windows (the process that owns the clipboard);
  when Windows does not report one, the entry says "Unknown app".
- **Deleting clipboard entries** removes them from the database and overwrites their
  text inside the database file (SQLite `secure_delete`); the write-ahead log is
  truncated afterwards. Older copies may still exist outside Lilt's control, for example
  in backups, System Restore points, or blocks an SSD has remapped. The database is not
  encrypted: it is protected by your Windows account's file permissions, like the rest
  of your profile. Encrypting it with DPAPI would protect against the same people (any
  program running as you can decrypt DPAPI data) while preventing search inside the
  database, so Lilt does not pretend it is more protected than it is.
- **The glass appearances** read the screen behind the island to draw their backdrop
  and choose how dark their tint must be: reduced and blurred for blurred glass, at
  full resolution for clear glass. The image is kept in memory only to draw the glass
  and is never written or sent. Midnight and frosted glass never read the screen.

## Files and registry entries

| What | Where |
| --- | --- |
| Settings | `%APPDATA%\Lilt\settings.json` |
| Tasks | `%LOCALAPPDATA%\Lilt\tasks.sqlite3` (your tasks; never sent anywhere) |
| Clipboard shelf | `%LOCALAPPDATA%\Lilt\clipboard.sqlite3` (snippets, and copied text if capture is on; never sent anywhere) |
| Logs | `%LOCALAPPDATA%\Lilt\logs\` (application events only, no media or calendar content) |
| Start with Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Lilt` |
| Installation (installer) | `%LOCALAPPDATA%\Programs\Lilt` by default, and its uninstall entry under `HKCU` |

The uninstaller removes the program, its shortcuts, the start-up entry and, if you
choose, the settings, tasks, clipboard shelf and logs.

## Hardening

- Lilt runs with the user's normal privileges (`asInvoker`) and never asks for
  elevation. The installer installs per user and does not require administrator rights.
- Untrusted input is size-limited and parsed defensively:
  - artwork streams over 8 MB are ignored; images are decoded with dimension
    (4096 px) and allocation (64 MB) limits;
  - calendar files over 8 MB are not read, and at most 20 000 events and 10 000
    recurrence steps per event are processed;
  - settings files over 256 KB are rejected, and every value is validated and clamped
    on load. Invalid files are backed up and replaced by defaults.
- Quick actions open fixed Windows Settings pages and built-in tools only; no command
  line from settings or other input is ever executed.
- No credentials are stored, and none are needed.
