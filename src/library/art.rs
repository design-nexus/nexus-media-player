//! Posters and frames. Pictures come from images beside the videos
//! (`poster.jpg`, `<name>-poster.jpg`, `<name>-thumb.jpg`…), from TMDB, or a
//! frame grabbed from the video with ffmpeg. Each is scaled once into the cache
//! in two sizes; the UI loads those small JPEGs off the main thread.

use super::hash;
use super::probe::output_within;
use crate::paths;
use gtk::gdk_pixbuf::{InterpType, Pixbuf};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const SMALL: i32 = 420;
const LARGE: i32 = 1280;
const IMAGE_EXTS: &[&str] = &["jpg", "jpeg", "png", "webp"];

pub fn file(key: &str, small: bool) -> PathBuf {
    paths::art_dir().join(if small { format!("{key}-s.jpg") } else { format!("{key}.jpg") })
}

fn mtime(path: &Path) -> i64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs() as i64)
}

/// Fit inside `max`×`max`, never upscaling.
fn fit(src: &Pixbuf, max: i32) -> Option<Pixbuf> {
    let (w, h) = (src.width(), src.height());
    if w <= 0 || h <= 0 {
        return None;
    }
    let s = (max as f64 / w.max(h) as f64).min(1.0);
    let img = src.scale_simple(((w as f64 * s) as i32).max(1), ((h as f64 * s) as i32).max(1), InterpType::Bilinear)?;
    // Flatten any transparency; JPEG has none.
    if img.has_alpha() {
        img.composite_color_simple(img.width(), img.height(), InterpType::Nearest, 255, 8, 0x101010, 0x101010)
    } else {
        Some(img)
    }
}

fn save(key: &str, src: &Pixbuf) -> Option<String> {
    std::fs::create_dir_all(paths::art_dir()).ok()?;
    for (small, size) in [(false, LARGE), (true, SMALL)] {
        let img = fit(src, size)?;
        let out = file(key, small);
        let tmp = out.with_extension(format!("tmp-{}", std::process::id()));
        img.savev(&tmp, "jpeg", &[("quality", "88")]).ok()?;
        std::fs::rename(&tmp, &out).ok()?;
    }
    Some(key.to_string())
}

/// Cache an image file. Returns its key, or "" when it can't be read.
pub fn from_image(path: &Path) -> String {
    let key = hash(&format!("img\u{1f}{}\u{1f}{}", path.display(), mtime(path)));
    if file(&key, true).exists() {
        return key;
    }
    Pixbuf::from_file(path).ok().and_then(|p| save(&key, &p)).unwrap_or_default()
}

/// Cache downloaded image bytes under `id` (a URL, say).
pub fn from_bytes(id: &str, bytes: &[u8]) -> String {
    let key = hash(&format!("url\u{1f}{id}"));
    if file(&key, true).exists() {
        return key;
    }
    let loader = gtk::gdk_pixbuf::PixbufLoader::new();
    if loader.write(bytes).is_err() || loader.close().is_err() {
        return String::new();
    }
    loader.pixbuf().and_then(|p| save(&key, &p)).unwrap_or_default()
}

/// The key a cached URL image would have, if it's there.
pub fn cached_url(id: &str) -> Option<String> {
    let key = hash(&format!("url\u{1f}{id}"));
    file(&key, true).exists().then_some(key)
}

/// Grab a frame a tenth of the way in. Runs ffmpeg; call it off the UI thread.
pub fn frame(video: &Path, duration: f64) -> String {
    let key = hash(&format!("frame\u{1f}{}\u{1f}{}", video.display(), mtime(video)));
    if file(&key, true).exists() {
        return key;
    }
    let at = if duration > 20.0 { (duration * 0.1).min(600.0) } else { (duration * 0.3).max(0.0) };
    let Some(dir) = std::fs::create_dir_all(paths::art_dir()).ok().map(|_| paths::art_dir()) else { return String::new() };
    let tmp = dir.join(format!("{key}.grab-{}.jpg", std::process::id()));
    let ok = output_within(
        Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-ss", &format!("{at:.2}"), "-i"])
            .arg(video)
            .args(["-frames:v", "1", "-vf", &format!("scale='min({LARGE},iw)':-2"), "-q:v", "3"])
            .arg(&tmp),
        Duration::from_secs(30),
    )
    .is_some();
    let key = if ok { Pixbuf::from_file(&tmp).ok().and_then(|p| save(&key, &p)).unwrap_or_default() } else { String::new() };
    let _ = std::fs::remove_file(&tmp);
    key
}

