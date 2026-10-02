//! The library as the UI sees it: every video in memory (on the UI thread),
//! grouped into movies, shows (with seasons) and other videos, plus the
//! playlists. Views subscribe to changes and rebuild their models.

use super::db::{self, Playlist, ShowRow};
use super::scan::{self, Progress};
use super::{Kind, Video, Watch, parse};
use crate::{cmd, fmt, prefs, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

#[derive(Debug)]
pub struct Season {
    /// `None` for episodes without a season number (specials, odd names).
    pub number: Option<u32>,
    /// In episode order.
    pub episodes: Vec<Rc<Video>>,
}

impl Season {
    pub fn name(&self) -> String {
        match self.number {
            Some(0) => "Specials".into(),
            Some(n) => format!("Season {n}"),
            None => "Episodes".into(),
        }
    }
}

#[derive(Debug)]
pub struct Show {
    pub key: String,
    pub name: String,
    pub year: Option<i32>,
    pub plot: String,
    pub rating: Option<f64>,
    pub genres: String,
    pub poster: String,
    pub folder: String,
    pub seasons: Vec<Season>,
}

impl Show {
    pub fn episodes(&self) -> impl Iterator<Item = &Rc<Video>> {
        self.seasons.iter().flat_map(|s| s.episodes.iter())
    }

    pub fn count(&self) -> usize {
        self.seasons.iter().map(|s| s.episodes.len()).sum()
    }

    pub fn unwatched(&self) -> usize {
        self.episodes().filter(|e| !e.watch.get().watched).count()
    }

    pub fn last_played(&self) -> i64 {
        self.episodes().map(|e| e.watch.get().last_played).max().unwrap_or(0)
    }

    pub fn added(&self) -> i64 {
        self.episodes().map(|e| e.added).max().unwrap_or(0)
    }

    /// The poster, else the first episode's frame.
    pub fn art(&self) -> String {
        if !self.poster.is_empty() {
            return self.poster.clone();
        }
        self.episodes().map(|e| e.wide_art().to_string()).find(|a| !a.is_empty()).unwrap_or_default()
    }

    /// What to play when the show is opened with Play: the episode after the
    /// last one watched, or the first.
    pub fn next_episode(&self) -> Option<Rc<Video>> {
        let eps: Vec<&Rc<Video>> = self.episodes().collect();
        if let Some(v) = eps.iter().filter(|e| e.in_progress()).max_by_key(|e| e.watch.get().last_played) {
            return Some((*v).clone());
        }
        let last = eps.iter().enumerate().filter(|(_, e)| e.watch.get().watched).max_by_key(|(_, e)| e.watch.get().last_played);
        let from = last.map_or(0, |(i, _)| i + 1);
        eps.iter()
            .skip(from)
            .find(|e| !e.watch.get().watched)
            .or_else(|| eps.iter().find(|e| !e.watch.get().watched))
            .map(|e| (*e).clone())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    Library,
    Playlists,
    Scan,
    /// Watched marks or positions changed.
    Watch,
}

#[derive(Default)]
struct Lib {
    loaded: bool,
    videos: Vec<Rc<Video>>,
    by_path: HashMap<PathBuf, Rc<Video>>,
    shows: Vec<Rc<Show>>,
    playlists: Vec<Playlist>,
    scanning: Option<(usize, usize)>,
    /// What the background work after a scan is doing, for the sidebar footer.
    busy: Option<String>,
    rescan_again: bool,
    folder_stamp: i64,
    error: Option<String>,
}

type Listener = (glib::WeakRef<gtk::Widget>, Box<dyn Fn(Change)>);

thread_local! {
    static LIB: RefCell<Lib> = RefCell::new(Lib::default());
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    static RELOAD_PENDING: Cell<bool> = const { Cell::new(false) };
}

/// Call `f` on every change while `owner` is alive.
pub fn subscribe(owner: &impl IsA<gtk::Widget>, f: impl Fn(Change) + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push((owner.upcast_ref::<gtk::Widget>().downgrade(), Box::new(f))));
}

fn notify(change: Change) {
    // Take the list so listeners may subscribe or read the store freely.
    let listeners = LISTENERS.with(|l| std::mem::take(&mut *l.borrow_mut()));
    let mut alive = Vec::with_capacity(listeners.len());
    for (w, f) in listeners {
        if w.upgrade().is_some() {
            f(change);
            alive.push((w, f));
        }
    }
    LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        alive.append(&mut l);
        *l = alive;
    });
}

