//! Now playing: the video, with controls that float over it and fade away
//! while it plays. Click to pause, double-click for fullscreen, swipe sideways
//! on a touchpad to seek and up or down for the volume.

use crate::library::Kind;
use crate::library::art::Art;
use crate::player::{self, Event, State, TrackKind, video};
use crate::seekbar::SeekBar;
use crate::widgets::{self, Page};
use crate::{fmt, prefs, window};
use gtk::prelude::*;
use gtk::{gdk, gio, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Touchpad seeking: seconds per unit of horizontal scroll.
const SECS_PER_UNIT: f64 = 0.3;
const HIDE_AFTER: Duration = Duration::from_millis(2500);

thread_local! {
    /// Fill the window, cropping the picture, instead of fitting inside it.
    static FILL: Cell<bool> = const { Cell::new(false) };
    static FLASH: RefCell<Option<(gtk::Label, Rc<Cell<u32>>)>> = const { RefCell::new(None) };
}

/// Show a short message over the video (seek, volume, speed).
pub fn flash(text: &str) {
    FLASH.with(|f| {
        let Some((label, generation)) = f.borrow().clone() else { return };
        if !label.is_mapped() {
            return;
        }
        label.set_text(text);
        label.set_visible(true);
        let g = generation.get().wrapping_add(1);
        generation.set(g);
        glib::timeout_add_local_once(Duration::from_millis(1100), move || {
            if generation.get() == g {
                label.set_visible(false);
            }
        });
    });
}

fn volume_icon(v: f64, muted: bool) -> &'static str {
    if muted || v <= 0.001 {
        "audio-volume-muted-symbolic"
    } else if v < 0.34 {
        "audio-volume-low-symbolic"
    } else if v < 0.67 {
        "audio-volume-medium-symbolic"
    } else {
        "audio-volume-high-symbolic"
    }
}

/// A row in a track popover: a check when picked.
fn choice(label: &str, detail: &str, selected: bool, on: impl Fn() + 'static) -> gtk::Button {
    let b = gtk::Button::new();
    b.add_css_class("flat");
    b.add_css_class("menu-item");
    let row = widgets::hbox(10);
    let check = gtk::Image::from_icon_name("object-select-symbolic");
    check.set_opacity(if selected { 1.0 } else { 0.0 });
    check.add_css_class("accent-text");
    row.append(&check);
    let l = widgets::label(label, "");
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    l.set_max_width_chars(32);
    row.append(&l);
    if !detail.is_empty() {
        let d = widgets::label(detail, "dim");
        d.add_css_class("mono");
        row.append(&d);
    }
    b.set_child(Some(&row));
    b.connect_clicked(move |_| on());
    b
}

fn clear(b: &gtk::Box) {
    while let Some(c) = b.first_child() {
        b.remove(&c);
    }
}

/// A button that opens a popover filled fresh each time.
fn menu_button(icon: &str, tooltip: &str, fill: impl Fn(&gtk::Box, &gtk::Popover) + 'static) -> gtk::MenuButton {
    let mb = gtk::MenuButton::new();
    mb.set_icon_name(icon);
    mb.set_tooltip_text(Some(tooltip));
    mb.add_css_class("flat");
    mb.add_css_class("osd-button");
    mb.set_focus_on_click(false);
    let pop = gtk::Popover::new();
    pop.add_css_class("menu-popover");
    let content = widgets::vbox(1);
    pop.set_child(Some(&content));
    mb.set_popover(Some(&pop));
    let p2 = pop.clone();
    pop.connect_show(move |_| {
        clear(&content);
        fill(&content, &p2);
    });
    mb
}

/// A popover row: Delay  −  value  +
fn delay_row(tips: (&str, &str), step: f64, get: fn() -> f64, set: fn(f64), show: fn(f64) -> String) -> gtk::Box {
    let row = widgets::hbox(6);
    row.add_css_class("menu-row");
    let l = widgets::label("Delay", "");
    l.set_hexpand(true);
    row.append(&l);
    let minus = widgets::icon_button("list-remove-symbolic", tips.0);
    let value = widgets::label(&show(get()), "value-readout");
    value.set_width_chars(7);
    value.set_xalign(0.5);
    let plus = widgets::icon_button("list-add-symbolic", tips.1);
    for (b, d) in [(&minus, -step), (&plus, step)] {
        let v = value.clone();
        b.connect_clicked(move |_| {
            set(get() + d);
            v.set_text(&show(get()));
        });
    }
    row.append(&minus);
    row.append(&value);
    row.append(&plus);
    row
}

