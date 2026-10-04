//! Seek-bar previews. The first time a video plays, ffmpeg makes a contact
//! sheet of up to 100 small frames from its keyframes (on a worker thread);
//! hovering the seek bar shows the frame nearest the pointer, cut out of
//! that sheet by [`FramePaintable`].

use crate::library::probe::{self, output_within};
use crate::library::{Video, hash};
use crate::{cmd, paths, prefs};
use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, gio, glib, graphene};
use serde::{Deserialize, Serialize};
use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

const COLS: u32 = 10;
const ROWS: u32 = 10;
const CELL_WIDTH: u32 = 192;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
struct Layout {
    /// Seconds between frames.
    interval: f64,
    count: u32,
}

struct Sheet {
    path: PathBuf,
    texture: gdk::Texture,
    layout: Layout,
}

thread_local! {
    static SHEET: RefCell<Option<Sheet>> = const { RefCell::new(None) };
    static WORKING: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    static GENERATION: Cell<u64> = const { Cell::new(0) };
}

fn files(path: &Path) -> (PathBuf, PathBuf) {
    let stamp = std::fs::metadata(path)
        .ok()
        .map(|m| {
            (m.len(), m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs()))
        })
        .unwrap_or_default();
    let key = hash(&format!("{}\u{1f}{}\u{1f}{}", path.display(), stamp.0, stamp.1));
    let dir = paths::scrub_dir();
    (dir.join(format!("{key}.jpg")), dir.join(format!("{key}.json")))
}

/// Seconds between frames so a video fits in one sheet, never closer than 2 s.
fn interval_for(duration: f64) -> f64 {
    (duration / (COLS * ROWS) as f64).max(2.0)
}

/// Make the sheet for `path` (runs ffmpeg; call it off the UI thread).
fn generate(path: &Path, mut duration: f64) -> Option<Layout> {
    let (jpg, json) = files(path);
    if let Some(l) = std::fs::read_to_string(&json).ok().and_then(|t| serde_json::from_str(&t).ok())
        && jpg.exists()
    {
        return Some(l);
    }
    if duration <= 0.0 {
        duration = probe::probe(path).ok().flatten()?.duration;
    }
    if duration < 10.0 {
        return None;
    }
    let interval = interval_for(duration);
    std::fs::create_dir_all(paths::scrub_dir()).ok()?;
    let tmp = jpg.with_extension(format!("tmp-{}.jpg", std::process::id()));
    let filter = format!("fps=1/{interval:.3},scale={CELL_WIDTH}:-2,tile={COLS}x{ROWS}");
    // Keyframes only: fast, and close enough for a preview.
    output_within(
        Command::new("ffmpeg")
            .args(["-v", "error", "-nostdin", "-y", "-skip_frame", "nokey", "-i"])
            .arg(path)
            .args(["-an", "-sn", "-dn", "-vf", &filter, "-frames:v", "1", "-q:v", "5"])
            .arg(&tmp),
        Duration::from_secs(600),
    )?;
    std::fs::rename(&tmp, &jpg).ok()?;
    let layout = Layout { interval, count: ((duration / interval).ceil() as u32).min(COLS * ROWS) };
    let _ = cmd::atomic_write(&json, &serde_json::to_string(&layout).ok()?);
    Some(layout)
}

/// Get the sheet for a video that's starting to play: from the cache, or
/// made in the background.
pub fn prepare(v: &Video) {
    let path = v.path.clone();
    if SHEET.with(|s| s.borrow().as_ref().is_some_and(|s| s.path == path)) {
        return;
    }
    SHEET.with(|s| *s.borrow_mut() = None);
    if !prefs::get().scrub || !cmd::present("ffmpeg") || crate::library::is_url(&path) {
        return;
    }
    // One at a time; a newer video takes over when the current one is done.
    let generation = GENERATION.with(|g| {
        g.set(g.get() + 1);
        g.get()
    });
    if WORKING.with(|w| w.borrow().is_some()) {
        WORKING.with(|w| *w.borrow_mut() = Some(path.clone()));
        return;
    }
    start(path, v.duration, generation);
}

