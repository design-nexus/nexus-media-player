//! The main window: navigation sidebar with library search, a stack of pages
//! built the first time they're shown, and the player bar underneath. In
//! fullscreen only the player page shows.

use crate::library::store;
use crate::sections::{self, Section};
use crate::widgets;
use crate::{player, playerbar, prefs, theme};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

struct Ui {
    window: gtk::ApplicationWindow,
    stack: gtk::Stack,
    nav: gtk::Box,
    nav_items: HashMap<String, gtk::Button>,
    playlist_box: gtk::Box,
    search: gtk::SearchEntry,
    bar: gtk::Box,
    pages: HashMap<String, gtk::Widget>,
    sections: Vec<Section>,
    current: String,
    /// Where Esc in the search goes back to.
    before_search: String,
    overlay: gtk::Overlay,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
    static NARROW: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// A playlist asked for before the playlists finished loading.
    static PENDING: RefCell<Option<String>> = const { RefCell::new(None) };
}

fn ui() -> Option<Rc<RefCell<Ui>>> {
    UI.with(|u| u.borrow().clone())
}

pub fn present(app: &gtk::Application, section: Option<&str>) {
    if let Some(ui) = ui() {
        let window = ui.borrow().window.clone();
        if let Some(s) = section {
            navigate(s);
        }
        window.present();
        return;
    }
    theme::install();
    install_icons();
    build(app);
    let start = section.map(String::from).unwrap_or_else(|| prefs::get().last_section);
    navigate(&start);
    // Developer aid: NMP_SNAPSHOT=/path.png renders the window to a PNG
    // (invisibly) and quits, so layouts can be checked without a visible window.
    if let Some(out) = std::env::var_os("NMP_SNAPSHOT") {
        snapshot_and_quit(app, std::path::PathBuf::from(out));
        return;
    }
    if let Some(ui) = ui() {
        ui.borrow().window.present();
    }
    if prefs::take_broken() {
        toast("Your settings file couldn't be read, so defaults are in use. The old file is kept as settings.toml.bak.");
    }
}

/// Our own symbolic icons, for things the icon theme has no glyph for.
/// They're written to the cache once and added to the icon search path.
fn install_icons() {
    const ICONS: &[(&str, &str)] = &[
        ("nmp-home-symbolic.svg", include_str!("../data/icons/nmp-home-symbolic.svg")),
        ("nmp-movie-symbolic.svg", include_str!("../data/icons/nmp-movie-symbolic.svg")),
        ("nmp-tv-symbolic.svg", include_str!("../data/icons/nmp-tv-symbolic.svg")),
        ("nmp-video-symbolic.svg", include_str!("../data/icons/nmp-video-symbolic.svg")),
        ("nmp-playlist-symbolic.svg", include_str!("../data/icons/nmp-playlist-symbolic.svg")),
        ("nmp-queue-symbolic.svg", include_str!("../data/icons/nmp-queue-symbolic.svg")),
        ("nmp-equalizer-symbolic.svg", include_str!("../data/icons/nmp-equalizer-symbolic.svg")),
        ("nmp-subtitles-symbolic.svg", include_str!("../data/icons/nmp-subtitles-symbolic.svg")),
        ("nmp-audio-track-symbolic.svg", include_str!("../data/icons/nmp-audio-track-symbolic.svg")),
        ("nmp-speed-symbolic.svg", include_str!("../data/icons/nmp-speed-symbolic.svg")),
    ];
    let dir = crate::paths::cache_dir().join("icons");
    for (name, svg) in ICONS {
        let path = dir.join(name);
        if std::fs::read_to_string(&path).ok().as_deref() != Some(*svg) {
            let _ = crate::cmd::atomic_write(&path, svg);
        }
    }
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_search_path(&dir);
    }
}

