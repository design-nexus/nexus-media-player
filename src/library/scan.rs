//! Incremental library scan on a worker thread: only new or changed files are
//! read, and files that disappeared are dropped (unless their whole folder is
//! missing, which usually means an unplugged drive). After the names come the
//! slow parts: frames grabbed from videos that have no picture, then TMDB.

use super::db::{self, ShowRow};
use super::{Kind, Video, art, is_video, nfo, parse, probe, show_folder, show_key};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub enum Progress {
    /// Files found; `total` of them need reading.
    Reading {
        done: usize,
        total: usize,
    },
    /// The names are in; `stamp` is the newest folder time seen, for change watching.
    Finished {
        added: usize,
        updated: usize,
        removed: usize,
        stamp: i64,
    },
    /// Grabbing frames for videos with no picture.
    Pictures {
        done: usize,
        total: usize,
    },
    /// Looking videos up on TMDB.
    Online {
        done: usize,
        total: usize,
    },
    /// Some pictures or details changed; reload the library.
    Changed,
    Failed(String),
}

pub type Stamp = (i64, i64);

/// What changed between the files on disk and the database.
#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub new: Vec<PathBuf>,
    pub changed: Vec<PathBuf>,
    pub removed: Vec<PathBuf>,
}

pub fn diff(on_disk: &[(PathBuf, Stamp)], known: &HashMap<PathBuf, Stamp>, roots: &[PathBuf]) -> Diff {
    let mut d = Diff::default();
    let mut seen = HashSet::new();
    for (path, stamp) in on_disk {
        seen.insert(path);
        match known.get(path) {
            None => d.new.push(path.clone()),
            Some(k) if k != stamp => d.changed.push(path.clone()),
            _ => {}
        }
    }
    let present: Vec<&PathBuf> = roots.iter().filter(|r| r.is_dir()).collect();
    for path in known.keys() {
        if seen.contains(path) {
            continue;
        }
        // Only forget files under a root that's still there, and drop files whose
        // root was removed from the library altogether.
        let under_present = present.iter().any(|r| path.starts_with(r));
        let under_any = roots.iter().any(|r| path.starts_with(r));
        if under_present || !under_any {
            d.removed.push(path.clone());
        }
    }
    d.new.sort();
    d.changed.sort();
    d.removed.sort();
    d
}

fn stamp(path: &Path) -> Option<Stamp> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    Some((mtime, meta.len() as i64))
}

fn hidden(e: &walkdir::DirEntry) -> bool {
    e.depth() > 0 && e.file_name().to_string_lossy().starts_with('.')
}

/// Kodi keeps trailers and extras beside movies; they aren't part of the library.
fn extra(path: &Path) -> bool {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
    let in_extras = path.components().any(|c| {
        let c = c.as_os_str().to_string_lossy().to_lowercase();
        matches!(c.as_str(), "extras" | "featurettes" | "trailers" | "behind the scenes" | "deleted scenes" | "sample")
    });
    in_extras || stem.ends_with("-trailer") || stem.ends_with("-sample") || stem == "sample"
}

pub fn walk(roots: &[PathBuf]) -> Vec<(PathBuf, Stamp)> {
    let mut out = Vec::new();
    for root in roots {
        for entry in walkdir::WalkDir::new(root).follow_links(true).into_iter().filter_entry(|e| !hidden(e)) {
            let Ok(entry) = entry else { continue };
            if entry.file_type().is_file()
                && is_video(entry.path())
                && !extra(entry.path())
                && let Some(s) = stamp(entry.path())
            {
                out.push((entry.into_path(), s));
            }
        }
    }
    out
}

/// The newest folder modification time under the roots: a cheap way to notice
/// files being added, removed or renamed.
pub fn folder_stamp(roots: &[PathBuf]) -> i64 {
    let mut newest = 0;
    for root in roots {
        for entry in walkdir::WalkDir::new(root)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| e.file_type().is_dir() && !hidden(e))
            .flatten()
        {
            if let Ok(m) = entry.metadata()
                && let Ok(t) = m.modified()
                && let Ok(d) = t.duration_since(std::time::UNIX_EPOCH)
            {
                newest = newest.max(d.as_secs() as i64);
            }
        }
    }
    newest
}

