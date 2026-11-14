//! Publishes a silent media session with metadata and artwork for testing Lilt without
//! touching real media.
//!
//! Usage: `media-session [seconds-per-track] [track-count]` (defaults: 240, 3).
//!
//! Each track has a title, artist, album and a different colour of artwork, so track
//! changes, artwork and ambient colours can be checked. Play/pause, next/previous and
//! seeking from Lilt act on this session. The playlist repeats until the process is
//! stopped.

use std::io::Write;
use windows::Foundation::Uri;
use windows::Media::Core::MediaSource;
use windows::Media::MediaPlaybackType;
use windows::Media::Playback::{MediaPlaybackItem, MediaPlaybackList, MediaPlayer};
use windows::Storage::Streams::RandomAccessStreamReference;

const COLOURS: [(&str, [u8; 3]); 6] = [
    ("Blue", [36, 92, 214]),
    ("Amber", [232, 132, 34]),
    ("Violet", [128, 70, 200]),
    ("Crimson", [196, 32, 48]),
    ("Forest", [44, 150, 84]),
    ("Graphite", [120, 120, 120]),
];

fn silent_wav(path: &std::path::Path, seconds: u32) {
    let rate = 8000u32;
    let data_len = rate * seconds;
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(b"RIFF").unwrap();
    f.write_all(&(36 + data_len).to_le_bytes()).unwrap();
    f.write_all(b"WAVEfmt ").unwrap();
    f.write_all(&16u32.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&rate.to_le_bytes()).unwrap();
    f.write_all(&1u16.to_le_bytes()).unwrap();
    f.write_all(&8u16.to_le_bytes()).unwrap();
    f.write_all(b"data").unwrap();
    f.write_all(&data_len.to_le_bytes()).unwrap();
    f.write_all(&vec![128u8; data_len as usize]).unwrap();
}

fn artwork(path: &std::path::Path, rgb: [u8; 3]) {
    let img = image::RgbImage::from_fn(300, 300, |x, y| {
        // Mostly the colour, with a dark band so extraction must find the subject.
        if y > 220 || x < 20 { image::Rgb([12, 12, 14]) } else { image::Rgb(rgb) }
    });
    img.save(path).unwrap();
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let seconds: u32 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(240);
    let count: usize = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(3).min(COLOURS.len());
    let dir = std::env::temp_dir().join("lilt-media-session");
    std::fs::create_dir_all(&dir).unwrap();
    let audio = dir.join("silence.wav");
    silent_wav(&audio, seconds);

    let list = MediaPlaybackList::new()?;
    list.SetAutoRepeatEnabled(true)?;
    for (i, (name, rgb)) in COLOURS.iter().take(count).enumerate() {
        let art = dir.join(format!("art-{i}.png"));
        artwork(&art, *rgb);
        let source = MediaSource::CreateFromUri(&Uri::CreateUri(
            &format!("file:///{}", audio.display()).replace(char::from(92), "/").into(),
        )?)?;
        let item = MediaPlaybackItem::Create(&source)?;
        let props = item.GetDisplayProperties()?;
        props.SetType(MediaPlaybackType::Music)?;
        let music = props.MusicProperties()?;
        music.SetTitle(&format!("{name} Hours").into())?;
        music.SetArtist(&"Lilt Test Ensemble".into())?;
        music.SetAlbumTitle(&"Colour Studies".into())?;
        let uri = Uri::CreateUri(&format!("file:///{}", art.display()).replace(char::from(92), "/").into())?;
        props.SetThumbnail(&RandomAccessStreamReference::CreateFromUri(&uri)?)?;
        item.ApplyDisplayProperties(&props)?;
        list.Items()?.Append(&item)?;
    }
    let player = MediaPlayer::new()?;
    player
        .CommandManager()?
        .PositionBehavior()?
        .SetEnablingRule(windows::Media::Playback::MediaCommandEnablingRule::Always)?;
    player.SetSource(&list)?;
    player.Play()?;
    println!("playing {count} silent tracks of {seconds} s, repeating until stopped");
    std::thread::sleep(std::time::Duration::from_secs(3600));
    player.Pause()?;
    Ok(())
}
