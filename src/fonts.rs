//! The fonts bundled with Lilt (SIL Open Font License 1.1, see `assets/fonts`).
//!
//! They are compiled into the executable and registered with Slint's font collection at
//! start-up, before any window is created; nothing is loaded from disk or the network.
//! Registered this way they are drawn as outlines, so every size and display scale is
//! sharp. (Importing them in a `.slint` file would pre-render them as bitmaps at the
//! sizes written in the UI, which cannot follow computed sizes or display scaling.)

use std::cell::Cell;
use std::sync::Arc;

use slint::fontique_011::fontique;

/// DM Sans (interface), Space Grotesk (Midnight headings), Instrument Serif (Smoked
/// Glass headings), Allura (Liquid Glass headings) and JetBrains Mono (code).
const FONTS: [&[u8]; 5] = [
    include_bytes!("../assets/fonts/DMSans-Variable.ttf"),
    include_bytes!("../assets/fonts/SpaceGrotesk-Variable.ttf"),
    include_bytes!("../assets/fonts/InstrumentSerif-Regular.ttf"),
    include_bytes!("../assets/fonts/Allura-Regular.ttf"),
    include_bytes!("../assets/fonts/JetBrainsMono-Variable.ttf"),
];

/// Registers the bundled fonts with the current thread's Slint context, once. Call right
/// after the Slint platform is set. (Lilt has one UI thread; tests have one per test.)
pub fn register() {
    thread_local! {
        static DONE: Cell<bool> = const { Cell::new(false) };
    }
    if DONE.replace(true) {
        return;
    }
    let mut collection = slint::fontique_011::shared_collection();
    for data in FONTS {
        // The executable's own bytes, shared without a copy.
        let registered = collection.register_fonts(fontique::Blob::new(Arc::new(data)), None);
        debug_assert!(!registered.is_empty(), "a bundled font did not load");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_bundled_font_is_a_font_with_the_expected_family() {
        let mut collection =
            fontique::Collection::new(fontique::CollectionOptions { shared: false, system_fonts: false });
        let names: Vec<String> = FONTS
            .iter()
            .map(|data| {
                let fonts = collection.register_fonts(fontique::Blob::new(Arc::new(*data)), None);
                let (family, _) = fonts.first().expect("a font");
                collection.family_name(*family).expect("a family name").to_owned()
            })
            .collect();
        assert_eq!(names, ["DM Sans", "Space Grotesk", "Instrument Serif", "Allura", "JetBrains Mono"]);
    }
}