/// Movies shorter than this are filed as other videos (unless an .nfo says otherwise).
const MOVIE_MIN_SECS: f64 = 40.0 * 60.0;

/// The show a folder holds, with names from its `tvshow.nfo` when there is one.
#[derive(Clone)]
struct ShowInfo {
    name: Option<String>,
}

fn show_info(folder: &Path, cache: &Mutex<HashMap<PathBuf, ShowInfo>>) -> ShowInfo {
    if let Some(s) = cache.lock().ok().and_then(|c| c.get(folder).cloned()) {
        return s;
    }
    let info = ShowInfo { name: nfo::for_show(folder).map(|n| n.title).filter(|t| !t.is_empty()) };
    if let Ok(mut c) = cache.lock() {
        c.insert(folder.to_path_buf(), info.clone());
    }
    info
}

/// Read one file: names from its path and .nfo, details from ffprobe, and any
/// picture beside it. `None` for files with no picture (audio).
fn read(path: &Path, stamp: Stamp, alone: bool, shows: &Mutex<HashMap<PathBuf, ShowInfo>>) -> Option<(Video, &'static str)> {
    let info = match probe::probe(path) {
        Ok(Some(i)) => i,
        Ok(None) => return None,
        Err(()) => probe::Info::default(),
    };
    let p = parse::parse(path);
    let n = nfo::for_video(path);
    let mut v = Video {
        path: path.to_path_buf(),
        title: p.title.clone(),
        year: p.year,
        duration: info.duration,
        width: info.width,
        height: info.height,
        vcodec: info.vcodec,
        acodec: info.acodec,
        audio_langs: info.audio_langs.join(","),
        sub_langs: info.sub_langs.join(","),
        added: super::now(),
        mtime: stamp.0,
        size: stamp.1,
        ..Default::default()
    };
    let nfo_kind = n.as_ref().map(|n| n.root.as_str());
    v.kind = match nfo_kind {
        Some("movie") => Kind::Movie,
        Some("episodedetails") => Kind::Episode,
        _ if p.episode => Kind::Episode,
        _ if info.duration >= MOVIE_MIN_SECS || (info.duration <= 0.0 && p.year.is_some()) => Kind::Movie,
        _ => Kind::Other,
    };
    if v.kind == Kind::Episode {
        v.show = p.show.clone();
        v.season = p.season;
        v.episode = p.number;
        if let Some(folder) = show_folder(path)
            && let Some(name) = show_info(&folder, shows).name
        {
            v.show = name;
        }
        if v.show.is_empty() {
            v.show = "Unknown show".into();
        }
        v.year = None;
    }
    let mut source = "file";
    if let Some(n) = &n {
        source = "nfo";
        if !n.title.is_empty() {
            v.title = n.title.clone();
        }
        if v.kind == Kind::Episode {
            if !n.show.is_empty() && v.show == p.show {
                v.show = n.show.clone();
            }
            v.season = n.season.or(v.season);
            v.episode = n.episode.or(v.episode);
        } else {
            v.year = n.year.or(v.year);
        }
        v.plot = n.plot.clone();
        v.rating = n.rating;
        v.genres = n.genres.join(", ");
        v.tmdb_id = n.tmdb_id;
    }
    match v.kind {
        Kind::Movie => {
            v.poster = art::movie_poster(path, alone).map(|i| art::from_image(&i)).unwrap_or_default();
            v.backdrop = art::movie_backdrop(path, alone).map(|i| art::from_image(&i)).unwrap_or_default();
        }
        Kind::Episode => v.still = art::episode_still(path).map(|i| art::from_image(&i)).unwrap_or_default(),
        Kind::Other => {}
    }
    Some((v, source))
}

