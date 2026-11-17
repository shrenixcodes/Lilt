# Island and audio controls implementation plan

Goal: clear track navigation, easy centering, complete sound controls and useful idle information.
Architecture: retain Slint views and native Windows worker boundaries.
Tech stack: Rust, Slint, Windows Core Audio, ureq.

- [ ] Replace next glyph with double chevrons and end bar.
- [ ] Add scaled 24 logical pixel center magnet in WM_MOVING; show accent outline while snapped; verify both sides and escape.
- [ ] Add output/input selectors and input volume/mute through existing controls worker; handle disconnected endpoints.
- [ ] Cycle idle time/date/weather with reduced-motion support; bounded background weather retrieval and offline fallback.
- [ ] Run tools/install-dev.ps1; resolve formatting, Clippy, tests and release errors.
- [ ] Exercise installed app on PC, normal and edge cases; report external verification gaps.

User approved proceeding with approximate location weather. Keep D:\Lilt on main; no push or merge.