fn nav_button(icon: &str, title: &str, tooltip: &str) -> (gtk::Button, gtk::Label) {
    let button = gtk::Button::new();
    button.add_css_class("nav-item");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&gtk::Image::from_icon_name(icon));
    let l = widgets::label(title, "nav-label");
    l.set_hexpand(true);
    l.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&l);
    button.set_child(Some(&content));
    button.set_tooltip_text(Some(tooltip));
    (button, l)
}

fn build(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Nexus Media Player")
        .default_width(1180)
        .default_height(820)
        .build();
    window.add_css_class("media-window");
    // No client-side titlebar: Hyprland manages the window.
    window.set_titlebar(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    window.set_icon_name(Some(crate::APP_ID));

    let sections = sections::all();

    // ----- Sidebar -----
    let nav = gtk::Box::new(gtk::Orientation::Vertical, 0);
    nav.add_css_class("settings-navigation");
    nav.set_hexpand(false);
    let heading = widgets::label("MEDIA PLAYER", "menu-heading");
    heading.add_css_class("compact-hide");
    heading.set_hexpand(true);
    nav.append(&nav_head(&heading));

    let search = gtk::SearchEntry::new();
    search.set_placeholder_text(Some("Search library"));
    search.add_css_class("settings-search");
    search.add_css_class("compact-hide");
    nav.append(&search);

    let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut nav_items = HashMap::new();
    let playlist_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let mut last_group = "";
    for s in sections.iter().filter(|s| s.nav) {
        if s.group != last_group {
            let g = widgets::label(&s.group.to_uppercase(), "nav-group");
            g.add_css_class("compact-hide");
            list.append(&g);
            last_group = s.group;
        }
        let (button, label) = nav_button(s.icon, s.title, s.description);
        label.add_css_class("compact-hide");
        let id = s.id;
        button.connect_clicked(move |_| navigate(id));
        list.append(&button);
        nav_items.insert(s.id.to_string(), button);
        if s.id == "queue" {
            list.append(&playlist_box);
            let (new_button, label) = nav_button("list-add-symbolic", "New playlist", "Make a playlist, or import one");
            label.add_css_class("compact-hide");
            new_button.add_css_class("nav-add");
            new_button.connect_clicked(|_| sections::playlist::new_dialog(Vec::new()));
            list.append(&new_button);
        }
    }
    let nav_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        // Scrolls with wheel, trackpad and keyboard; no visible scrollbar.
        .vscrollbar_policy(gtk::PolicyType::External)
        .vexpand(true)
        .child(&list)
        .build();
    nav.append(&nav_scroll);

    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    footer.add_css_class("nav-footer");
    footer.add_css_class("compact-hide");
    let version = widgets::label(concat!("Nexus Media Player ", env!("CARGO_PKG_VERSION")), "dim");
    version.set_hexpand(true);
    version.set_ellipsize(gtk::pango::EllipsizeMode::End);
    footer.append(&version);
    let scan_label = widgets::label("", "nav-readout");
    scan_label.add_css_class("mono");
    footer.append(&scan_label);
    nav.append(&footer);
    {
        let l = scan_label.clone();
        store::subscribe(&scan_label, move |c| {
            if c == store::Change::Scan {
                l.set_text(&match (store::scanning(), store::busy()) {
                    (Some((done, total)), _) if total > 0 => format!("{}%", done * 100 / total),
                    (Some(_), _) => "Scanning".into(),
                    (None, Some(b)) => b,
                    (None, None) => String::new(),
                });
            }
        });
    }

    // ----- Content -----
    let stack = gtk::Stack::new();
    stack.add_css_class("settings-content");
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::Crossfade);
    stack.set_transition_duration(if prefs::get().reduce_motion { 0 } else { 160 });

    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_hexpand(true);
    content.append(&stack);
    let bar = playerbar::build();
    content.append(&bar);

    let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    body.append(&nav);
    body.append(&content);

    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&body));
    window.set_child(Some(&overlay));

    install_keys(&window, &search);
    accept_file_drops(&overlay);
    // Video drawing needs the window's surface; set it up as soon as there is one.
    window.connect_realize(player::attach);
    window.connect_fullscreened_notify(|w| apply_fullscreen(w.is_fullscreen()));
    search.connect_search_changed(|e| on_search(&e.text()));
    search.connect_activate(|_| sections::search::focus_results());
    search.connect_stop_search(|e| e.set_text(""));

    // Narrow windows (a tiled half-screen) get an icon-only sidebar.
    let apply_width = {
        let nav = nav.clone();
        move |w: &gtk::ApplicationWindow| {
            let width = if w.width() > 0 { w.width() } else { w.default_width() };
            let narrow = width > 0 && width < 980;
            if narrow == NARROW.with(|n| n.get()) && nav.has_css_class("sized") {
                return;
            }
            nav.add_css_class("sized");
            set_narrow(narrow);
            apply_compact(&nav, narrow || prefs::get().sidebar_collapsed);
        }
    };
    let aw = apply_width.clone();
    window.connect_default_width_notify(move |w| aw(w));
    let aw = apply_width.clone();
    window.connect_realize(move |w| aw(w));
    // Tiled windows are resized by the compositor; watch the real size too.
    let w2 = window.clone();
    glib::timeout_add_local(std::time::Duration::from_millis(400), move || {
        apply_width(&w2);
        glib::ControlFlow::Continue
    });

    let ui = Ui {
        window,
        stack,
        nav,
        nav_items,
        playlist_box: playlist_box.clone(),
        search,
        bar,
        pages: HashMap::new(),
        sections,
        current: String::new(),
        before_search: String::new(),
        overlay,
    };
    UI.with(|u| *u.borrow_mut() = Some(Rc::new(RefCell::new(ui))));

    store::subscribe(&playlist_box, |c| {
        if c == store::Change::Playlists {
            refresh_playlists();
        }
    });
    refresh_playlists();
}