/// Refresh every show's row from its folder: name, `tvshow.nfo`, poster.
fn refresh_shows(conn: &rusqlite::Connection) -> anyhow::Result<()> {
    let mut stmt = conn.prepare("SELECT path, show FROM videos WHERE kind = 'episode'")?;
    let rows: Vec<(PathBuf, String)> =
        stmt.query_map([], |r| Ok((PathBuf::from(r.get::<_, String>(0)?), r.get(1)?)))?.collect::<Result<_, _>>()?;
    // Show → the folder most of its episodes are under.
    let mut folders: BTreeMap<String, (String, HashMap<PathBuf, usize>)> = BTreeMap::new();
    for (path, show) in rows {
        let entry = folders.entry(show_key(&show)).or_insert_with(|| (show.clone(), HashMap::new()));
        if let Some(f) = show_folder(&path) {
            *entry.1.entry(f).or_default() += 1;
        }
    }
    for (key, (name, counts)) in folders {
        let folder = counts.into_iter().max_by_key(|(_, n)| *n).map(|(f, _)| f);
        let n = folder.as_deref().and_then(nfo::for_show);
        let mut row = ShowRow { key: key.clone(), name, ..Default::default() };
        if let Some(f) = &folder {
            row.folder = f.to_string_lossy().into_owned();
            row.poster = art::show_poster(f).map(|p| art::from_image(&p)).unwrap_or_default();
            row.backdrop = art::show_backdrop(f).map(|p| art::from_image(&p)).unwrap_or_default();
            if let Some(n) = &n {
                row.year = n.year;
                row.plot = n.plot.clone();
                row.rating = n.rating;
                row.genres = n.genres.join(", ");
                row.tmdb_id = n.tmdb_id;
            }
        }
        db::upsert_show(conn, &row, n.is_some())?;
    }
    db::prune_shows(conn)?;
    Ok(())
}

/// Do `work` on every item with a few threads (ffprobe and ffmpeg are
/// mostly waiting on the disk).
fn parallel<T: Sync, R: Send>(items: &[T], work: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(2, 6);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, R)>> = Mutex::new(Vec::with_capacity(items.len()));
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(item) = items.get(i) else { break };
                    let r = work(item);
                    if let Ok(mut v) = results.lock() {
                        v.push((i, r));
                    }
                }
            });
        }
    });
    let mut v = results.into_inner().unwrap_or_default();
    v.sort_by_key(|(i, _)| *i);
    v.into_iter().map(|(_, r)| r).collect()
}

pub fn run(roots: Vec<PathBuf>, frames: bool, send: impl Fn(Progress)) {
    if let Err(e) = run_inner(&roots, &send) {
        send(Progress::Failed(e.to_string()));
        return;
    }
    if frames && let Err(e) = grab_frames(&send) {
        eprintln!("media-player: frames: {e}");
    }
}