fn chapter_menu(content: &gtk::Box, pop: &gtk::Popover) {
    content.append(&widgets::label("CHAPTERS", "popover-heading"));
    let now = player::current_chapter();
    for (i, (t, name)) in player::chapters().into_iter().enumerate() {
        let pop = pop.clone();
        let label = if name.is_empty() { format!("Chapter {}", i + 1) } else { name };
        content.append(&choice(&label, &fmt::time(t), now == Some(i), move || {
            player::seek(t);
            pop.popdown();
        }));
    }
}

fn picture_menu(content: &gtk::Box, pop: &gtk::Popover, picture: &gtk::Picture) {
    content.append(&widgets::label("SHAPE", "popover-heading"));
    let now = player::aspect();
    for (ratio, label) in [("no", "As made"), ("16:9", "16:9"), ("4:3", "4:3"), ("2.35:1", "2.35:1"), ("1:1", "Square")] {
        let pop = pop.clone();
        let picked = if ratio == "no" { now == "no" || now == "-1" } else { now == ratio };
        content.append(&choice(label, "", picked, move || {
            player::set_aspect(ratio);
            pop.popdown();
        }));
    }
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let fill = FILL.with(|f| f.get());
    let (pop2, pic) = (pop.clone(), picture.clone());
    content.append(&choice("Fill the window", "crop", fill, move || {
        FILL.with(|f| f.set(!fill));
        pic.set_content_fit(if fill { gtk::ContentFit::Contain } else { gtk::ContentFit::Cover });
        pop2.popdown();
    }));
    let turn = player::rotation();
    let p = pop.clone();
    content.append(&choice("Turn right", &format!("{turn}°"), turn != 0, move || {
        player::set_rotation(turn + 90);
        p.popdown();
    }));
    let de = player::deinterlace();
    let p = pop.clone();
    content.append(&choice("Deinterlace", "", de, move || {
        player::set_deinterlace(!de);
        p.popdown();
    }));
}

fn speed_menu(content: &gtk::Box, pop: &gtk::Popover) {
    content.append(&widgets::label("SPEED", "popover-heading"));
    let now = player::speed();
    for s in player::SPEEDS {
        let pop = pop.clone();
        content.append(&choice(
            &format!("{}×", fmt::speed(s)),
            if s == 1.0 { "normal" } else { "" },
            (now - s).abs() < 0.01,
            move || {
                player::set_speed(s);
                pop.popdown();
            },
        ));
    }
}

fn audio_menu(content: &gtk::Box, pop: &gtk::Popover) {
    content.append(&widgets::label("SOUND", "popover-heading"));
    let tracks = player::tracks(TrackKind::Audio);
    if tracks.is_empty() {
        content.append(&widgets::label("No sound", "dim"));
    }
    for t in tracks {
        let pop = pop.clone();
        let id = t.id;
        content.append(&choice(&t.label(), &t.detail(), t.selected, move || {
            player::set_audio(id);
            pop.popdown();
        }));
    }
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    content.append(&delay_row(
        ("Sound earlier (Ctrl+−)", "Sound later (Ctrl++)"),
        0.05,
        player::audio_delay,
        player::set_audio_delay,
        fmt::delay_ms,
    ));
    let night = prefs::get().night_mode;
    let p = pop.clone();
    content.append(&choice("Night mode", "N", night, move || {
        player::set_night_mode(!night);
        p.popdown();
    }));
}

