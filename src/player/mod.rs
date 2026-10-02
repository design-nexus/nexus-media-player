//! Playback state on the UI thread: the queue, mpv and what's playing.
//! Widgets subscribe to [`Event`]s instead of polling.

pub mod mpv;
pub mod queue;
pub mod scrub;
pub mod video;

use crate::library::{Video, store};
use crate::{cmd, eq, paths, prefs, window};
use gtk::glib;
use gtk::prelude::*;
use mpv::{Data, EndReason, Mpv};
use queue::{Queue, Repeat};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum State {
    Stopped,
    Playing,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// A different video is loaded (or none).
    Track,
    State,
    /// The regular position tick.
    Position,
    /// The position jumped (seek).
    Seeked,
    Queue,
    /// Volume, shuffle, repeat or speed changed.
    Options,
    /// The audio and subtitle tracks, or which are picked, changed.
    Tracks,
    /// The picture's size changed (or there's none).
    Video,
    /// Waiting for data, or not any more.
    Buffering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Sub,
}

/// An audio or subtitle track of the playing file.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackInfo {
    pub kind: TrackKind,
    pub id: i64,
    pub title: String,
    pub lang: String,
    pub codec: String,
    pub channels: i64,
    pub external: bool,
    pub forced: bool,
    pub selected: bool,
}

impl TrackInfo {
    /// "English · 5.1 · eac3", "Commentary (English)", "Track 2".
    pub fn label(&self) -> String {
        let lang = language_name(&self.lang);
        let mut name = match (self.title.is_empty(), lang.is_empty()) {
            (false, false) if !self.title.to_lowercase().contains(&lang.to_lowercase()) => format!("{} ({lang})", self.title),
            (false, _) => self.title.clone(),
            (true, false) => lang,
            (true, true) => format!("Track {}", self.id),
        };
        if self.forced {
            name.push_str(" · forced");
        }
        name
    }

    pub fn detail(&self) -> String {
        let mut parts = Vec::new();
        if self.kind == TrackKind::Audio {
            match self.channels {
                0 => {}
                1 => parts.push("mono".to_string()),
                2 => parts.push("stereo".to_string()),
                6 => parts.push("5.1".to_string()),
                8 => parts.push("7.1".to_string()),
                n => parts.push(format!("{n} ch")),
            }
        }
        if !self.codec.is_empty() {
            parts.push(self.codec.clone());
        }
        if self.external {
            parts.push("file".to_string());
        }
        parts.join(" · ")
    }
}

/// English names for the common ISO 639 codes mpv reports.
pub fn language_name(code: &str) -> String {
    let c = code.to_lowercase();
    let name = match c.as_str() {
        "en" | "eng" => "English",
        "fr" | "fre" | "fra" => "French",
        "de" | "ger" | "deu" => "German",
        "es" | "spa" => "Spanish",
        "it" | "ita" => "Italian",
        "pt" | "por" => "Portuguese",
        "nl" | "dut" | "nld" => "Dutch",
        "sv" | "swe" => "Swedish",
        "no" | "nor" | "nob" => "Norwegian",
        "da" | "dan" => "Danish",
        "fi" | "fin" => "Finnish",
        "pl" | "pol" => "Polish",
        "cs" | "cze" | "ces" => "Czech",
        "hu" | "hun" => "Hungarian",
        "ru" | "rus" => "Russian",
        "uk" | "ukr" => "Ukrainian",
        "el" | "gre" | "ell" => "Greek",
        "tr" | "tur" => "Turkish",
        "ar" | "ara" => "Arabic",
        "he" | "heb" => "Hebrew",
        "hi" | "hin" => "Hindi",
        "ja" | "jpn" => "Japanese",
        "ko" | "kor" => "Korean",
        "zh" | "chi" | "zho" => "Chinese",
        "th" | "tha" => "Thai",
        "vi" | "vie" => "Vietnamese",
        "id" | "ind" => "Indonesian",
        "ro" | "rum" | "ron" => "Romanian",
        "bg" | "bul" => "Bulgarian",
        "hr" | "hrv" => "Croatian",
        "sr" | "srp" => "Serbian",
        "" | "und" => "",
        _ => return code.to_string(),
    };
    name.to_string()
}

struct Player {
    mpv: Option<Mpv>,
    error: Option<String>,
    queue: Queue,
    current: Option<Rc<Video>>,
    state: State,
    position: f64,
    duration: f64,
    tracks: Vec<TrackInfo>,
    chapters: Vec<(f64, String)>,
    video_size: (i32, i32),
    speed: f64,
    sub_delay: f64,
    buffering: bool,
    /// Between `loadfile` and FILE_LOADED.
    loading: bool,
    /// The video reached its end (keep-open holds the last frame).
    ended: bool,
    counted: bool,
    failures: usize,
    last_saved: Instant,
    last_tick: Instant,
    /// The whole-session queue position to restore once the video is ready.
    restore_at: Option<f64>,
}

type Callback = Rc<dyn Fn(Event)>;
type Listener = (glib::WeakRef<gtk::Widget>, Callback);