/// Hide everything marked `compact-hide` in the sidebar (labels, headings,
/// search, footer) when it's icon-only.
/// The button that collapses the sidebar to icons, beside the app heading.
fn nav_head(heading: &gtk::Label) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    row.add_css_class("nav-head");
    row.append(heading);
    let button = gtk::Button::from_icon_name("sidebar-show-symbolic");
    button.add_css_class("nav-collapse");
    button.set_tooltip_text(Some("Collapse or expand the sidebar (Ctrl+B)"));
    button.set_valign(gtk::Align::Center);
    button.connect_clicked(|_| toggle_sidebar());
    row.append(&button);
    row
}

/// The sidebar shows only icons: hide the labels, centre the icons and the toggle.
fn apply_compact(nav: &gtk::Box, compact: bool) {
    if compact {
        nav.add_css_class("compact");
    } else {
        nav.remove_css_class("compact");
    }
    set_compact_hidden(nav, compact);
}

pub fn toggle_sidebar() {
    prefs::update(|p| p.sidebar_collapsed = !p.sidebar_collapsed);
    let Some(ui) = ui() else { return };
    let nav = ui.borrow().nav.clone();
    apply_compact(&nav, NARROW.with(|n| n.get()) || prefs::get().sidebar_collapsed);
}

fn set_compact_hidden(root: &gtk::Box, compact: bool) {
    fn walk(w: &gtk::Widget, compact: bool) {
        if w.has_css_class("compact-hide") {
            w.set_visible(!compact);
        }
        // Icon-only: centre the icon in its pill, and the toggle in the column.
        if w.has_css_class("nav-item")
            && let Some(content) = w.downcast_ref::<gtk::Button>().and_then(|b| b.child())
        {
            content.set_halign(if compact { gtk::Align::Center } else { gtk::Align::Fill });
        }
        if w.has_css_class("nav-collapse") {
            w.set_halign(if compact { gtk::Align::Center } else { gtk::Align::End });
            w.set_hexpand(compact);
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            walk(&c, compact);
            child = c.next_sibling();
        }
    }
    walk(root.upcast_ref(), compact);
}

