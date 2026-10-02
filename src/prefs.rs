//! This app's own preferences (`~/.config/nexus-media-player/settings.toml`).

use crate::{cmd, paths};
use gtk::glib;
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeMode {
    /// Follow the active Omarchy theme live.
    Omarchy,
    /// Use a bundled or custom theme.
    Theme,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EqPreset {
    pub name: String,
    pub preamp: f64,
    pub bands: [f64; 10],
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub mode: ThemeMode,
    pub theme: String,
    pub reduce_motion: bool,
    pub glow: bool,
    pub last_section: String,
    /// Folders the library is built from.
    pub library_folders: Vec<String>,
    /// Rescan when files are added, removed or renamed.
    pub watch: bool,
    /// Grab a frame from videos that have no picture.
    pub frames: bool,
    /// Pick up each video where it was left.
    pub resume: bool,
    /// Bring back the queue on start.
    pub restore_queue: bool,
    /// Play a show's next episode when one ends.
    pub autoplay_next: bool,
    /// Seconds for ←/→ and Shift+←/→.
    pub skip_short: f64,
    pub skip_long: f64,
    /// Touchpad seeking goes the other way (for natural scrolling).
    pub swipe_reverse: bool,
    pub speed: f64,
    /// Decode on the graphics card when it can.
    pub hwdec: bool,
    /// Preferred track languages, as mpv takes them (`en,eng`); empty for the file's default.
    pub audio_lang: String,
    pub sub_lang: String,
    /// Show subtitles when a video has them.
    pub subtitles: bool,
    pub sub_scale: f64,
    /// Thumbnails on the seek bar.
    pub scrub: bool,
    pub notify: bool,
    pub tmdb: bool,
    pub tmdb_key: String,
    pub tmdb_language: String,
    /// 0..1 (what the slider shows; mpv's own curve is cubic).
    pub volume: f64,
    pub muted: bool,
    pub shuffle: bool,
    /// off, all or one.
    pub repeat: String,
    pub eq_enabled: bool,
    /// The preset the faders came from ("" once they're edited by hand).
    pub eq_preset: String,
    pub eq_preamp: f64,
    pub eq_bands: [f64; 10],
    pub eq_custom: Vec<EqPreset>,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            mode: ThemeMode::Omarchy,
            theme: "tokyo-night".into(),
            reduce_motion: false,
            glow: true,
            last_section: "home".into(),
            library_folders: vec![paths::videos_dir().to_string_lossy().into_owned()],
            watch: true,
            frames: true,
            resume: true,
            restore_queue: true,
            autoplay_next: true,
            skip_short: 5.0,
            skip_long: 30.0,
            swipe_reverse: false,
            speed: 1.0,
            hwdec: true,
            audio_lang: String::new(),
            sub_lang: String::new(),
            subtitles: true,
            sub_scale: 1.0,
            scrub: true,
            notify: false,
            tmdb: false,
            tmdb_key: String::new(),
            tmdb_language: "en-US".into(),
            volume: 1.0,
            muted: false,
            shuffle: false,
            repeat: "off".into(),
            eq_enabled: false,
            eq_preset: "flat".into(),
            eq_preamp: 0.0,
            eq_bands: [0.0; 10],
            eq_custom: Vec::new(),
        }
    }
}

thread_local! {
    static BROKEN: Cell<bool> = const { Cell::new(false) };
    static PREFS: RefCell<Prefs> = RefCell::new(load());
    static PENDING: Cell<Option<glib::SourceId>> = const { Cell::new(None) };
}

fn load() -> Prefs {
    let file = paths::prefs_file();
    let Ok(text) = std::fs::read_to_string(&file) else { return Prefs::default() };
    match toml::from_str(&text) {
        Ok(p) => p,
        Err(e) => {
            // Don't lose a file with a typo in it: keep a copy before the
            // defaults are saved over it.
            let backup = file.with_extension("toml.bak");
            let _ = std::fs::copy(&file, &backup);
            eprintln!("media-player: {} couldn't be read ({e}); kept a copy as {}", file.display(), backup.display());
            BROKEN.with(|b| b.set(true));
            Prefs::default()
        }
    }
}

/// True (once) when the settings file couldn't be read at start.
pub fn take_broken() -> bool {
    BROKEN.with(|b| b.replace(false))
}

pub fn get() -> Prefs {
    PREFS.with(|p| p.borrow().clone())
}

/// Change the prefs now; the file is written about 450 ms after the last change,
/// so dragging a slider doesn't rewrite it on every tick.
pub fn update(change: impl FnOnce(&mut Prefs)) {
    PREFS.with(|p| change(&mut p.borrow_mut()));
    if let Some(id) = PENDING.with(|p| p.take()) {
        id.remove();
    }
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(450), || {
        PENDING.with(|p| p.set(None));
        save();
    });
    PENDING.with(|p| p.set(Some(id)));
}

/// Write any pending change now (on quit).
pub fn flush() {
    if let Some(id) = PENDING.with(|p| p.take()) {
        id.remove();
        save();
    }
}

fn save() {
    let text = PREFS.with(|p| toml::to_string_pretty(&*p.borrow()));
    if let Ok(text) = text {
        let _ = cmd::atomic_write(&paths::prefs_file(), &text);
    }
}