thread_local! {
    static PLAYER: RefCell<Option<Player>> = const { RefCell::new(None) };
    static LISTENERS: RefCell<Vec<Listener>> = const { RefCell::new(Vec::new()) };
    static GLOBAL: RefCell<Vec<Callback>> = const { RefCell::new(Vec::new()) };
    static SAVE_PENDING: Cell<Option<glib::SourceId>> = const { Cell::new(None) };
}

fn with<R>(f: impl FnOnce(&mut Player) -> R) -> Option<R> {
    PLAYER.with(|p| p.borrow_mut().as_mut().map(f))
}

/// Run `f` with mpv, if it started.
fn with_mpv<R>(f: impl FnOnce(&Mpv) -> R) -> Option<R> {
    with(|p| p.mpv.as_ref().map(f)).flatten()
}

/// Call `f` on every event while `owner` is alive.
pub fn subscribe(owner: &impl IsA<gtk::Widget>, f: impl Fn(Event) + 'static) {
    LISTENERS.with(|l| l.borrow_mut().push((owner.upcast_ref::<gtk::Widget>().downgrade(), Rc::new(f))));
}

/// Call `f` on every event for the life of the app (MPRIS).
pub fn subscribe_global(f: impl Fn(Event) + 'static) {
    GLOBAL.with(|g| g.borrow_mut().push(Rc::new(f)));
}

fn emit(e: Event) {
    let fs: Vec<Callback> = LISTENERS.with(|l| {
        let mut l = l.borrow_mut();
        l.retain(|(w, _)| w.upgrade().is_some());
        l.iter().map(|(_, f)| f.clone()).collect()
    });
    let globals: Vec<Callback> = GLOBAL.with(|g| g.borrow().clone());
    for f in fs.iter().chain(globals.iter()) {
        f(e);
    }
}

/// The audio filter chain: ten equalizer bands and a preamp, adjusted live
/// with `af-command` so moving a fader never interrupts the sound.
fn eq_filter(bands: &[f64; 10], preamp: f64) -> String {
    let mut parts: Vec<String> = eq::FREQUENCIES
        .iter()
        .zip(bands)
        .enumerate()
        .map(|(i, (f, g))| format!("equalizer@b{i}=f={f}:t=o:w=1:g={g:.1}"))
        .collect();
    parts.push(format!("volume@pre=volume={preamp:.1}dB"));
    format!("@eq:lavfi=[{}]", parts.join(","))
}

fn options() -> Vec<(String, String)> {
    let p = prefs::get();
    let mut o: Vec<(String, String)> = [
        ("vo", "libmpv"),
        ("hwdec", if p.hwdec { "auto-safe" } else { "no" }),
        ("keep-open", "yes"),
        ("idle", "yes"),
        ("config", "no"),
        ("terminal", "no"),
        ("input-default-bindings", "no"),
        ("input-vo-keyboard", "no"),
        ("osc", "no"),
        ("osd-level", "0"),
        ("ytdl", "no"),
        ("sub-auto", "fuzzy"),
        ("audio-file-auto", "no"),
        ("audio-client-name", "nexus-media-player"),
        ("save-position-on-quit", "no"),
        ("reset-on-next-file", "speed,sub-delay"),
    ]
    .iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    o.push(("volume".into(), format!("{:.0}", p.volume * 100.0)));
    o.push(("mute".into(), if p.muted { "yes" } else { "no" }.into()));
    o.push(("sub-scale".into(), format!("{:.2}", p.sub_scale)));
    if !p.audio_lang.is_empty() {
        o.push(("alang".into(), p.audio_lang.clone()));
    }
    if !p.sub_lang.is_empty() {
        o.push(("slang".into(), p.sub_lang.clone()));
    }
    if p.eq_enabled {
        o.push(("af".into(), eq_filter(&p.eq_bands, p.eq_preamp)));
    }
    // Developer aid: NMP_AO=null plays silently (in real time) for tests.
    if let Some(ao) = std::env::var("NMP_AO").ok().filter(|s| !s.is_empty()) {
        o.push(("ao".into(), ao));
    }
    o
}

pub fn init() {
    let p = prefs::get();
    let opts = options();
    let refs: Vec<(&str, &str)> = opts.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let (mpv, error, wakeup) = match Mpv::new(&refs) {
        Ok((m, rx)) => (Some(m), None, Some(rx)),
        Err(e) => (None, Some(e), None),
    };
    if let Some(m) = &mpv {
        for (name, format) in [
            ("pause", mpv::FORMAT_FLAG),
            ("eof-reached", mpv::FORMAT_FLAG),
            ("duration", mpv::FORMAT_DOUBLE),
            ("track-list", mpv::FORMAT_NODE),
            ("chapter-list", mpv::FORMAT_NODE),
            ("dwidth", mpv::FORMAT_INT64),
            ("dheight", mpv::FORMAT_INT64),
            ("speed", mpv::FORMAT_DOUBLE),
            ("sub-delay", mpv::FORMAT_DOUBLE),
            ("paused-for-cache", mpv::FORMAT_FLAG),
        ] {
            m.observe(name, format);
        }
    }
    let mut queue = Queue::default();
    queue.repeat = Repeat::from_id(&p.repeat);
    queue.set_shuffle(p.shuffle);
    PLAYER.with(|cell| {
        *cell.borrow_mut() = Some(Player {
            mpv,
            error,
            queue,
            current: None,
            state: State::Stopped,
            position: 0.0,
            duration: 0.0,
            tracks: Vec::new(),
            chapters: Vec::new(),
            video_size: (0, 0),
            speed: 1.0,
            sub_delay: 0.0,
            buffering: false,
            loading: false,
            ended: false,
            counted: false,
            failures: 0,
            last_saved: Instant::now(),
            last_tick: Instant::now(),
            restore_at: None,
        })
    });
    if let Some(rx) = wakeup {
        glib::spawn_future_local(async move {
            while rx.recv().await.is_ok() {
                drain_events();
            }
        });
    }
    glib::timeout_add_local(std::time::Duration::from_millis(250), || {
        tick();
        glib::ControlFlow::Continue
    });
}