fn install_keys(window: &gtk::ApplicationWindow, search: &gtk::SearchEntry) {
    // Capture phase: these work wherever focus is, except while typing.
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let s2 = search.clone();
    let w2 = window.clone();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let ctrl = mods.contains(gdk::ModifierType::CONTROL_MASK);
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        let alt = mods.contains(gdk::ModifierType::ALT_MASK);
        let typing = gtk::prelude::GtkWindowExt::focus(&w2).is_some_and(|f| {
            f.is::<gtk::Text>() || f.ancestor(gtk::Entry::static_type()).is_some() || f.is::<gtk::SearchEntry>()
        });
        let on_player = current() == "now-playing";
        let p = prefs::get();
        let stop = glib::Propagation::Stop;
        if ctrl {
            return match key {
                gdk::Key::f => {
                    if w2.is_fullscreen() {
                        set_fullscreen(false);
                    }
                    s2.grab_focus();
                    stop
                }
                gdk::Key::o => {
                    open_files();
                    stop
                }
                gdk::Key::l => {
                    open_url();
                    stop
                }
                gdk::Key::s if !typing => {
                    player::screenshot();
                    stop
                }
                gdk::Key::b => {
                    toggle_sidebar();
                    stop
                }
                gdk::Key::q | gdk::Key::w => {
                    w2.close();
                    stop
                }
                gdk::Key::Left if !typing => {
                    player::previous();
                    stop
                }
                gdk::Key::minus | gdk::Key::KP_Subtract if !typing => {
                    player::nudge_audio_delay(-0.05);
                    stop
                }
                gdk::Key::plus | gdk::Key::equal | gdk::Key::KP_Add if !typing => {
                    player::nudge_audio_delay(0.05);
                    stop
                }
                gdk::Key::Right if !typing => {
                    player::next();
                    stop
                }
                _ => glib::Propagation::Proceed,
            };
        }
        if typing || alt {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::space => player::toggle(),
            gdk::Key::Escape if w2.is_fullscreen() => set_fullscreen(false),
            gdk::Key::Escape if !s2.text().is_empty() => s2.set_text(""),
            gdk::Key::f | gdk::Key::F => toggle_fullscreen(),
            gdk::Key::m | gdk::Key::M => {
                player::set_muted(!p.muted);
                flash(if p.muted { "Sound on" } else { "Muted" });
            }
            gdk::Key::s | gdk::Key::S => cycle_track(player::TrackKind::Sub),
            gdk::Key::n | gdk::Key::N => {
                player::set_night_mode(!p.night_mode);
                flash(if p.night_mode { "Night mode off" } else { "Night mode on" });
            }
            gdk::Key::a | gdk::Key::A => cycle_track(player::TrackKind::Audio),
            gdk::Key::z | gdk::Key::Z => player::nudge_sub_delay(-0.1),
            gdk::Key::x | gdk::Key::X => player::nudge_sub_delay(0.1),
            gdk::Key::bracketleft => player::step_speed(false),
            gdk::Key::bracketright => player::step_speed(true),
            gdk::Key::comma | gdk::Key::less => player::frame_step(false),
            gdk::Key::period | gdk::Key::greater => player::frame_step(true),
            k if player::current().is_some() && digit(k).is_some() => {
                let d = player::duration();
                if d > 0.0 {
                    let n = digit(k).unwrap_or(0);
                    player::seek(d * n as f64 / 10.0);
                    flash(&format!("{}0% · {}", n, crate::fmt::time(d * n as f64 / 10.0)));
                }
            }
            gdk::Key::Page_Up => player::step_chapter(false),
            gdk::Key::Page_Down => player::step_chapter(true),
            gdk::Key::Home if on_player => player::seek(0.0),
            gdk::Key::End if on_player => player::seek(player::duration() - 1.0),
            // On the player page the arrows always drive playback.
            gdk::Key::Left | gdk::Key::Right | gdk::Key::Up | gdk::Key::Down if on_player => arrow(key, shift),
            _ => return glib::Propagation::Proceed,
        }
        stop
    });
    window.add_controller(keys);

    // Bubble phase: elsewhere the arrows seek and set the volume only when
    // nothing focused used them (lists and grids move their selection).
    let keys = gtk::EventControllerKey::new();
    keys.connect_key_pressed(move |_, key, _, mods| {
        let shift = mods.contains(gdk::ModifierType::SHIFT_MASK);
        if !(mods.is_empty() || mods == gdk::ModifierType::SHIFT_MASK) {
            return glib::Propagation::Proceed;
        }
        match key {
            gdk::Key::Left | gdk::Key::Right | gdk::Key::Up | gdk::Key::Down => {
                arrow(key, shift);
                glib::Propagation::Stop
            }
            _ => glib::Propagation::Proceed,
        }
    });
    window.add_controller(keys);
}