fn subtitle_menu(content: &gtk::Box, pop: &gtk::Popover) {
    content.append(&widgets::label("SUBTITLES", "popover-heading"));
    let tracks = player::tracks(TrackKind::Sub);
    let any = tracks.iter().any(|t| t.selected);
    let p = pop.clone();
    content.append(&choice("Off", "", !any, move || {
        player::set_subtitle(0);
        p.popdown();
    }));
    for t in tracks {
        let pop = pop.clone();
        let id = t.id;
        content.append(&choice(&t.label(), &t.detail(), t.selected, move || {
            player::set_subtitle(id);
            pop.popdown();
        }));
    }
    let load = gtk::Button::with_label("Load a subtitle file…");
    load.add_css_class("flat");
    load.add_css_class("menu-item");
    if let Some(l) = load.child().and_downcast::<gtk::Label>() {
        l.set_xalign(0.0);
    }
    let p = pop.clone();
    load.connect_clicked(move |_| {
        p.popdown();
        choose_subtitle_file();
    });
    content.append(&load);
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let boxed = prefs::get().sub_background;
    let p = pop.clone();
    content.append(&choice("Dark background", "", boxed, move || {
        player::set_sub_background(!boxed);
        p.popdown();
    }));

    content.append(&delay_row(("Earlier (Z)", "Later (X)"), 0.1, player::sub_delay, player::set_sub_delay, fmt::delay));

    // Size
    let size = widgets::hbox(6);
    size.add_css_class("menu-row");
    size.append(&widgets::label("Size", ""));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.5, 2.5, 0.05);
    scale.set_draw_value(false);
    scale.set_hexpand(true);
    scale.set_size_request(120, -1);
    scale.set_value(prefs::get().sub_scale);
    let readout = widgets::label(&format!("{:.0}%", prefs::get().sub_scale * 100.0), "value-readout");
    readout.set_width_chars(5);
    readout.set_xalign(1.0);
    let r = readout.clone();
    scale.connect_value_changed(move |s| {
        player::set_sub_scale(s.value());
        r.set_text(&format!("{:.0}%", s.value() * 100.0));
    });
    size.append(&scale);
    size.append(&readout);
    content.append(&size);
}