/// Set up video drawing on the window (called once it's realized), then bring
/// back the last session's queue.
pub fn attach(window: &gtk::ApplicationWindow) {
    if video::ready() {
        return;
    }
    // Snapshots (a developer aid) render offscreen, which our GL textures
    // don't survive; they show the pages without video.
    if std::env::var_os("NMP_SNAPSHOT").is_some() {
        if prefs::get().restore_queue {
            restore();
        }
        return;
    }
    let result = with(|p| p.mpv.as_ref().map(|m| video::init(window, m, video_size_for_render))).flatten();
    if let Some(Err(e)) = result {
        // Without our own drawing, let mpv open a window of its own.
        with_mpv(|m| {
            let _ = m.set_str("vo", "gpu");
        });
        window::toast(&format!("Video can't be drawn in this window ({e}); it will open in its own."));
    }
    if prefs::get().restore_queue {
        restore();
    }
}

/// The size to draw frames at: the picture's own (display) size. The first
/// frames can arrive before the size is announced, so ask mpv then.
fn video_size_for_render() -> (i32, i32) {
    let (w, h) = video_size();
    if w > 0 && h > 0 {
        return (w, h);
    }
    let asked = with_mpv(|m| (m.get_i64("dwidth").unwrap_or(0) as i32, m.get_i64("dheight").unwrap_or(0) as i32));
    match asked {
        Some((w, h)) if w > 0 && h > 0 => (w, h),
        _ => (16, 16),
    }
}

// ---------- Reading ----------

pub fn engine_error() -> Option<String> {
    with(|p| p.error.clone()).flatten()
}

pub fn current() -> Option<Rc<Video>> {
    with(|p| p.current.clone()).flatten()
}

pub fn state() -> State {
    with(|p| p.state).unwrap_or(State::Stopped)
}

pub fn position() -> f64 {
    with(|p| p.position).unwrap_or(0.0)
}

pub fn duration() -> f64 {
    with(|p| p.duration).unwrap_or(0.0)
}

pub fn video_size() -> (i32, i32) {
    with(|p| p.video_size).unwrap_or((0, 0))
}

pub fn has_video() -> bool {
    let (w, h) = video_size();
    w > 0 && h > 0
}

pub fn tracks(kind: TrackKind) -> Vec<TrackInfo> {
    with(|p| p.tracks.iter().filter(|t| t.kind == kind).cloned().collect()).unwrap_or_default()
}

pub fn chapters() -> Vec<(f64, String)> {
    with(|p| p.chapters.clone()).unwrap_or_default()
}

pub fn speed() -> f64 {
    with(|p| p.speed).unwrap_or(1.0)
}

pub fn sub_delay() -> f64 {
    with(|p| p.sub_delay).unwrap_or(0.0)
}

pub fn buffering() -> bool {
    with(|p| p.buffering).unwrap_or(false)
}

pub fn queue_items() -> (Vec<PathBuf>, Option<usize>) {
    with(|p| (p.queue.items.clone(), p.queue.cursor)).unwrap_or_default()
}

pub fn shuffle() -> bool {
    with(|p| p.queue.shuffled()).unwrap_or(false)
}

pub fn repeat() -> Repeat {
    with(|p| p.queue.repeat).unwrap_or(Repeat::Off)
}

pub fn can_next() -> bool {
    with(|p| p.queue.cursor.is_some_and(|c| c + 1 < p.queue.items.len() || p.queue.repeat != Repeat::Off)).unwrap_or(false)
        || current().is_some_and(|v| prefs::get().autoplay_next && store::next_episode(&v).is_some())
}

pub fn can_previous() -> bool {
    with(|p| p.current.is_some()).unwrap_or(false)
}

// ---------- mpv events ----------

fn drain_events() {
    loop {
        let Some(e) = with_mpv(|m| m.next_event()).flatten() else { return };
        on_event(e);
    }
}

fn parse_tracks(v: &Value) -> Vec<TrackInfo> {
    let Some(list) = v.as_array() else { return Vec::new() };
    list.iter()
        .filter_map(|t| {
            let kind = match t.get("type")?.as_str()? {
                "audio" => TrackKind::Audio,
                "sub" => TrackKind::Sub,
                _ => return None,
            };
            let s = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let b = |k: &str| t.get(k).and_then(Value::as_bool).unwrap_or(false);
            Some(TrackInfo {
                kind,
                id: t.get("id")?.as_i64()?,
                title: s("title"),
                lang: s("lang"),
                codec: s("codec"),
                channels: t.get("demux-channel-count").and_then(Value::as_i64).unwrap_or(0),
                external: b("external"),
                forced: b("forced"),
                selected: b("selected"),
            })
        })
        .collect()
}

