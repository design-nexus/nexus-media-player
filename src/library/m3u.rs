//! M3U/M3U8 playlists: read paths (absolute, relative or file:// URIs) and
//! write extended M3U with paths relative to the playlist where possible.

use super::Video;
use std::path::{Path, PathBuf};

pub fn parse(text: &str, base: &Path) -> Vec<PathBuf> {
    text.lines()
        .map(|l| l.trim().trim_start_matches('\u{feff}'))
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            if let Some(rest) = l.strip_prefix("file://") {
                return gtk::glib::filename_from_uri(&format!("file://{rest}")).ok().map(|(p, _)| p);
            }
            if l.contains("://") {
                return None; // Streams aren't part of the library.
            }
            let p = PathBuf::from(l.replace('\\', "/"));
            Some(if p.is_absolute() { p } else { base.join(p) })
        })
        .collect()
}

pub fn write(videos: &[Video], base: &Path) -> String {
    let mut out = String::from("#EXTM3U\n");
    for t in videos {
        out.push_str(&format!("#EXTINF:{},{}\n", t.duration.round() as i64, t.label()));
        let p = t.path.strip_prefix(base).unwrap_or(&t.path);
        out.push_str(&p.to_string_lossy());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        let base = Path::new("/videos");
        let videos = vec![
            Video { path: "/videos/A/1.mkv".into(), title: "One".into(), duration: 61.4, ..Default::default() },
            Video { path: "/elsewhere/2.mp4".into(), title: "Two".into(), duration: 5.0, ..Default::default() },
        ];
        let text = write(&videos, base);
        assert!(text.starts_with("#EXTM3U\n#EXTINF:61,One\nA/1.mkv\n"));
        assert_eq!(parse(&text, base), vec![PathBuf::from("/videos/A/1.mkv"), PathBuf::from("/elsewhere/2.mp4")]);
    }

    #[test]
    fn reads_uris_and_skips_streams() {
        let text = "\u{feff}#EXTM3U\nfile:///m/a%20b.ogg\nhttp://radio/x\n\nsub\\c.mp3\n";
        assert_eq!(parse(text, Path::new("/p")), vec![PathBuf::from("/m/a b.ogg"), PathBuf::from("/p/sub/c.mp3")]);
    }
}