/// 0–9 on the main row or the keypad.
fn digit(key: gdk::Key) -> Option<u32> {
    key.to_unicode().and_then(|c| c.to_digit(10))
}

fn arrow(key: gdk::Key, shift: bool) {
    let p = prefs::get();
    let skip = if shift { p.skip_long } else { p.skip_short };
    match key {
        gdk::Key::Left | gdk::Key::Right => {
            if player::current().is_none() {
                return;
            }
            let delta = if key == gdk::Key::Left { -skip } else { skip };
            player::seek_by(delta);
            let sign = if delta < 0.0 { "−" } else { "+" };
            flash(&format!("{sign}{} · {}", crate::fmt::time(skip), crate::fmt::time(player::position())));
        }
        _ => {
            let step = if key == gdk::Key::Up { 0.05 } else { -0.05 };
            let v = (p.volume + step).clamp(0.0, player::max_volume());
            player::set_volume(v);
            if p.muted && step > 0.0 {
                player::set_muted(false);
            }
            flash(&format!("Volume {:.0}%", v * 100.0));
        }
    }
}

/// Move to the next audio or subtitle track (subtitles go through Off).
fn cycle_track(kind: player::TrackKind) {
    let tracks = player::tracks(kind);
    if tracks.is_empty() {
        flash(if kind == player::TrackKind::Sub { "No subtitles" } else { "No other sound tracks" });
        return;
    }
    let cur = tracks.iter().position(|t| t.selected);
    let (id, label) = match (kind, cur) {
        (player::TrackKind::Sub, Some(i)) if i + 1 >= tracks.len() => (0, "Subtitles off".to_string()),
        (_, Some(i)) => {
            let t = &tracks[(i + 1) % tracks.len()];
            (t.id, t.label())
        }
        (_, None) => (tracks[0].id, tracks[0].label()),
    };
    match kind {
        player::TrackKind::Sub => player::set_subtitle(id),
        player::TrackKind::Audio => player::set_audio(id),
    }
    flash(&label);
}

/// Videos, folders and M3U files dropped on the window play; with Shift they
/// join the queue.
fn accept_file_drops(overlay: &gtk::Overlay) {
    let hint = widgets::label("Drop to play · hold Shift to add to the queue", "drop-hint");
    hint.set_halign(gtk::Align::Center);
    hint.set_valign(gtk::Align::Center);
    hint.set_can_target(false);
    hint.set_visible(false);
    overlay.add_overlay(&hint);
    let target = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let h = hint.clone();
    target.connect_enter(move |_, _, _| {
        h.set_visible(true);
        gdk::DragAction::COPY
    });
    let h = hint.clone();
    target.connect_leave(move |_| h.set_visible(false));
    target.connect_drop(move |t, value, _, _| {
        hint.set_visible(false);
        let Ok(list) = value.get::<gdk::FileList>() else { return false };
        let dropped: Vec<std::path::PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
        let paths = crate::expand(&dropped);
        if paths.is_empty() {
            toast("Nothing there plays as a video.");
            return false;
        }
        if t.current_event_state().contains(gdk::ModifierType::SHIFT_MASK) {
            let n = paths.len();
            player::enqueue(paths);
            toast(&crate::menu::added_text(n, "to the queue"));
        } else {
            player::play_paths(paths, 0);
            navigate("now-playing");
        }
        true
    });
    overlay.add_controller(target);
}