fn image_named(dir: &Path, stems: &[String]) -> Option<PathBuf> {
    for stem in stems {
        for ext in IMAGE_EXTS {
            let p = dir.join(format!("{stem}.{ext}"));
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// A movie's poster: `<name>-poster.jpg`, or `poster.jpg`/`folder.jpg`/`cover.jpg`
/// when the movie has its folder to itself.
pub fn movie_poster(video: &Path, alone_in_folder: bool) -> Option<PathBuf> {
    let dir = video.parent()?;
    let stem = video.file_stem()?.to_string_lossy();
    let mut names = vec![format!("{stem}-poster"), format!("{stem}.poster")];
    if alone_in_folder {
        names.extend(["poster", "folder", "cover", "movie"].map(String::from));
    }
    image_named(dir, &names)
}

/// An episode's own picture: `<name>-thumb.jpg` or `<name>.jpg`.
pub fn episode_still(video: &Path) -> Option<PathBuf> {
    let dir = video.parent()?;
    let stem = video.file_stem()?.to_string_lossy();
    image_named(dir, &[format!("{stem}-thumb"), stem.to_string()])
}

/// A show's poster in its folder.
pub fn show_poster(folder: &Path) -> Option<PathBuf> {
    image_named(folder, &["poster", "folder", "show", "cover"].map(String::from))
}

/// A show's wide backdrop in its folder.
pub fn show_backdrop(folder: &Path) -> Option<PathBuf> {
    image_named(folder, &["fanart", "backdrop", "background", "landscape"].map(String::from))
}

// ---------- UI side ----------

thread_local! {
    static TEXTURES: RefCell<HashMap<(String, bool), gdk::Texture>> = RefCell::new(HashMap::new());
}

/// Load a picture texture off the main thread and hand it to `done`
/// (immediately when it's cached). `done` gets `None` when there's no picture.
pub fn load(key: &str, small: bool, done: impl FnOnce(Option<gdk::Texture>) + 'static) {
    if key.is_empty() {
        done(None);
        return;
    }
    let id = (key.to_string(), small);
    if let Some(t) = TEXTURES.with(|c| c.borrow().get(&id).cloned()) {
        done(Some(t));
        return;
    }
    let path = file(key, small);
    let handle = gio::spawn_blocking(move || std::fs::read(path).ok());
    glib::spawn_future_local(async move {
        let bytes = handle.await.ok().flatten();
        let tex = bytes.and_then(|b| gdk::Texture::from_bytes(&glib::Bytes::from_owned(b)).ok());
        if let Some(t) = &tex {
            TEXTURES.with(|c| {
                let mut c = c.borrow_mut();
                // Keep memory bounded on huge libraries; pictures reload quickly.
                if c.len() > 500 {
                    c.clear();
                }
                c.insert(id, t.clone());
            });
        }
        done(tex);
    });
}

/// Forget cached textures (after a rescan replaced pictures).
pub fn forget() {
    TEXTURES.with(|c| c.borrow_mut().clear());
}

/// A picture of a fixed size with a placeholder glyph when there's none.
#[derive(Clone)]
pub struct Art {
    pub root: gtk::Overlay,
    picture: gtk::Picture,
    placeholder: gtk::Image,
    key: std::rc::Rc<RefCell<String>>,
    small: bool,
}

impl Art {
    pub fn new(width: i32, height: i32, small: bool, icon: &str) -> Art {
        let root = gtk::Overlay::new();
        root.add_css_class("art");
        root.set_overflow(gtk::Overflow::Hidden);
        root.set_size_request(width, height);
        root.set_halign(gtk::Align::Start);
        root.set_valign(gtk::Align::Start);
        let placeholder = gtk::Image::from_icon_name(icon);
        placeholder.add_css_class("art-placeholder");
        placeholder.set_pixel_size((width.min(height) / 3).clamp(16, 72));
        placeholder.set_size_request(width, height);
        root.set_child(Some(&placeholder));
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        picture.set_size_request(width, height);
        picture.set_visible(false);
        root.add_overlay(&picture);
        Art { root, picture, placeholder, key: Default::default(), small }
    }

    /// A 2:3 poster.
    pub fn poster(width: i32) -> Art {
        Art::new(width, width * 3 / 2, true, "nmp-movie-symbolic")
    }

    /// A 16:9 frame.
    pub fn wide(width: i32) -> Art {
        Art::new(width, width * 9 / 16, true, "nmp-video-symbolic")
    }

    pub fn set_key(&self, key: &str) {
        if *self.key.borrow() == key && (self.picture.is_visible() || key.is_empty()) {
            return;
        }
        *self.key.borrow_mut() = key.to_string();
        self.picture.set_visible(false);
        self.placeholder.set_visible(true);
        let me = self.clone();
        let want = key.to_string();
        load(key, self.small, move |tex| {
            // The widget may have been re-bound to another video meanwhile.
            if *me.key.borrow() != want {
                return;
            }
            if let Some(t) = tex {
                me.picture.set_paintable(Some(&t));
                me.picture.set_visible(true);
                me.placeholder.set_visible(false);
            }
        });
    }
}