pub fn loaded() -> bool {
    LIB.with(|l| l.borrow().loaded)
}

pub fn videos() -> Vec<Rc<Video>> {
    LIB.with(|l| l.borrow().videos.clone())
}

pub fn movies() -> Vec<Rc<Video>> {
    LIB.with(|l| l.borrow().videos.iter().filter(|v| v.kind == Kind::Movie).cloned().collect())
}

pub fn shows() -> Vec<Rc<Show>> {
    LIB.with(|l| l.borrow().shows.clone())
}

pub fn show(key: &str) -> Option<Rc<Show>> {
    LIB.with(|l| l.borrow().shows.iter().find(|s| s.key == key).cloned())
}

pub fn show_of(v: &Video) -> Option<Rc<Show>> {
    if v.kind == Kind::Episode { show(&v.show_key()) } else { None }
}

pub fn playlists() -> Vec<Playlist> {
    LIB.with(|l| l.borrow().playlists.clone())
}

pub fn scanning() -> Option<(usize, usize)> {
    LIB.with(|l| l.borrow().scanning)
}

pub fn busy() -> Option<String> {
    LIB.with(|l| l.borrow().busy.clone())
}

pub fn error() -> Option<String> {
    LIB.with(|l| l.borrow().error.clone())
}

pub fn find(path: &Path) -> Option<Rc<Video>> {
    LIB.with(|l| l.borrow().by_path.get(path).cloned())
}

/// A video for any file: from the library, or named from its path.
pub fn video_for(path: &Path) -> Rc<Video> {
    find(path).unwrap_or_else(|| {
        let p = parse::parse(path);
        let mut v = Video { path: path.to_path_buf(), title: p.title, year: p.year, ..Default::default() };
        if p.episode {
            v.kind = Kind::Episode;
            v.show = p.show;
            v.season = p.season;
            v.episode = p.number;
        }
        Rc::new(v)
    })
}

/// Case-insensitive, ignoring a leading "The ".
pub fn sort_key(s: &str) -> String {
    let l = s.trim().to_lowercase();
    l.strip_prefix("the ").map(str::to_string).unwrap_or(l)
}

fn episode_order(a: &Video, b: &Video) -> std::cmp::Ordering {
    (a.season.unwrap_or(u32::MAX), a.episode.unwrap_or(u32::MAX), &a.path).cmp(&(
        b.season.unwrap_or(u32::MAX),
        b.episode.unwrap_or(u32::MAX),
        &b.path,
    ))
}

fn group_shows(videos: &[Rc<Video>], rows: &[ShowRow]) -> Vec<Rc<Show>> {
    let mut by_key: HashMap<String, Vec<Rc<Video>>> = HashMap::new();
    for v in videos.iter().filter(|v| v.kind == Kind::Episode) {
        by_key.entry(v.show_key()).or_default().push(v.clone());
    }
    let info: HashMap<&str, &ShowRow> = rows.iter().map(|r| (r.key.as_str(), r)).collect();
    let mut shows: Vec<Rc<Show>> = by_key
        .into_iter()
        .map(|(key, mut eps)| {
            eps.sort_by(|a, b| episode_order(a, b));
            let mut seasons: Vec<Season> = Vec::new();
            for e in eps {
                match seasons.last_mut() {
                    Some(s) if s.number == e.season => s.episodes.push(e),
                    _ => seasons.push(Season { number: e.season, episodes: vec![e] }),
                }
            }
            // Specials last.
            seasons.sort_by_key(|s| match s.number {
                Some(0) => u32::MAX - 1,
                Some(n) => n,
                None => u32::MAX,
            });
            let row = info.get(key.as_str());
            let first = seasons.first().and_then(|s| s.episodes.first());
            Rc::new(Show {
                name: row.map(|r| r.name.clone()).or_else(|| first.map(|e| e.show.clone())).unwrap_or_default(),
                year: row.and_then(|r| r.year),
                plot: row.map(|r| r.plot.clone()).unwrap_or_default(),
                rating: row.and_then(|r| r.rating),
                genres: row.map(|r| r.genres.clone()).unwrap_or_default(),
                poster: row.map(|r| r.poster.clone()).unwrap_or_default(),
                folder: row.map(|r| r.folder.clone()).unwrap_or_default(),
                key,
                seasons,
            })
        })
        .collect();
    shows.sort_by_key(|s| sort_key(&s.name));
    shows
}