/// Open video files from a file dialog and play them.
pub fn open_files() {
    let dialog = gtk::FileDialog::builder().title("Open videos").modal(true).build();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Videos"));
    filter.add_mime_type("video/*");
    for ext in crate::library::EXTENSIONS {
        filter.add_suffix(ext);
    }
    let filters = gtk::gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    dialog.set_filters(Some(&filters));
    let start = crate::paths::videos_dir();
    if start.is_dir() {
        dialog.set_initial_folder(Some(&gtk::gio::File::for_path(start)));
    }
    dialog.open_multiple(window().as_ref(), gtk::gio::Cancellable::NONE, |res| {
        let Ok(files) = res else { return };
        let paths: Vec<std::path::PathBuf> = (0..files.n_items())
            .filter_map(|i| files.item(i).and_downcast::<gtk::gio::File>())
            .filter_map(|f| f.path())
            .collect();
        if !paths.is_empty() {
            player::play_paths(paths, 0);
            navigate("now-playing");
        }
    });
}

/// Play a web address: a stream, or a page yt-dlp understands.
pub fn open_url() {
    let desc = if crate::cmd::present("yt-dlp") {
        "A stream, or a video page that yt-dlp understands."
    } else {
        "A stream address. Install yt-dlp to play video pages too."
    };
    widgets::ask_text("Open address", desc, "", "Play", |text| {
        let url = if text.contains("://") { text } else { format!("https://{text}") };
        player::play_paths(vec![url.into()], 0);
        navigate("now-playing");
    });
}

/// Show a short message over the video.
pub fn flash(text: &str) {
    sections::now_playing::flash(text);
}

pub fn toggle_fullscreen() {
    let on = window().is_some_and(|w| w.is_fullscreen());
    set_fullscreen(!on);
}

pub fn set_fullscreen(on: bool) {
    let Some(w) = window() else { return };
    if on {
        navigate("now-playing");
        w.fullscreen();
    } else {
        w.unfullscreen();
    }
    apply_fullscreen(on);
}

/// Fullscreen shows only the player: no sidebar, no player bar.
fn apply_fullscreen(on: bool) {
    let Some(ui) = ui() else { return };
    let u = ui.borrow();
    u.nav.set_visible(!on);
    u.bar.set_visible(!on && u.current != "now-playing");
    if on {
        u.window.add_css_class("fullscreen");
    } else {
        u.window.remove_css_class("fullscreen");
    }
}

fn on_search(text: &str) {
    let Some(ui) = ui() else { return };
    let q = text.trim().to_string();
    if q.is_empty() {
        let back = std::mem::take(&mut ui.borrow_mut().before_search);
        if ui.borrow().current == "search" {
            navigate(if back.is_empty() { "home" } else { &back });
        }
        return;
    }
    if ui.borrow().current != "search" {
        let cur = ui.borrow().current.clone();
        ui.borrow_mut().before_search = cur;
    }
    navigate("search");
    sections::search::set_query(&q);
}

fn set_narrow(narrow: bool) {
    NARROW.with(|n| n.set(narrow));
    playerbar::set_narrow(narrow);
    crate::videotable::set_narrow(narrow);
    let Some(ui) = ui() else { return };
    for page in ui.borrow().pages.values() {
        mark_page(page, narrow);
    }
}

pub fn narrow() -> bool {
    NARROW.with(|n| n.get())
}