fn run_inner(roots: &[PathBuf], send: &impl Fn(Progress)) -> anyhow::Result<()> {
    let mut conn = db::open()?;
    let stamp = folder_stamp(roots);
    let known = db::stamps(&conn)?;
    let on_disk = walk(roots);
    let d = diff(&on_disk, &known, roots);
    let stamps: HashMap<&PathBuf, Stamp> = on_disk.iter().map(|(p, s)| (p, *s)).collect();
    let mut per_dir: HashMap<PathBuf, usize> = HashMap::new();
    for (p, _) in &on_disk {
        if let Some(dir) = p.parent() {
            *per_dir.entry(dir.to_path_buf()).or_default() += 1;
        }
    }
    let todo: Vec<&PathBuf> = d.new.iter().chain(d.changed.iter()).collect();
    let total = todo.len();
    send(Progress::Reading { done: 0, total });

    let shows: Mutex<HashMap<PathBuf, ShowInfo>> = Mutex::default();
    let mut skipped = 0;
    for (i, chunk) in todo.chunks(48).enumerate() {
        let read_chunk = parallel(chunk, |p| {
            let alone = p.parent().is_some_and(|d| per_dir.get(d).copied().unwrap_or(0) <= 1);
            read(p, stamps.get(p).copied().unwrap_or_default(), alone, &shows)
        });
        let tx = conn.transaction()?;
        for (p, r) in chunk.iter().zip(read_chunk) {
            match r {
                Some((v, source)) => db::upsert(&tx, &v, source)?,
                None => {
                    // Audio in a video container: not part of this library.
                    db::delete(&tx, p)?;
                    skipped += 1;
                }
            }
        }
        tx.commit()?;
        send(Progress::Reading { done: (i * 48 + chunk.len()).min(total), total });
    }
    if !d.removed.is_empty() {
        let tx = conn.transaction()?;
        for p in &d.removed {
            db::delete(&tx, p)?;
        }
        tx.commit()?;
    }
    refresh_shows(&conn)?;
    send(Progress::Finished {
        added: d.new.len().saturating_sub(skipped),
        updated: d.changed.len(),
        removed: d.removed.len(),
        stamp,
    });
    Ok(())
}

/// Grab a frame for every video that has no picture, a few at a time,
/// asking the UI to reload as they arrive.
fn grab_frames(send: &impl Fn(Progress)) -> anyhow::Result<()> {
    let conn = db::open()?;
    let todo = db::missing_stills(&conn)?;
    if todo.is_empty() || !crate::cmd::present("ffmpeg") {
        return Ok(());
    }
    let total = todo.len();
    for (i, chunk) in todo.chunks(12).enumerate() {
        send(Progress::Pictures { done: i * 12, total });
        let keys = parallel(chunk, |(path, duration)| art::frame(path, *duration));
        for ((path, _), key) in chunk.iter().zip(keys) {
            db::set_still(&conn, path, &key)?;
        }
        send(Progress::Changed);
    }
    send(Progress::Pictures { done: total, total });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diffs_new_changed_and_removed() {
        let root = std::env::temp_dir();
        let a = root.join("a.mkv");
        let b = root.join("b.mkv");
        let c = root.join("c.mkv");
        let on_disk = vec![(a.clone(), (1, 10)), (b.clone(), (2, 20))];
        let known: HashMap<PathBuf, Stamp> = [(b.clone(), (1, 20)), (c.clone(), (1, 1))].into_iter().collect();
        let d = diff(&on_disk, &known, &[root]);
        assert_eq!(d.new, vec![a]);
        assert_eq!(d.changed, vec![b]);
        assert_eq!(d.removed, vec![c]);
    }

    #[test]
    fn keeps_files_from_a_missing_root() {
        let gone = PathBuf::from("/nonexistent-video-drive");
        let known: HashMap<PathBuf, Stamp> = [(gone.join("x.mkv"), (1, 1))].into_iter().collect();
        let d = diff(&[], &known, std::slice::from_ref(&gone));
        assert!(d.removed.is_empty());
    }

    #[test]
    fn drops_files_from_a_removed_root() {
        let known: HashMap<PathBuf, Stamp> = [(PathBuf::from("/old/x.mkv"), (1, 1))].into_iter().collect();
        let d = diff(&[], &known, &[std::env::temp_dir()]);
        assert_eq!(d.removed, vec![PathBuf::from("/old/x.mkv")]);
    }

    #[test]
    fn skips_extras() {
        assert!(extra(Path::new("/m/Heat (1995)/Extras/making of.mkv")));
        assert!(extra(Path::new("/m/Heat (1995)/heat-trailer.mkv")));
        assert!(!extra(Path::new("/m/Heat (1995)/heat.mkv")));
    }

    #[test]
    fn runs_in_parallel_in_order() {
        let items: Vec<usize> = (0..50).collect();
        assert_eq!(parallel(&items, |i| i * 2), items.iter().map(|i| i * 2).collect::<Vec<_>>());
    }
}
