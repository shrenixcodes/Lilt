# Next release notes

Status: unreleased changes on top of 0.4.12. No version tag or GitHub release is
created by this integration. See [CHANGELOG](../CHANGELOG.md) for version history
and [release procedure](../RELEASE.md) for packaging and publication.

## Audio

Open the Sound quick panel to select the default output or input endpoint, adjust
microphone volume or mute, and control individual applications. Application sessions
are paged; unavailable or disconnected endpoints are refreshed from Windows.
Device selection changes the Windows default endpoint, not just Lilt playback.

## Clock and weather

Settings exposes time and date formats, an idle cycle toggle, and separate hold
times (3?60 seconds). The default clock hold is seven seconds; other items use five.
Search for a city or place and choose a result to add weather. Place search uses
Open-Meteo geocoding; current weather uses Open-Meteo forecast data and refreshes
every 20 minutes. No automatic location permission is requested. Failed weather
requests leave the clock/date available. The current temperature is shown in Celsius.

## Media and placement

Expanded media has larger artwork and scrolling text for long metadata. Paused
media gives way to the idle carousel after a grace period. Dragging retains its
anchor across center snapping, allowing movement away from the snapped position.

## Integration

Remote main through d5689c7 (0.4.12) is retained, including liquid glass, auto-hide,
brightness sun rotation, clipboard actions and shelf toggle fixes. The previous
local committed tree matched remote e127a32 exactly despite rewritten commit IDs.
The two overlapping local edits were resolved to preserve the remote skip glyphs
and animated brightness image along with the local Sound panel changes.

## Validation

Run tools/install-dev.ps1 for formatting, Clippy, tests, release dev build and local
installation. Hardware changes, real media sessions, weather networking, dragging
and auto-hide also require hands-on testing. A clean-account production installer
and uninstall check remains part of the release procedure before publication.