fn mark_page(page: &gtk::Widget, narrow: bool) {
    let body = page
        .downcast_ref::<gtk::ScrolledWindow>()
        .and_then(|s| s.child())
        .and_then(|v| v.first_child())
        .unwrap_or_else(|| page.clone());
    if narrow {
        body.add_css_class("narrow");
    } else {
        body.remove_css_class("narrow");
    }
}

fn ensure_built(id: &str) -> bool {
    let Some(ui) = ui() else { return false };
    if ui.borrow().pages.contains_key(id) {
        return true;
    }
    let page: gtk::Widget = if let Some(pid) = id.strip_prefix("playlist:").and_then(|p| p.parse::<i64>().ok()) {
        if !store::playlists().iter().any(|p| p.id == pid) {
            return false;
        }
        sections::playlist::build(pid)
    } else {
        let section = {
            let u = ui.borrow();
            u.sections.iter().find(|s| s.id == id).map(|s| (s.id, s.title, s.description, (s.files)(), s.build, s.fill, s.bare))
        };
        let Some((sid, title, description, files, build, fill, bare)) = section else { return false };
        let page = widgets::page(sid, title, description, &files);
        if fill {
            page.fill();
        }
        if bare {
            page.bare();
        }
        build(&page);
        page.root.upcast()
    };
    mark_page(&page, narrow());
    let stack = ui.borrow().stack.clone();
    stack.add_named(&page, Some(id));
    ui.borrow_mut().pages.insert(id.to_string(), page);
    true
}

pub fn navigate(id: &str) {
    // Deep links into a show or a movie: `tv:<show>`, `movie:<path>`.
    if let Some(name) = id.strip_prefix("tv:") {
        let key = crate::library::show_key(name);
        when_loaded(move || sections::shows::open(&key));
        return;
    }
    if let Some(path) = id.strip_prefix("movie:") {
        let path = std::path::PathBuf::from(path);
        when_loaded(move || sections::movies::open(&path));
        return;
    }
    let Some(ui) = ui() else { return };
    if id.starts_with("playlist:") && !store::loaded() {
        PENDING.with(|p| *p.borrow_mut() = Some(id.to_string()));
    }
    let id = if ensure_built(id) { id.to_string() } else { "home".to_string() };
    if !ensure_built(&id) {
        return;
    }
    let mut u = ui.borrow_mut();
    if let Some(prev) = u.nav_items.get(&u.current) {
        prev.remove_css_class("active");
    }
    if let Some(b) = u.nav_items.get(&id) {
        b.add_css_class("active");
    }
    u.stack.set_visible_child_name(&id);
    u.current = id.clone();
    // The player has its own controls; the bar is for everywhere else.
    let full = u.window.is_fullscreen();
    u.bar.set_visible(id != "now-playing" && !full);
    let search = u.search.clone();
    let leaving_player = full && id != "now-playing";
    drop(u);
    if leaving_player {
        set_fullscreen(false);
    }
    if id != "search" {
        if !search.text().is_empty() {
            ui.borrow_mut().before_search.clear();
            search.set_text("");
        }
        prefs::update(|p| p.last_section = id);
    }
}

/// Run `f` once the library is in memory.
fn when_loaded(f: impl FnOnce() + 'static) {
    if store::loaded() {
        f();
        return;
    }
    let f = std::cell::RefCell::new(Some(f));
    glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
        if !store::loaded() {
            return glib::ControlFlow::Continue;
        }
        if let Some(f) = f.borrow_mut().take() {
            f();
        }
        glib::ControlFlow::Break
    });
}

/// Drop a page so it's rebuilt next time (a deleted playlist).
pub fn remove_page(id: &str) {
    let Some(ui) = ui() else { return };
    let old = ui.borrow_mut().pages.remove(id);
    if let Some(old) = old {
        ui.borrow().stack.remove(&old);
    }
    if ui.borrow().current == id {
        navigate("queue");
    }
}