// ---------- Home rows ----------

/// Started but not finished, most recent first.
pub fn continue_watching() -> Vec<Rc<Video>> {
    let mut v: Vec<Rc<Video>> = videos().into_iter().filter(|v| v.in_progress()).collect();
    v.sort_by_key(|v| std::cmp::Reverse(v.watch.get().last_played));
    v.truncate(24);
    v
}

/// The next episode of each show being watched (and not already half-watched).
pub fn next_up() -> Vec<Rc<Video>> {
    let mut out: Vec<(i64, Rc<Video>)> = Vec::new();
    for s in shows() {
        let last = s.last_played();
        if last == 0 || s.unwatched() == 0 {
            continue;
        }
        if let Some(e) = s.next_episode()
            && !e.in_progress()
        {
            out.push((last, e));
        }
    }
    out.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    out.into_iter().map(|(_, e)| e).take(24).collect()
}

/// Newest first; a show's new episodes count once.
pub fn recently_added() -> Vec<Rc<Video>> {
    let mut v = videos();
    v.sort_by_key(|v| std::cmp::Reverse(v.added));
    let mut seen = std::collections::HashSet::new();
    v.into_iter().filter(|v| v.kind != Kind::Episode || seen.insert(v.show_key())).take(24).collect()
}

/// The episode after `v` in its show, for auto-play.
pub fn next_episode(v: &Video) -> Option<Rc<Video>> {
    let show = show_of(v)?;
    let eps: Vec<&Rc<Video>> = show.episodes().collect();
    let i = eps.iter().position(|e| e.path == v.path)?;
    eps.get(i + 1).map(|e| (*e).clone())
}

// ---------- Loading and scanning ----------

/// Read the database into memory (off the main thread) and tell the views.
pub fn reload() {
    cmd::background(
        || -> anyhow::Result<(Vec<Video>, Vec<ShowRow>, Vec<Playlist>)> {
            let conn = db::open()?;
            Ok((db::load_all(&conn)?, db::shows(&conn)?, db::playlists(&conn)?))
        },
        |res| match res {
            Ok((videos, rows, playlists)) => {
                let videos: Vec<Rc<Video>> = videos.into_iter().map(Rc::new).collect();
                let by_path = videos.iter().map(|v| (v.path.clone(), v.clone())).collect();
                let shows = group_shows(&videos, &rows);
                LIB.with(|l| {
                    let mut l = l.borrow_mut();
                    l.loaded = true;
                    l.videos = videos;
                    l.by_path = by_path;
                    l.shows = shows;
                    l.playlists = playlists;
                    l.error = None;
                });
                notify(Change::Library);
                notify(Change::Playlists);
                crate::player::library_changed();
            }
            Err(e) => {
                LIB.with(|l| {
                    let mut l = l.borrow_mut();
                    l.loaded = true;
                    l.error = Some(e.to_string());
                });
                notify(Change::Library);
            }
        },
    );
}

/// Reload soon, at most every couple of seconds (pictures arriving in batches).
fn reload_soon() {
    if RELOAD_PENDING.with(|r| r.replace(true)) {
        return;
    }
    glib::timeout_add_local_once(std::time::Duration::from_millis(1500), || {
        RELOAD_PENDING.with(|r| r.set(false));
        super::art::forget();
        reload();
    });
}

pub fn roots() -> Vec<PathBuf> {
    prefs::get().library_folders.iter().map(PathBuf::from).collect()
}