fn start(path: PathBuf, duration: f64, generation: u64) {
    WORKING.with(|w| *w.borrow_mut() = Some(path.clone()));
    let p2 = path.clone();
    cmd::background(
        move || generate(&p2, duration).map(|l| (l, files(&p2).0)),
        move |res| {
            let latest = WORKING.with(|w| w.borrow_mut().take());
            let current = GENERATION.with(|g| g.get()) == generation;
            if current && let Some((layout, jpg)) = res {
                load(path.clone(), layout, jpg);
            }
            // A different video started meanwhile: do that one now.
            if let Some(next) = latest.filter(|n| *n != path)
                && let Some(v) = crate::player::current().filter(|c| c.path == next)
            {
                start(next, v.duration, GENERATION.with(|g| g.get()));
            }
        },
    );
}

fn load(path: PathBuf, layout: Layout, jpg: PathBuf) {
    let handle = gio::spawn_blocking(move || std::fs::read(jpg).ok());
    glib::spawn_future_local(async move {
        let Some(bytes) = handle.await.ok().flatten() else { return };
        let Ok(texture) = gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) else { return };
        if crate::player::current().is_some_and(|c| c.path == path) {
            SHEET.with(|s| *s.borrow_mut() = Some(Sheet { path, texture, layout }));
        }
    });
}

/// Show the frame nearest `secs` in `paintable`. False when there's none.
pub fn show(paintable: &FramePaintable, secs: f64) -> bool {
    SHEET.with(|s| {
        let s = s.borrow();
        let Some(s) = s.as_ref() else { return false };
        let i = ((secs / s.layout.interval).floor().max(0.0) as u32).min(s.layout.count.saturating_sub(1));
        let (cw, ch) = (s.texture.width() as f32 / COLS as f32, s.texture.height() as f32 / ROWS as f32);
        let cell = graphene::Rect::new((i % COLS) as f32 * cw, (i / COLS) as f32 * ch, cw, ch);
        paintable.set(&s.texture, cell);
        true
    })
}

/// Forget every sheet on disk.
pub fn clear_cache() {
    let _ = std::fs::remove_dir_all(paths::scrub_dir());
    SHEET.with(|s| *s.borrow_mut() = None);
}

// ---------- One frame out of a sheet ----------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct FramePaintable {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub cell: Cell<(f32, f32, f32, f32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for FramePaintable {
        const NAME: &'static str = "NmpFramePaintable";
        type Type = super::FramePaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for FramePaintable {}

    impl PaintableImpl for FramePaintable {
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            let Some(t) = self.texture.borrow().clone() else { return };
            let (x, y, w, h) = self.cell.get();
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let snapshot = snapshot.downcast_ref::<gtk::Snapshot>().expect("gtk snapshot");
            let (sx, sy) = (width as f32 / w, height as f32 / h);
            snapshot.push_clip(&graphene::Rect::new(0.0, 0.0, width as f32, height as f32));
            let full = graphene::Rect::new(-x * sx, -y * sy, t.width() as f32 * sx, t.height() as f32 * sy);
            snapshot.append_scaled_texture(&t, gtk::gsk::ScalingFilter::Linear, &full);
            snapshot.pop();
        }

        fn intrinsic_width(&self) -> i32 {
            self.cell.get().2 as i32
        }

        fn intrinsic_height(&self) -> i32 {
            self.cell.get().3 as i32
        }
    }
}

glib::wrapper! {
    pub struct FramePaintable(ObjectSubclass<imp::FramePaintable>) @implements gdk::Paintable;
}

impl Default for FramePaintable {
    fn default() -> Self {
        glib::Object::new()
    }
}

impl FramePaintable {
    fn set(&self, texture: &gdk::Texture, cell: graphene::Rect) {
        let imp = self.imp();
        let same = imp.texture.borrow().as_ref() == Some(texture);
        let new = (cell.x(), cell.y(), cell.width(), cell.height());
        let resized = imp.cell.get().2 != new.2 || imp.cell.get().3 != new.3;
        if same && imp.cell.get() == new {
            return;
        }
        *imp.texture.borrow_mut() = Some(texture.clone());
        imp.cell.set(new);
        if resized {
            self.invalidate_size();
        }
        self.invalidate_contents();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spaces_frames_to_fit_one_sheet() {
        assert_eq!(interval_for(60.0), 2.0);
        assert!((interval_for(7200.0) - 72.0).abs() < 1e-9);
    }
}