/// Pick a subtitle file for the playing video.
pub fn choose_subtitle_file() {
    let Some(v) = player::current() else { return };
    let dialog = gtk::FileDialog::builder().title("Load subtitles").modal(true).build();
    if let Some(dir) = v.path.parent() {
        dialog.set_initial_folder(Some(&gio::File::for_path(dir)));
    }
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Subtitles"));
    for ext in ["srt", "ass", "ssa", "vtt", "sub", "idx", "sup"] {
        filter.add_suffix(ext);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    dialog.open(window::window().as_ref(), gio::Cancellable::NONE, |res| {
        if let Ok(file) = res
            && let Some(path) = file.path()
        {
            player::add_subtitle(&path);
        }
    });
}

/// Seconds before the end when Up next appears (if there are no credits).
const UP_NEXT_LEAD: f64 = 20.0;

/// The card near the end of a video: what's next, a countdown, Play now
/// and Cancel.
#[derive(Clone)]
struct UpNext {
    root: gtk::Box,
    art: Art,
    title: gtk::Label,
    detail: gtk::Label,
    when: gtk::Label,
    showing: Rc<RefCell<Option<std::path::PathBuf>>>,
}

impl UpNext {
    fn new() -> UpNext {
        let root = widgets::hbox(14);
        root.add_css_class("up-next");
        root.set_halign(gtk::Align::End);
        root.set_valign(gtk::Align::End);
        root.set_visible(false);
        let art = Art::wide(160);
        root.append(&art.root);
        let text = widgets::vbox(3);
        text.set_valign(gtk::Align::Center);
        let when = widgets::label("", "up-next-when");
        when.add_css_class("mono");
        let title = widgets::label("", "up-next-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        title.set_max_width_chars(28);
        let detail = widgets::label("", "dim");
        detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
        detail.set_max_width_chars(28);
        let buttons = widgets::hbox(8);
        buttons.set_margin_top(6);
        let play = widgets::labeled_button("media-playback-start-symbolic", "Play now");
        play.add_css_class("suggested-action");
        play.connect_clicked(|_| player::play_up_next());
        let cancel = gtk::Button::with_label("Cancel");
        cancel.connect_clicked(|_| player::cancel_up_next());
        buttons.append(&play);
        buttons.append(&cancel);
        for w in [when.upcast_ref::<gtk::Widget>(), title.upcast_ref(), detail.upcast_ref(), buttons.upcast_ref()] {
            text.append(w);
        }
        root.append(&text);
        UpNext { root, art, title, detail, when, showing: Rc::default() }
    }

    /// When the credits start: a chapter named like "Credits" near the end.
    fn credits_at() -> Option<f64> {
        let d = player::duration();
        player::chapters().iter().rev().find(|(t, name)| *t > d * 0.8 && name.to_lowercase().contains("credit")).map(|(t, _)| *t)
    }

    fn update(&self) {
        let (d, pos) = (player::duration(), player::position());
        let left = (d - pos).max(0.0);
        let due = d > 60.0
            && player::state() == State::Playing
            && (left <= UP_NEXT_LEAD || Self::credits_at().is_some_and(|c| pos >= c));
        let next = if due { player::up_next() } else { None };
        let Some(next) = next else {
            self.root.set_visible(false);
            *self.showing.borrow_mut() = None;
            return;
        };
        if self.showing.borrow().as_ref() != Some(&next.path) {
            let (t, s) = match next.kind {
                Kind::Episode => (next.show.clone(), next.episode_line()),
                _ => (next.title.clone(), next.year.map(|y| y.to_string()).unwrap_or_default()),
            };
            self.title.set_text(&t);
            self.detail.set_text(&s);
            self.detail.set_visible(!s.is_empty());
            self.art.set_key(next.wide_art());
            *self.showing.borrow_mut() = Some(next.path.clone());
        }
        let secs = (left / player::speed().max(0.01)).ceil() as u64;
        self.when.set_text(&format!("UP NEXT · IN {secs} S"));
        self.root.set_visible(true);
    }
}

#[derive(Default)]
struct Swipe {
    active: bool,
    /// Some(true) when seeking sideways, Some(false) for volume.
    sideways: Option<bool>,
    dx: f64,
    dy: f64,
    start_pos: f64,
    start_vol: f64,
    target: f64,
    last_seek: Option<Instant>,
    generation: u32,
}

fn finish_swipe(swipe: &RefCell<Swipe>) {
    let s = std::mem::take(&mut *swipe.borrow_mut());
    if s.active && s.sideways == Some(true) {
        player::seek(s.target);
    }
    swipe.borrow_mut().generation = s.generation;
}

fn on_swipe(swipe: &Rc<RefCell<Swipe>>, dx: f64, dy: f64) {
    {
        let mut s = swipe.borrow_mut();
        if !s.active {
            *s = Swipe {
                active: true,
                start_pos: player::position(),
                start_vol: prefs::get().volume,
                generation: s.generation,
                ..Default::default()
            };
        }
        s.dx += dx;
        s.dy += dy;
        if s.sideways.is_none() && (s.dx.abs() > 6.0 || s.dy.abs() > 6.0) {
            s.sideways = Some(s.dx.abs() >= s.dy.abs());
        }
    }
    let (sideways, sdx, sdy, start_pos, start_vol, last) = {
        let s = swipe.borrow();
        (s.sideways, s.dx, s.dy, s.start_pos, s.start_vol, s.last_seek)
    };
    match sideways {
        Some(true) if player::current().is_some() => {
            let dir = if prefs::get().swipe_reverse { -1.0 } else { 1.0 };
            let d = player::duration();
            let target = (start_pos + dir * sdx * SECS_PER_UNIT).clamp(0.0, if d > 0.0 { d - 0.5 } else { f64::MAX });
            swipe.borrow_mut().target = target;
            let delta = target - start_pos;
            let sign = if delta >= 0.0 { "+" } else { "−" };
            flash(&format!("{sign}{} · {}", fmt::time(delta.abs()), fmt::time(target)));
            if last.is_none_or(|t| t.elapsed() > Duration::from_millis(150)) {
                swipe.borrow_mut().last_seek = Some(Instant::now());
                player::seek_fast(target);
            }
        }
        Some(false) => {
            let v = (start_vol - sdy * 0.004).clamp(0.0, player::max_volume());
            player::set_volume(v);
            if prefs::get().muted && v > 0.0 {
                player::set_muted(false);
            }
            flash(&format!("Volume {:.0}%", v * 100.0));
        }
        _ => {}
    }
    // Finish shortly after the fingers stop, in case no scroll-end comes.
    let g = {
        let mut s = swipe.borrow_mut();
        s.generation = s.generation.wrapping_add(1);
        s.generation
    };
    let sw = swipe.clone();
    glib::timeout_add_local_once(Duration::from_millis(300), move || {
        if sw.borrow().generation == g {
            finish_swipe(&sw);
        }
    });
}

pub fn build(page: &Page) {
    page.body.add_css_class("player-page");
    if let Some(e) = player::engine_error() {
        let b = widgets::banner(
            &format!(
                "Videos can't play: mpv didn't start (<tt>{}</tt>). Install <b>mpv</b> and open Nexus Media Player again.",
                gtk::glib::markup_escape_text(&e)
            ),
            true,
        );
        b.set_margin_top(16);
        b.set_margin_start(16);
        b.set_margin_end(16);
        page.body.append(&b);
    }
    let overlay = gtk::Overlay::new();
    overlay.add_css_class("player-stage");
    overlay.set_vexpand(true);
    overlay.set_hexpand(true);
    overlay.set_overflow(gtk::Overflow::Hidden);

    // ----- The picture -----
    let stage = gtk::Stack::new();
    stage.set_transition_type(gtk::StackTransitionType::Crossfade);
    stage.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });
    let picture = gtk::Picture::for_paintable(&video::paintable());
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.add_css_class("video-surface");
    stage.add_named(&picture, Some("video"));
    // Audio-only files: the poster.
    let still = widgets::vbox(14);
    still.set_valign(gtk::Align::Center);
    still.set_halign(gtk::Align::Center);
    let art = Art::poster(220);
    art.root.set_halign(gtk::Align::Center);
    still.append(&art.root);
    stage.add_named(&still, Some("still"));
    let empty = widgets::empty_state(
        "nmp-video-symbolic",
        "Nothing playing",
        "Pick a movie or an episode, or open a file with <b>Ctrl+O</b>.",
        Some(("Browse movies", Box::new(|| window::navigate("movies")))),
    );
    stage.add_named(&empty, Some("empty"));
    overlay.set_child(Some(&stage));

    // ----- Top: what's playing -----
    let top = widgets::vbox(2);
    top.add_css_class("osd-top");
    top.set_valign(gtk::Align::Start);
    let title = widgets::label("", "osd-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let subtitle = widgets::label("", "osd-subtitle");
    subtitle.set_ellipsize(gtk::pango::EllipsizeMode::End);
    top.append(&title);
    top.append(&subtitle);
    overlay.add_overlay(&top);

    // ----- Middle: buffering and messages -----
    let spinner = gtk::Spinner::new();
    spinner.set_size_request(40, 40);
    spinner.set_halign(gtk::Align::Center);
    spinner.set_valign(gtk::Align::Center);
    spinner.set_can_target(false);
    spinner.set_visible(false);
    overlay.add_overlay(&spinner);
    let flash_label = widgets::label("", "osd-flash");
    flash_label.add_css_class("mono");
    flash_label.set_halign(gtk::Align::Center);
    flash_label.set_valign(gtk::Align::Start);
    flash_label.set_can_target(false);
    flash_label.set_visible(false);
    overlay.add_overlay(&flash_label);
    FLASH.with(|f| *f.borrow_mut() = Some((flash_label, Rc::new(Cell::new(0)))));

    // ----- Bottom: controls -----
    let controls = widgets::vbox(6);
    controls.add_css_class("osd-controls");
    controls.set_valign(gtk::Align::End);
    let seek = SeekBar::new();
    controls.append(&seek.root);
    let row = widgets::hbox(4);
    let prev = widgets::icon_button("media-skip-backward-symbolic", "Previous (Ctrl+←)");
    let back = widgets::icon_button("media-seek-backward-symbolic", "Back (←)");
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.add_css_class("play-button");
    let fwd = widgets::icon_button("media-seek-forward-symbolic", "Forward (→)");
    let next = widgets::icon_button("media-skip-forward-symbolic", "Next (Ctrl+→)");
    for b in [&prev, &back, &play, &fwd, &next] {
        b.set_focus_on_click(false);
        b.add_css_class("osd-button");
        row.append(b);
    }
    let p = prefs::get();
    let mute = widgets::icon_button(volume_icon(p.volume, p.muted), "Mute (M)");
    mute.add_css_class("osd-button");
    mute.set_focus_on_click(false);
    mute.set_margin_start(10);
    row.append(&mute);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, player::max_volume(), 0.01);
    volume.set_draw_value(false);
    volume.set_value(p.volume);
    volume.set_size_request(96, -1);
    volume.add_css_class("volume");
    volume.set_focus_on_click(false);
    row.append(&volume);
    let spacer = widgets::hbox(0);
    spacer.set_hexpand(true);
    row.append(&spacer);
    let speed_label = widgets::label("", "osd-speed");
    speed_label.add_css_class("mono");
    row.append(&speed_label);
    let chapters = menu_button("view-list-ordered-symbolic", "Chapters (Page Up and Page Down)", chapter_menu);
    chapters.set_visible(false);
    let pic = picture.clone();
    let shape = menu_button("video-display-symbolic", "Picture: shape, fill, turn", move |c, p| picture_menu(c, p, &pic));
    let speed = menu_button("nmp-speed-symbolic", "Speed ([ and ])", speed_menu);
    let audio = menu_button("nmp-audio-track-symbolic", "Sound: track, sync and night mode (A)", audio_menu);
    let subs = menu_button("nmp-subtitles-symbolic", "Subtitles (S)", subtitle_menu);
    let full = widgets::icon_button("view-fullscreen-symbolic", "Fullscreen (F)");
    full.add_css_class("osd-button");
    full.set_focus_on_click(false);
    let close = widgets::icon_button("window-close-symbolic", "Close video");
    close.add_css_class("osd-button");
    close.set_focus_on_click(false);
    row.append(&chapters);
    row.append(&shape);
    row.append(&speed);
    row.append(&audio);
    row.append(&subs);
    row.append(&full);
    row.append(&close);
    controls.append(&row);
    overlay.add_overlay(&controls);
    let up_next = UpNext::new();
    overlay.add_overlay(&up_next.root);
    page.body.append(&overlay);

    // ----- Wiring -----
    play.connect_clicked(|_| player::toggle());
    prev.connect_clicked(|_| player::previous());
    next.connect_clicked(|_| player::next());
    back.connect_clicked(|_| player::seek_by(-prefs::get().skip_short));
    fwd.connect_clicked(|_| player::seek_by(prefs::get().skip_short));
    full.connect_clicked(|_| window::toggle_fullscreen());
    close.connect_clicked(|_| {
        player::close();
        window::navigate("home");
    });
    if let Some(w) = window::window() {
        let weak = full.downgrade();
        w.connect_fullscreened_notify(move |w| {
            if let Some(b) = weak.upgrade() {
                let on = w.is_fullscreen();
                b.set_icon_name(if on { "view-restore-symbolic" } else { "view-fullscreen-symbolic" });
                b.set_tooltip_text(Some(if on { "Leave fullscreen (F or Esc)" } else { "Fullscreen (F)" }));
            }
        });
    }
    mute.connect_clicked(|_| player::set_muted(!prefs::get().muted));
    volume.connect_change_value(|_, _, v| {
        let v = v.clamp(0.0, player::max_volume());
        player::set_volume(v);
        if prefs::get().muted && v > 0.0 {
            player::set_muted(false);
        }
        glib::Propagation::Proceed
    });

    // Subtitles move up out of the way while the controls show.
    let place_subs: Rc<dyn Fn(bool)> = {
        let (overlay, controls) = (overlay.clone(), controls.clone());
        Rc::new(move |shown: bool| {
            let (vw, vh) = player::video_size();
            let (w, h) = (overlay.width() as f64, overlay.height() as f64);
            let top = controls.compute_bounds(&overlay).map(|b| b.y() as f64);
            let (Some(top), true) = (top, shown && vw > 0 && vh > 0 && w > 0.0 && h > 0.0) else {
                player::set_sub_pos(100.0);
                return;
            };
            let (sx, sy) = (w / vw as f64, h / vh as f64);
            let shown_h = vh as f64 * if FILL.with(|f| f.get()) { sx.max(sy) } else { sx.min(sy) };
            // Below 100, mpv drops its bottom margin; keep a little room.
            let overlap = (h + shown_h) / 2.0 - top;
            let lift = if overlap > 0.0 { overlap + 16.0 } else { 0.0 };
            player::set_sub_pos(100.0 - lift / shown_h * 100.0);
        })
    };

    overlay.connect_unmap(|_| player::set_sub_pos(100.0));

    // Controls fade out while playing and the pointer rests.
    let idle_gen: Rc<Cell<u32>> = Rc::default();
    let over_controls = Rc::new(Cell::new(false));
    let menus = [chapters.clone(), shape.clone(), speed.clone(), audio.clone(), subs.clone()];
    let wake: Rc<dyn Fn()> = {
        let (overlay, top, controls, idle_gen, over_controls, place_subs) =
            (overlay.clone(), top.clone(), controls.clone(), idle_gen.clone(), over_controls.clone(), place_subs.clone());
        Rc::new(move || {
            for w in [top.upcast_ref::<gtk::Widget>(), controls.upcast_ref()] {
                w.remove_css_class("idle");
                w.set_can_target(true);
            }
            place_subs(true);
            overlay.set_cursor(None);
            let g = idle_gen.get().wrapping_add(1);
            idle_gen.set(g);
            let (overlay, top, controls, idle_gen, over, menus, place_subs) = (
                overlay.clone(),
                top.clone(),
                controls.clone(),
                idle_gen.clone(),
                over_controls.clone(),
                menus.clone(),
                place_subs.clone(),
            );
            glib::timeout_add_local_once(HIDE_AFTER, move || {
                // Stay while a menu is open.
                let busy_menu = menus.iter().any(|m| m.is_active());
                if idle_gen.get() != g || over.get() || busy_menu || player::state() != State::Playing {
                    return;
                }
                for w in [top.upcast_ref::<gtk::Widget>(), controls.upcast_ref()] {
                    w.add_css_class("idle");
                    w.set_can_target(false);
                }
                place_subs(false);
                overlay.set_cursor_from_name(Some("none"));
            });
        })
    };
    let motion = gtk::EventControllerMotion::new();
    let last_xy = Rc::new(Cell::new((0.0, 0.0)));
    let w = wake.clone();
    motion.connect_motion(move |_, x, y| {
        // Some compositors send motion with no movement; ignore those.
        let (lx, ly) = last_xy.get();
        if (x - lx).abs() + (y - ly).abs() > 1.0 {
            last_xy.set((x, y));
            w();
        }
    });
    overlay.add_controller(motion);
    let over = gtk::EventControllerMotion::new();
    let o = over_controls.clone();
    over.connect_enter(move |_, _, _| o.set(true));
    let o = over_controls.clone();
    over.connect_leave(move |_| o.set(false));
    controls.add_controller(over);

    // Click to pause, at once; a double-click undoes that and goes fullscreen.
    let click = gtk::GestureClick::new();
    let toggled = Rc::new(Cell::new(false));
    let t = toggled.clone();
    click.connect_pressed(move |g, n, _, _| {
        if n == 2 {
            if t.replace(false) {
                player::toggle();
            }
            window::toggle_fullscreen();
            g.set_state(gtk::EventSequenceState::Claimed);
        }
    });
    click.connect_released(move |_, n, _, _| {
        let first = n == 1 && player::current().is_some();
        toggled.set(first);
        if first {
            player::toggle();
        }
    });
    stage.add_controller(click);

    // Touchpad: sideways seeks, up/down sets the volume. A mouse wheel
    // changes the volume, or skips when tilted.
    let swipe: Rc<RefCell<Swipe>> = Rc::default();
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::BOTH_AXES);
    let sw = swipe.clone();
    let w = wake.clone();
    scroll.connect_scroll(move |c, dx, dy| {
        w();
        if c.unit() == gdk::ScrollUnit::Wheel {
            if dy != 0.0 {
                let v = (prefs::get().volume - dy * 0.05).clamp(0.0, player::max_volume());
                player::set_volume(v);
                flash(&format!("Volume {:.0}%", v * 100.0));
            } else if dx != 0.0 {
                player::seek_by(prefs::get().skip_short * dx.signum());
            }
            return glib::Propagation::Stop;
        }
        on_swipe(&sw, dx, dy);
        glib::Propagation::Stop
    });
    let sw = swipe.clone();
    scroll.connect_scroll_end(move |_| finish_swipe(&sw));
    overlay.add_controller(scroll);

    // ----- Following the player -----
    // The second line of the top: what's playing, then when it ends.
    let sub_base: Rc<RefCell<String>> = Rc::default();
    let show_sub: Rc<dyn Fn()> = {
        let (subtitle, sub_base) = (subtitle.clone(), sub_base.clone());
        Rc::new(move || {
            let base = sub_base.borrow();
            let d = player::duration();
            let ends = if d > 0.0 && player::current().is_some() {
                fmt::clock_in((d - player::position()) / player::speed().max(0.01))
            } else {
                String::new()
            };
            let text = match (base.is_empty(), ends.is_empty()) {
                (_, true) => base.clone(),
                (true, false) => format!("Ends at {ends}"),
                (false, false) => format!("{base} · Ends at {ends}"),
            };
            if subtitle.text() != text {
                subtitle.set_text(&text);
            }
        })
    };
    let refresh = {
        let top = top.clone();
        let (stage, art, title, play, spinner, mute, volume, speed_label, prev, next, wake) = (
            stage.clone(),
            art.clone(),
            title.clone(),
            play.clone(),
            spinner.clone(),
            mute.clone(),
            volume.clone(),
            speed_label.clone(),
            prev.clone(),
            next.clone(),
            wake.clone(),
        );
        let (subs, audio, close, chapters) = (subs.clone(), audio.clone(), close.clone(), chapters.clone());
        let (sub_base, show_sub) = (sub_base.clone(), show_sub.clone());
        let (place_subs, controls) = (place_subs.clone(), controls.clone());
        move |e: Event| match e {
            Event::Track | Event::Video => {
                let cur = player::current();
                let name = if player::has_video() || video::paintable().has_frame() {
                    "video"
                } else if cur.is_some() {
                    "still"
                } else {
                    "empty"
                };
                stage.set_visible_child_name(name);
                match &cur {
                    Some(v) => {
                        art.set_key(v.card_art());
                        match v.kind {
                            Kind::Episode => {
                                title.set_text(&v.show);
                                *sub_base.borrow_mut() = v.episode_line();
                            }
                            _ => {
                                title.set_text(&v.title);
                                *sub_base.borrow_mut() = v.year.map(|y| y.to_string()).unwrap_or_default();
                            }
                        }
                    }
                    None => {
                        title.set_text("");
                        sub_base.borrow_mut().clear();
                    }
                }
                show_sub();
                top.set_visible(cur.is_some());
                close.set_sensitive(cur.is_some());
                prev.set_sensitive(player::can_previous());
                next.set_sensitive(player::can_next());
            }
            Event::State => {
                let playing = player::state() == State::Playing;
                play.set_icon_name(if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
                play.set_tooltip_text(Some(if playing { "Pause (Space)" } else { "Play (Space)" }));
                wake();
            }
            Event::Queue => {
                prev.set_sensitive(player::can_previous());
                next.set_sensitive(player::can_next());
                up_next.update();
            }
            Event::Buffering => spinner.set_visible(player::buffering()),
            Event::Options => {
                let p = prefs::get();
                mute.set_icon_name(volume_icon(p.volume, p.muted));
                mute.set_tooltip_text(Some(if p.muted { "Unmute (M)" } else { "Mute (M)" }));
                if (volume.adjustment().upper() - player::max_volume()).abs() > 0.001 {
                    volume.set_range(0.0, player::max_volume());
                }
                if (volume.value() - p.volume).abs() > 0.005 {
                    volume.set_value(p.volume);
                }
                show_sub();
                let s = player::speed();
                speed_label.set_text(&if (s - 1.0).abs() > 0.001 { format!("{}×", fmt::speed(s)) } else { String::new() });
            }
            Event::Tracks => {
                let has_subs = !player::tracks(TrackKind::Sub).is_empty();
                subs.remove_css_class("dim");
                if !has_subs {
                    subs.add_css_class("dim");
                }
                audio.set_sensitive(player::current().is_some());
                chapters.set_visible(!player::chapters().is_empty());
            }
            Event::Position | Event::Seeked => {
                up_next.update();
                show_sub();
                // The layout may have changed (fullscreen, a resize).
                place_subs(!controls.has_css_class("idle"));
            }
        }
    };
    for e in [Event::Track, Event::State, Event::Options, Event::Tracks, Event::Buffering] {
        refresh(e);
    }
    spinner.connect_visible_notify(|s| s.set_spinning(s.is_visible()));
    player::subscribe(&overlay, refresh);
    let stage2 = stage.clone();
    video::paintable().connect_invalidate_contents(move |p| {
        if p.has_frame() && stage2.visible_child_name().as_deref() != Some("video") {
            stage2.set_visible_child_name("video");
        }
    });
}