fn set_busy(text: Option<String>) {
    LIB.with(|l| l.borrow_mut().busy = text);
    notify(Change::Scan);
}

/// Scan the library folders for new, changed and removed files, then grab
/// missing pictures and (when it's on) look things up on TMDB.
pub fn rescan(announce: bool) {
    let busy = LIB.with(|l| {
        let mut l = l.borrow_mut();
        if l.scanning.is_some() || l.busy.is_some() {
            l.rescan_again = true;
            return true;
        }
        l.scanning = Some((0, 0));
        false
    });
    if busy {
        return;
    }
    notify(Change::Scan);
    let roots = roots();
    let p = prefs::get();
    let frames = p.frames;
    let tmdb = super::tmdb::Settings::from_prefs(&p);
    let (tx, rx) = async_channel::unbounded::<Progress>();
    std::thread::spawn(move || {
        let send = |p| {
            let _ = tx.send_blocking(p);
        };
        scan::run(roots, frames, send);
        if let Some(t) = tmdb {
            super::tmdb::run(&t, send);
        }
    });
    glib::spawn_future_local(async move {
        while let Ok(p) = rx.recv().await {
            match p {
                Progress::Reading { done, total } => {
                    LIB.with(|l| l.borrow_mut().scanning = Some((done, total)));
                    notify(Change::Scan);
                }
                Progress::Finished { added, updated, removed, stamp } => {
                    LIB.with(|l| {
                        let mut l = l.borrow_mut();
                        l.folder_stamp = stamp;
                        l.scanning = None;
                    });
                    notify(Change::Scan);
                    if added + updated + removed > 0 {
                        super::art::forget();
                        reload();
                    }
                    if announce {
                        window::toast(&summary(added, updated, removed));
                    }
                }
                Progress::Pictures { done, total } => {
                    set_busy((done < total).then(|| format!("Pictures {}%", done * 100 / total.max(1))));
                }
                Progress::Online { done, total } => {
                    set_busy((done < total).then(|| format!("TMDB {}%", done * 100 / total.max(1))));
                }
                Progress::Changed => reload_soon(),
                Progress::Failed(e) => {
                    LIB.with(|l| l.borrow_mut().scanning = None);
                    notify(Change::Scan);
                    window::toast(&format!("Couldn't scan the library: {e}"));
                }
            }
        }
        // The worker is done with everything.
        let again = LIB.with(|l| {
            let mut l = l.borrow_mut();
            l.scanning = None;
            l.busy = None;
            std::mem::take(&mut l.rescan_again)
        });
        notify(Change::Scan);
        if again {
            rescan(false);
        }
    });
}

fn summary(added: usize, updated: usize, removed: usize) -> String {
    if added + updated + removed == 0 {
        return "The library is up to date.".into();
    }
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("{} added", fmt::count(added, "video", "videos")));
    }
    if updated > 0 {
        parts.push(format!("{} updated", fmt::thousands(updated)));
    }
    if removed > 0 {
        parts.push(format!("{} removed", fmt::thousands(removed)));
    }
    let mut s = parts.join(", ");
    s.push('.');
    s
}

/// Rescan shortly after files are added, removed or renamed (polled; folder
/// timestamps are cheap to read and more dependable than inotify on big trees).
pub fn start_watching() {
    glib::timeout_add_seconds_local(30, || {
        if !prefs::get().watch || scanning().is_some() || busy().is_some() {
            return glib::ControlFlow::Continue;
        }
        let last = LIB.with(|l| l.borrow().folder_stamp);
        cmd::background(
            || scan::folder_stamp(&roots()),
            move |now| {
                if now > last {
                    rescan(false);
                }
            },
        );
        glib::ControlFlow::Continue
    });
}

// ---------- Watch state ----------

/// Save a video's watch state (off the main thread).
pub fn persist_watch(v: &Video) {
    let (path, w) = (v.path.clone(), v.watch.get());
    cmd::background(move || db::open().and_then(|c| db::save_watch(&c, &path, &w)), |_| {});
}

/// Change a video's watch state in memory and on disk, and tell the views.
pub fn update_watch(v: &Video, change: impl FnOnce(&mut Watch)) {
    let mut w = v.watch.get();
    change(&mut w);
    v.watch.set(w);
    persist_watch(v);
    notify(Change::Watch);
}

