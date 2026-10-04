//! The video library: what's on disk, read once into SQLite, held in memory
//! on the UI thread for the views.

pub mod art;
pub mod db;
pub mod m3u;
pub mod nfo;
pub mod parse;
pub mod probe;
pub mod scan;
pub mod store;
pub mod tmdb;

use std::cell::Cell;
use std::path::{Path, PathBuf};

/// Video file extensions the scanner picks up.
pub const EXTENSIONS: &[&str] =
    &["mkv", "mp4", "m4v", "webm", "avi", "mov", "wmv", "flv", "ts", "m2ts", "mts", "mpg", "mpeg", "ogv", "3gp", "divx", "vob"];

/// A stream or web address (`https://…`) rather than a file.
pub fn is_url(path: &Path) -> bool {
    path.to_str().is_some_and(|s| s.split_once("://").is_some_and(|(scheme, _)| !scheme.is_empty() && !scheme.contains('/')))
}

pub fn is_video(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    Movie,
    Episode,
    /// Anything else: clips, recordings, home videos.
    #[default]
    Other,
}

impl Kind {
    pub fn id(self) -> &'static str {
        match self {
            Kind::Movie => "movie",
            Kind::Episode => "episode",
            Kind::Other => "other",
        }
    }

    pub fn from_id(id: &str) -> Kind {
        match id {
            "movie" => Kind::Movie,
            "episode" => Kind::Episode,
            _ => Kind::Other,
        }
    }
}

/// How far someone got with a video. Kept apart from the scanned data so a
/// rescan never loses it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Watch {
    /// Seconds into the video (0 when not started or finished).
    pub position: f64,
    pub watched: bool,
    pub last_played: i64,
    pub plays: u32,
    /// The audio track picked last time (mpv's id), if any.
    pub aid: Option<i64>,
    /// The subtitle track picked last time; `Some(0)` means subtitles off.
    pub sid: Option<i64>,
    pub sub_delay: f64,
    /// Sound against picture, in seconds (positive: sound later).
    pub audio_delay: f64,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Video {
    /// Row id in the database; 0 for files played from outside the library.
    pub id: i64,
    pub path: PathBuf,
    pub kind: Kind,
    /// Movie title, episode title, or the cleaned-up file name.
    pub title: String,
    pub year: Option<i32>,
    /// For episodes.
    pub show: String,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    pub plot: String,
    pub rating: Option<f64>,
    pub genres: String,
    /// Seconds.
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub vcodec: String,
    pub acodec: String,
    /// Comma-separated languages of the audio and subtitle tracks inside the file.
    pub audio_langs: String,
    pub sub_langs: String,
    /// Art cache keys (see `art`); empty when there's none.
    pub poster: String,
    pub still: String,
    /// A wide picture behind a movie's page.
    pub backdrop: String,
    pub tmdb_id: Option<i64>,
    /// When the file first joined the library (Unix seconds).
    pub added: i64,
    pub mtime: i64,
    pub size: i64,
    /// Live: updated as it plays.
    pub watch: Cell<Watch>,
}

impl Video {
    pub fn file_uri(&self) -> String {
        if is_url(&self.path) {
            return self.path.to_string_lossy().into_owned();
        }
        gtk::glib::filename_to_uri(&self.path, None).map(|u| u.to_string()).unwrap_or_default()
    }

    /// Groups episodes into shows.
    pub fn show_key(&self) -> String {
        show_key(&self.show)
    }

    /// `S01E02`, or empty.
    pub fn code(&self) -> String {
        match (self.season, self.episode) {
            (Some(s), Some(e)) => format!("S{s:02}E{e:02}"),
            (None, Some(e)) => format!("E{e:02}"),
            _ => String::new(),
        }
    }

    /// What lists show as the name: "Show · S01E02 · Title" for episodes.
    pub fn label(&self) -> String {
        match self.kind {
            Kind::Episode => {
                let mut parts = vec![self.show.clone()];
                let code = self.code();
                if !code.is_empty() {
                    parts.push(code);
                }
                if !self.title.is_empty() && self.title != self.show {
                    parts.push(self.title.clone());
                }
                parts.join(" · ")
            }
            _ => match self.year {
                Some(y) if self.kind == Kind::Movie => format!("{} ({y})", self.title),
                _ => self.title.clone(),
            },
        }
    }

    /// The episode's own name, or its code when it has none.
    pub fn episode_name(&self) -> String {
        if self.title.is_empty() || self.title == self.show { self.code() } else { self.title.clone() }
    }

    /// "S01E02 · Name", or just the code when there's no name of its own.
    pub fn episode_line(&self) -> String {
        let (code, name) = (self.code(), self.episode_name());
        if name == code { code } else { format!("{code} · {name}") }
    }

    /// The picture for a poster-shaped card: the poster, else a frame.
    pub fn card_art(&self) -> &str {
        if self.poster.is_empty() { &self.still } else { &self.poster }
    }

    /// The picture for a wide card: a frame, else the poster.
    pub fn wide_art(&self) -> &str {
        if self.still.is_empty() { &self.poster } else { &self.still }
    }

    /// 0..1 through the video, for progress strips.
    pub fn progress(&self) -> f64 {
        let w = self.watch.get();
        if w.watched || self.duration <= 0.0 { 0.0 } else { (w.position / self.duration).clamp(0.0, 1.0) }
    }

    pub fn in_progress(&self) -> bool {
        let w = self.watch.get();
        !w.watched && w.position > 0.0
    }

    pub fn resolution(&self) -> String {
        match self.height {
            0 => String::new(),
            h if h >= 2000 || self.width >= 3800 => "4K".into(),
            h if h >= 1000 || self.width >= 1900 => "1080p".into(),
            h if h >= 700 || self.width >= 1260 => "720p".into(),
            h => format!("{h}p"),
        }
    }

    /// Lowercased text the library search matches against.
    pub fn haystack(&self) -> String {
        format!("{} {} {} {} {}", self.title, self.show, self.code(), self.genres, self.plot).to_lowercase()
    }
}

pub fn show_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// FNV-1a, for stable cache file names.
pub fn hash(text: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

/// The folders a show lives in: `Show/Season 1/x.mkv` → `Show`.
pub fn show_folder(video: &Path) -> Option<PathBuf> {
    let dir = video.parent()?;
    let name = dir.file_name()?.to_string_lossy();
    if parse::season_folder(&name).is_some() { dir.parent().map(Path::to_path_buf) } else { Some(dir.to_path_buf()) }
}