fn refresh_playlists() {
    let Some(ui) = ui() else { return };
    let bx = ui.borrow().playlist_box.clone();
    while let Some(c) = bx.first_child() {
        bx.remove(&c);
    }
    ui.borrow_mut().nav_items.retain(|k, _| !k.starts_with("playlist:"));
    let current = ui.borrow().current.clone();
    for p in store::playlists() {
        let (button, label) = nav_button("nmp-playlist-symbolic", &p.name, &p.name);
        label.add_css_class("compact-hide");
        let id = format!("playlist:{}", p.id);
        if id == current {
            button.add_css_class("active");
        }
        let target = id.clone();
        button.connect_clicked(move |_| navigate(&target));
        sections::playlist::accept_drops(&button, p.id);
        bx.append(&button);
        ui.borrow_mut().nav_items.insert(id, button);
    }
    let nav = ui.borrow().nav.clone();
    set_compact_hidden(&nav, nav.has_css_class("compact"));
    if let Some(id) = PENDING.with(|p| p.borrow_mut().take()) {
        navigate(&id);
    }
}

/// Show a short message at the bottom of the window.
pub fn toast(message: &str) {
    let Some(ui) = ui() else {
        eprintln!("media-player: {message}");
        return;
    };
    let overlay = ui.borrow().overlay.clone();
    let label = gtk::Label::new(Some(message));
    label.set_wrap(true);
    label.set_max_width_chars(70);
    let bx = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    bx.add_css_class("toast");
    bx.append(&label);
    bx.set_halign(gtk::Align::Center);
    bx.set_valign(gtk::Align::End);
    overlay.add_overlay(&bx);
    glib::timeout_add_local_once(std::time::Duration::from_millis(3500), move || {
        overlay.remove_overlay(&bx);
    });
}

pub fn current() -> String {
    ui().map(|u| u.borrow().current.clone()).unwrap_or_default()
}

pub fn window() -> Option<gtk::ApplicationWindow> {
    ui().map(|u| u.borrow().window.clone())
}

pub fn is_active() -> bool {
    window().is_some_and(|w| w.is_active() && w.is_visible())
}

fn snapshot_and_quit(app: &gtk::Application, out: std::path::PathBuf) {
    let Some(ui) = ui() else { return };
    let window = ui.borrow().window.clone();
    window.set_opacity(0.01);
    // A distinct title lets a window rule float it at a set size for screenshots.
    window.set_title(Some("Nexus Media Player snapshot"));
    window.set_default_size(
        std::env::var("NMP_SNAPSHOT_W").ok().and_then(|v| v.parse().ok()).unwrap_or(1180),
        std::env::var("NMP_SNAPSHOT_H").ok().and_then(|v| v.parse().ok()).unwrap_or(820),
    );
    window.present();
    let app = app.clone();
    let delay = std::env::var("NMP_SNAPSHOT_DELAY").ok().and_then(|v| v.parse().ok()).unwrap_or(2500);
    if std::env::var_os("NMP_SNAPSHOT_PLAY").is_some() {
        glib::timeout_add_local_once(std::time::Duration::from_millis(300), player::play);
    }
    glib::timeout_add_local_once(std::time::Duration::from_millis(delay), move || {
        if let Some(child) = window.child() {
            let paintable = gtk::WidgetPaintable::new(Some(&child));
            let (w, h) = (child.width(), child.height());
            let snapshot = gtk::Snapshot::new();
            snapshot.append_color(&gdk::RGBA::BLACK, &gtk::graphene::Rect::new(0.0, 0.0, w as f32, h as f32));
            paintable.snapshot(&snapshot, w as f64, h as f64);
            if let (Some(node), Some(renderer)) = (snapshot.to_node(), window.renderer()) {
                let texture = renderer.render_texture(node, None);
                match texture.save_to_png(&out) {
                    Ok(()) => println!("snapshot {w}x{h} -> {}", out.display()),
                    Err(e) => eprintln!("snapshot failed: {e}"),
                }
            }
        }
        player::shutdown();
        app.quit();
    });
}
