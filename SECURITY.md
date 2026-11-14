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

## Files and registry entries

| What | Where |
| --- | --- |
| Settings | `%APPDATA%\Lilt\settings.json` |
| Logs | `%LOCALAPPDATA%\Lilt\logs\` (application events only, no media or calendar content) |
| Start with Windows | `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `Lilt` |
| Installation (installer) | `%LOCALAPPDATA%\Programs\Lilt` by default, and its uninstall entry under `HKCU` |

The uninstaller removes the program, its shortcuts, the start-up entry and, if you
choose, the settings and logs.

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