fn on_event(e: mpv::Event) {
    match e {
        mpv::Event::FileLoaded => {
            with(|p| {
                p.loading = false;
                p.ended = false;
                p.failures = 0;
            });
            emit(Event::Seeked);
            emit(Event::Tracks);
        }
        mpv::Event::EndFile { reason: EndReason::Error, error } => {
            let name = current().map(|v| v.label()).unwrap_or_default();
            window::toast(&format!("Couldn't play “{name}”: {}", error.unwrap_or_default()));
            let skip = with(|p| {
                p.loading = false;
                p.failures += 1;
                p.failures < p.queue.items.len()
            })
            .unwrap_or(false);
            if skip && with(|p| p.queue.advance(true)).flatten().is_some() {
                load_current(true);
            } else {
                stop();
            }
        }
        mpv::Event::PlaybackRestart => {
            with(|p| p.buffering = false);
            emit(Event::Seeked);
        }
        mpv::Event::Property { name, data } => on_property(&name, data),
        _ => {}
    }
}

fn on_property(name: &str, data: Data) {
    match (name, data) {
        ("eof-reached", Data::Flag(true)) => {
            let fresh = with(|p| !p.loading && !std::mem::replace(&mut p.ended, true)).unwrap_or(false);
            if fresh {
                finished();
            }
        }
        ("pause", Data::Flag(paused)) => {
            let changed = with(|p| {
                if p.state == State::Stopped || p.loading {
                    return false;
                }
                let s = if paused { State::Paused } else { State::Playing };
                std::mem::replace(&mut p.state, s) != s
            })
            .unwrap_or(false);
            if changed {
                emit(Event::State);
            }
        }
        ("duration", Data::Double(d)) => {
            with(|p| p.duration = d);
            emit(Event::Seeked);
        }
        ("track-list", Data::Node(v)) => {
            with(|p| p.tracks = parse_tracks(&v));
            emit(Event::Tracks);
        }
        ("chapter-list", Data::Node(v)) => {
            let chapters = v
                .as_array()
                .map(|l| {
                    l.iter()
                        .filter_map(|c| {
                            Some((
                                c.get("time")?.as_f64()?,
                                c.get("title").and_then(Value::as_str).unwrap_or_default().to_string(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default();
            with(|p| p.chapters = chapters);
            emit(Event::Tracks);
        }
        ("dwidth", d) | ("dheight", d) => {
            let n = match d {
                Data::Int(n) => n as i32,
                _ => 0,
            };
            let changed = with(|p| {
                let old = p.video_size;
                if name == "dwidth" {
                    p.video_size.0 = n;
                } else {
                    p.video_size.1 = n;
                }
                old != p.video_size
            })
            .unwrap_or(false);
            if changed {
                if n == 0 {
                    video::clear();
                }
                emit(Event::Video);
            }
        }
        ("speed", Data::Double(s)) => {
            with(|p| p.speed = s);
            emit(Event::Options);
        }
        ("sub-delay", Data::Double(s)) => {
            with(|p| p.sub_delay = s);
            emit(Event::Tracks);
        }
        ("paused-for-cache", Data::Flag(b)) => {
            with(|p| p.buffering = b);
            emit(Event::Buffering);
        }
        _ => {}
    }
}

/// The current video ended on its own.
fn finished() {
    if let Some(v) = current() {
        store::update_watch(&v, |w| {
            if !w.watched {
                w.plays += 1;
            }
            w.watched = true;
            w.position = 0.0;
            w.last_played = crate::library::now();
        });
    }
    if with(|p| p.queue.advance(false)).flatten().is_some() {
        load_current(true);
        return;
    }
    // Nothing queued after it: a show carries on with its next episode.
    if prefs::get().autoplay_next
        && let Some(next) = current().and_then(|v| store::next_episode(&v))
    {
        with(|p| p.queue.append(std::slice::from_ref(&next.path)));
        if with(|p| p.queue.advance(false)).flatten().is_some() {
            load_current(true);
            window::toast(&format!("Up next: {}", next.label()));
            return;
        }
    }
    with(|p| {
        p.state = State::Paused;
        p.position = p.duration;
    });
    emit(Event::State);
    emit(Event::Seeked);
}

/// Point `current` at the queue's video and reset the per-video counters.
fn adopt_current() {
    let path = with(|p| p.queue.current().cloned()).flatten();
    let video = path.map(|p| store::video_for(&p));
    with(|p| {
        p.duration = video.as_ref().map_or(0.0, |v| v.duration);
        p.current = video;
        p.position = 0.0;
        p.counted = false;
        p.ended = false;
        p.tracks.clear();
        p.chapters.clear();
    });
    schedule_save();
}

/// Where a video should start: where it was left, unless that's barely in or
/// nearly at the end.
pub fn resume_point(position: f64, duration: f64) -> Option<f64> {
    if position < 30.0 {
        return None;
    }
    if duration > 0.0 && position > duration * 0.95 {
        return None;
    }
    Some((position - 3.0).max(0.0))
}

fn load_current(play: bool) {
    save_progress();
    adopt_current();
    let Some(v) = current() else {
        stop();
        return;
    };
    let p = prefs::get();
    // Before the library has loaded (a file from the command line), read how
    // far it got straight from the database.
    if v.id == 0
        && let Ok(Some(saved)) = crate::library::db::open().and_then(|c| crate::library::db::load_watch(&c, &v.path))
    {
        v.watch.set(saved);
    }
    let w = v.watch.get();
    let mut opts: Vec<String> = Vec::new();
    let restore = with(|pl| pl.restore_at.take()).flatten();
    let start = restore.or_else(|| if p.resume { resume_point(w.position, v.duration) } else { None });
    if let Some(s) = start {
        opts.push(format!("start={s:.2}"));
        with(|pl| pl.position = s);
    }
    match w.aid {
        Some(id) if id > 0 => opts.push(format!("aid={id}")),
        _ => {}
    }
    match w.sid {
        Some(0) => opts.push("sid=no".into()),
        Some(id) => opts.push(format!("sid={id}")),
        None if !p.subtitles => opts.push("sid=no".into()),
        None => {}
    }
    if w.sub_delay.abs() > 0.001 {
        opts.push(format!("sub-delay={:.2}", w.sub_delay));
    }
    if (p.speed - 1.0).abs() > 0.001 {
        opts.push(format!("speed={:.2}", p.speed));
    }
    let uri = v.file_uri();
    video::clear();
    with(|pl| {
        pl.loading = true;
        pl.state = if play { State::Playing } else { State::Paused };
        pl.last_tick = Instant::now();
        pl.last_saved = Instant::now();
        if let Some(m) = pl.mpv.as_ref() {
            let _ = m.set_flag("pause", !play);
            let o = opts.join(",");
            if let Err(e) = m.command(&["loadfile", &uri, "replace", "-1", &o]) {
                pl.error = Some(e);
            }
        }
    });
    if play {
        store::update_watch(&v, |w| w.last_played = crate::library::now());
        scrub::prepare(&v);
    }
    queue_changed();
    emit(Event::Track);
    emit(Event::State);
    emit(Event::Seeked);
    if play {
        notify_video();
    }
}

fn tick() {
    let (playing, save) = with(|p| {
        let now = Instant::now();
        p.last_tick = now;
        if p.state != State::Playing || p.loading {
            return (false, false);
        }
        if let Some(pos) = p.mpv.as_ref().and_then(|m| m.get_f64("time-pos")) {
            p.position = pos;
        }
        let save = now.duration_since(p.last_saved).as_secs_f64() >= 5.0;
        if save {
            p.last_saved = now;
        }
        (true, save)
    })
    .unwrap_or((false, false));
    if !playing {
        return;
    }
    emit(Event::Position);
    // Mark watched once most of it has been seen.
    let (pos, dur) = (position(), duration());
    let watched_now = with(|p| {
        if !p.counted && dur > 0.0 && pos >= dur * 0.9 {
            p.counted = true;
            return true;
        }
        false
    })
    .unwrap_or(false);
    if watched_now && let Some(v) = current() {
        store::update_watch(&v, |w| {
            if !w.watched {
                w.plays += 1;
            }
            w.watched = true;
        });
    }
    if save {
        save_progress();
    }
}

/// Write the current video's position (so it resumes there next time).
fn save_progress() {
    let Some(v) = current() else { return };
    let (pos, dur, loading) = with(|p| (p.position, p.duration.max(v.duration), p.loading)).unwrap_or_default();
    if loading || pos <= 0.0 {
        return;
    }
    let mut w = v.watch.get();
    if dur > 0.0 && pos >= dur * 0.9 {
        w.position = 0.0;
    } else if pos >= 30.0 {
        w.position = pos;
        // Watching again after finishing: it's in progress once more.
        if w.watched && pos < dur * 0.5 {
            w.watched = false;
        }
    }
    w.last_played = crate::library::now();
    if w != v.watch.get() {
        v.watch.set(w);
        store::persist_watch(&v);
        store::watch_changed();
    }
}

// ---------- Commands ----------

pub fn play_paths(paths: Vec<PathBuf>, start: usize) {
    if paths.is_empty() {
        return;
    }
    with(|p| {
        p.queue.set(paths, start);
        p.failures = 0;
    });
    load_current(true);
}

/// Shuffle a list into a fresh queue.
pub fn shuffle_paths(paths: Vec<PathBuf>) {
    if paths.is_empty() {
        return;
    }
    set_shuffle(true);
    let start = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize)
        % paths.len();
    play_paths(paths, start);
}

pub fn enqueue(paths: Vec<PathBuf>) {
    with(|p| p.queue.append(&paths));
    queue_changed();
}

pub fn play_next(paths: Vec<PathBuf>) {
    with(|p| p.queue.insert_next(&paths));
    queue_changed();
}

pub fn play() {
    match state() {
        State::Playing => {}
        State::Paused => {
            let ended = with(|p| p.ended).unwrap_or(false);
            if ended {
                // Play after the end starts it over.
                with(|p| p.ended = false);
                seek(0.0);
            }
            with(|p| {
                if let Some(m) = p.mpv.as_ref() {
                    let _ = m.set_flag("pause", false);
                }
                p.state = State::Playing;
                p.last_tick = Instant::now();
            });
            emit(Event::State);
        }
        State::Stopped => {
            if with(|p| p.queue.current().is_some()).unwrap_or(false) {
                load_current(true);
            }
        }
    }
}

pub fn pause() {
    if state() != State::Playing {
        return;
    }
    with(|p| {
        if let Some(m) = p.mpv.as_ref() {
            let _ = m.set_flag("pause", true);
        }
        p.state = State::Paused;
    });
    save_progress();
    emit(Event::State);
    schedule_save();
}

pub fn toggle() {
    if state() == State::Playing { pause() } else { play() }
}

pub fn stop() {
    save_progress();
    with(|p| {
        if let Some(m) = p.mpv.as_ref() {
            let _ = m.command(&["stop"]);
        }
        p.state = State::Stopped;
        p.position = 0.0;
        p.loading = false;
        p.video_size = (0, 0);
        p.tracks.clear();
    });
    video::clear();
    emit(Event::State);
    emit(Event::Seeked);
    emit(Event::Video);
}

pub fn next() {
    let playing = state() != State::Paused;
    if with(|p| p.queue.advance(true)).flatten().is_some() {
        load_current(playing);
    } else if let Some(n) = current().filter(|_| prefs::get().autoplay_next).and_then(|v| store::next_episode(&v)) {
        with(|p| p.queue.append(std::slice::from_ref(&n.path)));
        if with(|p| p.queue.advance(true)).flatten().is_some() {
            load_current(playing);
        }
    }
}

pub fn previous() {
    if position() > 5.0 {
        seek(0.0);
        return;
    }
    let playing = state() != State::Paused;
    if with(|p| p.queue.previous()).flatten().is_some() {
        load_current(playing);
    }
}

/// Seek exactly to `secs`.
pub fn seek(secs: f64) {
    seek_with(secs, "absolute+exact");
}

/// Seek to the nearest keyframe: fast, for scrubbing and swiping.
pub fn seek_fast(secs: f64) {
    seek_with(secs, "absolute+keyframes");
}

fn seek_with(secs: f64, flags: &str) {
    if current().is_none() {
        return;
    }
    let d = duration();
    let secs = if d > 0.0 { secs.clamp(0.0, (d - 0.1).max(0.0)) } else { secs.max(0.0) };
    with(|p| {
        p.ended = false;
        if let Some(m) = p.mpv.as_ref() {
            let _ = m.command(&["seek", &format!("{secs:.3}"), flags]);
        }
        p.position = secs;
    });
    emit(Event::Seeked);
}

pub fn seek_by(delta: f64) {
    if current().is_some() {
        seek(position() + delta);
    }
}

/// One frame forward or back (pauses).
pub fn frame_step(forward: bool) {
    with(|p| {
        if let Some(m) = p.mpv.as_ref() {
            let _ = m.command(&[if forward { "frame-step" } else { "frame-back-step" }]);
        }
        if p.state == State::Playing {
            p.state = State::Paused;
        }
    });
    emit(Event::State);
    glib::timeout_add_local_once(std::time::Duration::from_millis(80), || {
        if let Some(pos) = with_mpv(|m| m.get_f64("time-pos")).flatten() {
            with(|p| p.position = pos);
            emit(Event::Seeked);
        }
    });
}

pub fn jump(index: usize) {
    if with(|p| p.queue.jump(index)).flatten().is_some() {
        load_current(true);
    }
}

pub fn remove(index: usize) {
    let was_current = with(|p| p.queue.cursor == Some(index)).unwrap_or(false);
    with(|p| p.queue.remove(index));
    if was_current {
        if with(|p| p.queue.current().is_some()).unwrap_or(false) {
            load_current(state() == State::Playing);
        } else {
            stop();
            with(|p| p.current = None);
            emit(Event::Track);
        }
    }
    queue_changed();
}

pub fn move_item(from: usize, to: usize) {
    with(|p| p.queue.move_item(from, to));
    queue_changed();
}

pub fn clear() {
    stop();
    with(|p| {
        p.queue.clear();
        p.current = None;
    });
    emit(Event::Track);
    queue_changed();
}

pub fn set_volume(v: f64) {
    let v = v.clamp(0.0, 1.0);
    prefs::update(|p| p.volume = v);
    with_mpv(|m| m.set_f64("volume", v * 100.0));
    emit(Event::Options);
}

pub fn set_muted(muted: bool) {
    prefs::update(|p| p.muted = muted);
    with_mpv(|m| m.set_flag("mute", muted));
    emit(Event::Options);
}

pub fn set_shuffle(on: bool) {
    prefs::update(|p| p.shuffle = on);
    with(|p| p.queue.set_shuffle(on));
    queue_changed();
    emit(Event::Options);
}

pub fn set_repeat(r: Repeat) {
    prefs::update(|p| p.repeat = r.id().to_string());
    with(|p| p.queue.repeat = r);
    queue_changed();
    emit(Event::Options);
}

/// The speeds the speed control offers.
pub const SPEEDS: [f64; 9] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0, 3.0];

pub fn set_speed(s: f64) {
    let s = s.clamp(0.25, 3.0);
    with_mpv(|m| m.set_f64("speed", s));
    with(|p| p.speed = s);
    emit(Event::Options);
}

/// One step faster or slower through [`SPEEDS`].
pub fn step_speed(faster: bool) {
    let now = speed();
    let next = if faster {
        SPEEDS.iter().copied().find(|s| *s > now + 0.001).unwrap_or(3.0)
    } else {
        SPEEDS.iter().rev().copied().find(|s| *s < now - 0.001).unwrap_or(0.25)
    };
    set_speed(next);
    window::flash(&format!("{}×", crate::fmt::speed(next)));
}

/// Pick an audio track (and remember it for this video).
pub fn set_audio(id: i64) {
    with_mpv(|m| m.set_i64("aid", id));
    if let Some(v) = current() {
        store::update_watch(&v, |w| w.aid = Some(id));
    }
}

/// Pick a subtitle track, or 0 for none (and remember it for this video).
pub fn set_subtitle(id: i64) {
    with_mpv(|m| if id == 0 { m.set_str("sid", "no") } else { m.set_i64("sid", id) });
    if let Some(v) = current() {
        store::update_watch(&v, |w| w.sid = Some(id));
    }
}

/// Load a subtitle file for the current video and show it.
pub fn add_subtitle(path: &std::path::Path) {
    let p = path.to_string_lossy().to_string();
    match with_mpv(|m| m.command(&["sub-add", &p, "select"])) {
        Some(Ok(())) => window::toast(&format!("Loaded {}.", paths::pretty(path))),
        Some(Err(e)) => window::toast(&format!("Couldn't load those subtitles: {e}")),
        None => {}
    }
}

pub fn set_sub_delay(secs: f64) {
    let secs = (secs * 10.0).round() / 10.0;
    with_mpv(|m| m.set_f64("sub-delay", secs));
    with(|p| p.sub_delay = secs);
    if let Some(v) = current() {
        store::update_watch(&v, |w| w.sub_delay = secs);
    }
    emit(Event::Tracks);
}

pub fn nudge_sub_delay(delta: f64) {
    set_sub_delay(sub_delay() + delta);
    window::flash(&format!("Subtitles {}", crate::fmt::delay(sub_delay())));
}

pub fn set_sub_scale(scale: f64) {
    let scale = scale.clamp(0.5, 2.5);
    prefs::update(|p| p.sub_scale = scale);
    with_mpv(|m| m.set_f64("sub-scale", scale));
}

/// Push the equalizer settings to mpv: the whole chain when it's switched on
/// or off, live commands for fader moves.
pub fn apply_eq() {
    let p = prefs::get();
    let current_af = with_mpv(|m| m.get_str("af")).flatten().unwrap_or_default();
    let has = current_af.contains("@eq");
    if !p.eq_enabled {
        if has {
            with_mpv(|m| m.set_str("af", ""));
        }
        return;
    }
    if !has {
        with_mpv(|m| m.set_str("af", &eq_filter(&p.eq_bands, p.eq_preamp)));
        return;
    }
    with_mpv(|m| {
        for (i, g) in p.eq_bands.iter().enumerate() {
            let _ = m.command(&["af-command", "eq", "g", &format!("{g:.1}"), &format!("equalizer@b{i}")]);
        }
        let _ = m.command(&["af-command", "eq", "volume", &format!("{:.1}dB", p.eq_preamp), "volume@pre"]);
    });
}

/// Preferred track languages, for the next videos.
pub fn apply_languages() {
    let p = prefs::get();
    with_mpv(|m| {
        let _ = m.set_str("alang", &p.audio_lang);
        let _ = m.set_str("slang", &p.sub_lang);
    });
}

pub fn set_hwdec(on: bool) {
    with_mpv(|m| m.set_str("hwdec", if on { "auto-safe" } else { "no" }));
}

/// Tell the queue views and MPRIS, and save.
fn queue_changed() {
    schedule_save();
    emit(Event::Queue);
}

/// The library was (re)loaded: pick up the library's copy of the current video.
pub fn library_changed() {
    let changed = with(|p| {
        let cur = p.current.as_ref()?;
        let fresh = store::find(&cur.path)?;
        if Rc::ptr_eq(cur, &fresh) {
            return None;
        }
        // Keep what playing has learned since.
        fresh.watch.set(cur.watch.get());
        p.current = Some(fresh);
        Some(())
    })
    .flatten()
    .is_some();
    if changed {
        emit(Event::Track);
    }
}

// ---------- Notifications ----------

fn notify_video() {
    if !prefs::get().notify || window::is_active() {
        return;
    }
    let Some(v) = current() else { return };
    if !cmd::present("notify-send") {
        return;
    }
    let art = v.card_art();
    let icon = if art.is_empty() {
        "video-x-generic".to_string()
    } else {
        crate::library::art::file(art, true).to_string_lossy().into_owned()
    };
    cmd::spawn(&[
        "notify-send",
        "--app-name=Nexus Media Player",
        &format!("--icon={icon}"),
        "--hint=string:x-canonical-private-synchronous:nexus-media-player",
        "--expire-time=4000",
        "Now playing",
        &v.label(),
    ]);
}

// ---------- Saved state ----------

#[derive(Serialize, Deserialize, Default)]
struct Saved {
    items: Vec<PathBuf>,
    cursor: Option<usize>,
    original: Option<Vec<PathBuf>>,
    position: f64,
}

fn schedule_save() {
    if let Some(id) = SAVE_PENDING.with(|s| s.take()) {
        id.remove();
    }
    let id = glib::timeout_add_local_once(std::time::Duration::from_millis(1000), || {
        SAVE_PENDING.with(|s| s.set(None));
        save();
    });
    SAVE_PENDING.with(|s| s.set(Some(id)));
}

/// Write the queue and position now (also called on quit).
pub fn save() {
    save_progress();
    let saved = with(|p| Saved {
        items: p.queue.items.clone(),
        cursor: p.queue.cursor,
        original: p.queue.original().cloned(),
        position: p.position,
    });
    if let Some(s) = saved
        && let Ok(text) = serde_json::to_string(&s)
    {
        let _ = cmd::atomic_write(&paths::state_file(), &text);
    }
}

fn restore() {
    let Some(saved) = std::fs::read_to_string(paths::state_file()).ok().and_then(|t| serde_json::from_str::<Saved>(&t).ok())
    else {
        return;
    };
    let shuffled = prefs::get().shuffle;
    with(|p| {
        p.queue.restore(saved.items, saved.cursor, if shuffled { saved.original } else { None });
        if shuffled && p.queue.original().is_none() {
            p.queue.set_shuffle(true);
        }
    });
    if with(|p| p.queue.current().is_some_and(|c| c.exists())).unwrap_or(false) {
        if saved.position > 1.0 {
            with(|p| p.restore_at = Some(saved.position));
        }
        load_current(false);
    } else {
        emit(Event::Queue);
    }
}

/// Stop drawing and close mpv (on quit).
pub fn shutdown() {
    save();
    video::shutdown();
    PLAYER.with(|p| {
        if let Some(p) = p.borrow_mut().as_mut() {
            p.mpv = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resumes_only_in_the_middle() {
        assert_eq!(resume_point(10.0, 3600.0), None);
        assert_eq!(resume_point(600.0, 3600.0), Some(597.0));
        assert_eq!(resume_point(3500.0, 3600.0), None);
        assert_eq!(resume_point(120.0, 0.0), Some(117.0));
    }

    #[test]
    fn builds_the_equalizer_chain() {
        let f = eq_filter(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, -2.5], -3.0);
        assert!(f.starts_with("@eq:lavfi=[equalizer@b0=f=31:t=o:w=1:g=1.0,"));
        assert!(f.contains("equalizer@b9=f=16000:t=o:w=1:g=-2.5"));
        assert!(f.ends_with("volume@pre=volume=-3.0dB]"));
    }

    /// mpv accepts the equalizer chain and adjusts its bands live.
    #[test]
    fn mpv_takes_the_equalizer() {
        let (m, _rx) = Mpv::new(&[("vo", "null"), ("ao", "null"), ("config", "no"), ("terminal", "no"), ("idle", "yes")])
            .expect("libmpv starts");
        m.set_str("af", &eq_filter(&[3.0; 10], -3.0)).expect("chain parses");
        m.command(&["loadfile", "av://lavfi:sine=frequency=440:duration=5"]).expect("loads");
        // Wait for the audio chain to come up.
        let start = Instant::now();
        while m.get_f64("time-pos").is_none_or(|t| t <= 0.0) && start.elapsed().as_secs() < 5 {
            while m.next_event().is_some() {}
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        m.command(&["af-command", "eq", "g", "-4.5", "equalizer@b3"]).expect("band command");
        m.command(&["af-command", "eq", "volume", "-6.0dB", "volume@pre"]).expect("preamp command");
        assert!(m.get_str("af").unwrap_or_default().contains("@eq"));
    }

    #[test]
    fn reads_tracks() {
        let v: Value = serde_json::from_str(
            r#"[{"id":1,"type":"video","codec":"h264"},
                {"id":1,"type":"audio","lang":"eng","codec":"eac3","demux-channel-count":6,"selected":true},
                {"id":2,"type":"audio","lang":"jpn","title":"Commentary","codec":"aac","demux-channel-count":2},
                {"id":1,"type":"sub","lang":"eng","forced":true,"codec":"subrip"},
                {"id":2,"type":"sub","title":"Signs","external":true,"codec":"ass"}]"#,
        )
        .unwrap();
        let t = parse_tracks(&v);
        assert_eq!(t.len(), 4);
        assert_eq!(t[0].label(), "English");
        assert_eq!(t[0].detail(), "5.1 · eac3");
        assert!(t[0].selected);
        assert_eq!(t[1].label(), "Commentary (Japanese)");
        assert_eq!(t[2].label(), "English · forced");
        assert_eq!(t[3].label(), "Signs");
        assert_eq!(t[3].detail(), "ass · file");
    }
}