pub fn set_watched(videos: &[Rc<Video>], watched: bool) {
    for v in videos {
        let mut w = v.watch.get();
        w.watched = watched;
        w.position = 0.0;
        if watched && w.last_played == 0 {
            w.last_played = super::now();
        }
        v.watch.set(w);
        persist_watch(v);
    }
    notify(Change::Watch);
}

/// Tell the views that positions moved (throttled by the caller).
pub fn watch_changed() {
    notify(Change::Watch);
}

// ---------- Playlists ----------

fn playlists_changed(res: anyhow::Result<Vec<Playlist>>) {
    match res {
        Ok(p) => {
            LIB.with(|l| l.borrow_mut().playlists = p);
            notify(Change::Playlists);
        }
        Err(e) => window::toast(&format!("Couldn't save the playlist: {e}")),
    }
}

/// Run a playlist edit off the main thread, then refresh the playlist list.
pub fn edit_playlists(
    work: impl FnOnce(&rusqlite::Connection) -> anyhow::Result<()> + Send + 'static,
    done: impl FnOnce() + 'static,
) {
    cmd::background(
        move || {
            let conn = db::open()?;
            work(&conn)?;
            db::playlists(&conn)
        },
        move |res| {
            let ok = res.is_ok();
            playlists_changed(res);
            if ok {
                done();
            }
        },
    );
}

pub fn playlist_videos(id: i64, done: impl FnOnce(Vec<Rc<Video>>) + 'static) {
    cmd::background(
        move || db::open().and_then(|c| db::playlist_paths(&c, id)).unwrap_or_default(),
        move |paths| done(paths.iter().map(|p| video_for(p)).collect()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ep(path: &str, show: &str, s: Option<u32>, e: u32) -> Rc<Video> {
        Rc::new(Video {
            path: path.into(),
            kind: Kind::Episode,
            show: show.into(),
            season: s,
            episode: Some(e),
            duration: 1800.0,
            ..Default::default()
        })
    }

    #[test]
    fn groups_shows_into_seasons() {
        let vids = vec![
            ep("/t/b2", "Lost", Some(2), 1),
            ep("/t/a2", "lost", Some(1), 2),
            ep("/t/a1", "Lost", Some(1), 1),
            ep("/t/sp", "Lost", Some(0), 1),
            ep("/t/x", "Dark", Some(1), 1),
        ];
        let shows = group_shows(&vids, &[]);
        assert_eq!(shows.len(), 2);
        assert_eq!(shows[0].name, "Dark");
        let lost = &shows[1];
        assert_eq!(lost.seasons.iter().map(|s| s.name()).collect::<Vec<_>>(), vec!["Season 1", "Season 2", "Specials"]);
        assert_eq!(lost.seasons[0].episodes[0].path, PathBuf::from("/t/a1"));
        assert_eq!(lost.count(), 4);
    }

    #[test]
    fn picks_the_next_episode() {
        let vids = vec![ep("/1", "S", Some(1), 1), ep("/2", "S", Some(1), 2), ep("/3", "S", Some(1), 3)];
        let shows = group_shows(&vids, &[]);
        let s = &shows[0];
        assert_eq!(s.next_episode().unwrap().path, PathBuf::from("/1"));
        vids[0].watch.set(Watch { watched: true, last_played: 5, ..Default::default() });
        assert_eq!(s.next_episode().unwrap().path, PathBuf::from("/2"));
        vids[2].watch.set(Watch { position: 60.0, last_played: 9, ..Default::default() });
        assert_eq!(s.next_episode().unwrap().path, PathBuf::from("/3"));
    }

    #[test]
    fn sorts_ignoring_the() {
        assert_eq!(sort_key("The Wire"), "wire");
        assert_eq!(sort_key("Theatre"), "theatre");
    }

    #[test]
    fn summarises_scans() {
        assert_eq!(summary(0, 0, 0), "The library is up to date.");
        assert_eq!(summary(3, 0, 1), "3 videos added, 1 removed.");
    }
}
