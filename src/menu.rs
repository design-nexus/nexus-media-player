//! Right-click menus for videos and shows, and the Fix match dialog.

use crate::library::store::{self, Show};
use crate::library::{Kind, Video, tmdb};
use crate::sections::{movies, playlist, shows};
use crate::{cmd, fmt, player, prefs, views, widgets, window};
use gtk::prelude::*;
use gtk::{gdk, glib};
use std::path::PathBuf;
use std::rc::Rc;

pub type Extra = (&'static str, Rc<dyn Fn()>);

pub fn added_text(n: usize, where_: &str) -> String {
    format!("Added {} {where_}.", fmt::count(n, "video", "videos"))
}

/// A popover menu at (x, y) in `anchor`, with a second page for playlists.
struct Menu {
    pop: gtk::Popover,
    pages: gtk::Stack,
    main: gtk::Box,
}

impl Menu {
    fn new(anchor: &gtk::Widget, x: f64, y: f64) -> Menu {
        let pop = gtk::Popover::new();
        pop.set_has_arrow(false);
        pop.set_parent(anchor);
        pop.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 1, 1)));
        pop.add_css_class("menu-popover");
        let pages = gtk::Stack::new();
        pages.set_vhomogeneous(false);
        pages.set_hhomogeneous(false);
        let main = widgets::vbox(1);
        pages.add_named(&main, Some("main"));
        pop.set_child(Some(&pages));
        let anchor_weak = anchor.downgrade();
        pop.connect_closed(move |p| {
            let p = p.clone();
            let a = anchor_weak.clone();
            glib::idle_add_local_once(move || {
                if a.upgrade().is_some() {
                    p.unparent();
                }
            });
        });
        Menu { pop, pages, main }
    }

    fn item(&self, label: &str, f: impl Fn() + 'static) -> gtk::Button {
        let b = gtk::Button::with_label(label);
        b.add_css_class("flat");
        b.add_css_class("menu-item");
        if let Some(l) = b.child().and_downcast::<gtk::Label>() {
            l.set_xalign(0.0);
        }
        let pop = self.pop.clone();
        b.connect_clicked(move |_| {
            pop.popdown();
            f();
        });
        b
    }

    fn add(&self, label: &str, f: impl Fn() + 'static) {
        self.main.append(&self.item(label, f));
    }

    fn separator(&self) {
        self.main.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    }

    /// "Add to playlist ›" and its page.
    fn playlists(&self, paths: Vec<PathBuf>) {
        let to = gtk::Button::new();
        to.add_css_class("flat");
        to.add_css_class("menu-item");
        let row = widgets::hbox(8);
        let l = widgets::label("Add to playlist", "");
        l.set_hexpand(true);
        row.append(&l);
        row.append(&gtk::Image::from_icon_name("pan-end-symbolic"));
        to.set_child(Some(&row));
        let pg = self.pages.clone();
        to.connect_clicked(move |_| pg.set_visible_child_name("playlists"));
        self.main.append(&to);

        let lists = widgets::vbox(1);
        let back = gtk::Button::new();
        back.add_css_class("flat");
        back.add_css_class("menu-item");
        let row = widgets::hbox(8);
        row.append(&gtk::Image::from_icon_name("pan-start-symbolic"));
        row.append(&widgets::label("Add to playlist", "menu-heading-inline"));
        back.set_child(Some(&row));
        let pg = self.pages.clone();
        back.connect_clicked(move |_| pg.set_visible_child_name("main"));
        lists.append(&back);
        lists.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        let p = paths.clone();
        lists.append(&self.item("New playlist…", move || playlist::new_dialog(p.clone())));
        let scroll_box = widgets::vbox(1);
        for pl in store::playlists() {
            let p = paths.clone();
            scroll_box.append(&self.item(&pl.name, move || playlist::append(pl.id, p.clone())));
        }
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::External)
            .propagate_natural_height(true)
            .max_content_height(320)
            .child(&scroll_box)
            .build();
        lists.append(&scroll);
        self.pages.add_named(&lists, Some("playlists"));
    }

    fn popup(&self) {
        self.pop.popup();
    }
}

/// The menu for one or more videos.
pub fn video_menu(anchor: &gtk::Widget, x: f64, y: f64, videos: Vec<Rc<Video>>, extra: Vec<Extra>) {
    let Some(first) = videos.first().cloned() else { return };
    let paths: Vec<PathBuf> = videos.iter().map(|v| v.path.clone()).collect();
    let m = Menu::new(anchor, x, y);
    let single = videos.len() == 1;
    if single {
        let v = first.clone();
        m.add(if v.in_progress() { "Resume" } else { "Play" }, move || views::play_video(&v));
        if first.in_progress() {
            let v = first.clone();
            m.add("Play from the start", move || {
                store::update_watch(&v, |w| w.position = 0.0);
                views::play_video(&v);
            });
        }
    } else {
        let p = paths.clone();
        m.add("Play", move || {
            player::play_paths(p.clone(), 0);
            window::navigate("now-playing");
        });
    }
    let p = paths.clone();
    m.add("Play next", move || {
        player::play_next(p.clone());
        window::toast(&added_text(p.len(), "to play next"));
    });
    let p = paths.clone();
    m.add("Add to queue", move || {
        player::enqueue(p.clone());
        window::toast(&added_text(p.len(), "to the queue"));
    });
    m.playlists(paths.clone());
    m.separator();
    let all_watched = videos.iter().all(|v| v.watch.get().watched);
    let vs = videos.clone();
    m.add(if all_watched { "Mark as unwatched" } else { "Mark as watched" }, move || store::set_watched(&vs, !all_watched));
    if single && first.id > 0 {
        match first.kind {
            Kind::Episode => {
                let key = first.show_key();
                m.add("Go to show", move || shows::open(&key));
            }
            Kind::Movie => {
                let path = first.path.clone();
                m.add("Movie details", move || movies::open(&path));
                if tmdb::Settings::from_prefs(&prefs::get()).is_some() {
                    let v = first.clone();
                    m.add("Fix match…", move || fix_match(&tmdb::movie_target(&v.path), "movie", &v.title, v.year));
                }
            }
            Kind::Other => {}
        }
    }
    if !crate::library::is_url(&first.path) {
        let dir = first.path.parent().map(|d| d.to_path_buf());
        m.add("Open folder", move || {
            if let Some(d) = &dir {
                cmd::spawn(&["xdg-open", &d.to_string_lossy()]);
            }
        });
    }
    if !extra.is_empty() {
        m.separator();
        for (label, f) in extra {
            m.add(label, move || f());
        }
    }
    m.popup();
}

/// The menu for a show.
pub fn show_menu(anchor: &gtk::Widget, x: f64, y: f64, show: Rc<Show>) {
    let m = Menu::new(anchor, x, y);
    if let Some(next) = show.next_episode() {
        let label = format!("{} {}", if next.in_progress() { "Resume" } else { "Play" }, next.code());
        m.add(&label, move || views::play_video(&next));
    }
    let eps: Vec<Rc<Video>> = show.episodes().cloned().collect();
    let paths: Vec<PathBuf> = eps.iter().map(|e| e.path.clone()).collect();
    let p = paths.clone();
    m.add("Play from the first episode", move || {
        player::play_paths(p.clone(), 0);
        window::navigate("now-playing");
    });
    let p = paths.clone();
    m.add("Add to queue", move || {
        player::enqueue(p.clone());
        window::toast(&added_text(p.len(), "to the queue"));
    });
    m.playlists(paths);
    m.separator();
    let all_watched = show.unwatched() == 0;
    let e = eps.clone();
    m.add(if all_watched { "Mark as unwatched" } else { "Mark as watched" }, move || store::set_watched(&e, !all_watched));
    if tmdb::Settings::from_prefs(&prefs::get()).is_some() {
        let s = show.clone();
        m.add("Fix match…", move || fix_match(&tmdb::show_target(&s.key), "tv", &s.name, s.year));
    }
    if !show.folder.is_empty() {
        let dir = show.folder.clone();
        m.add("Open folder", move || cmd::spawn(&["xdg-open", &dir]));
    }
    m.popup();
}

/// Search TMDB and pick what a movie or show really is.
pub fn fix_match(target: &str, kind: &'static str, query: &str, year: Option<i32>) {
    let Some(settings) = tmdb::Settings::from_prefs(&prefs::get()) else { return };
    let (dialog, card) = widgets::dialog("Fix match", 520);
    let d = widgets::label("Search TMDB and pick the right one. Its details and pictures replace what's shown now.", "dim");
    d.set_wrap(true);
    card.append(&d);
    let search = gtk::SearchEntry::new();
    search.set_text(query);
    card.append(&search);
    let results = widgets::vbox(4);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .min_content_height(320)
        .child(&results)
        .build();
    card.append(&scroll);
    let buttons = widgets::hbox(8);
    buttons.set_halign(gtk::Align::End);
    let cancel = gtk::Button::with_label("Not now");
    let d2 = dialog.clone();
    cancel.connect_clicked(move |_| d2.close());
    buttons.append(&cancel);
    card.append(&buttons);

    let target = target.to_string();
    let run = {
        let (results, dialog) = (results.clone(), dialog.clone());
        Rc::new(move |q: String, year: Option<i32>| {
            while let Some(c) = results.first_child() {
                results.remove(&c);
            }
            let spinner = gtk::Spinner::new();
            spinner.set_spinning(true);
            spinner.set_margin_top(20);
            results.append(&spinner);
            let s = settings.clone();
            let (results, dialog, target) = (results.clone(), dialog.clone(), target.clone());
            cmd::background(
                move || tmdb::search(&s, kind, &q, year),
                move |res| {
                    while let Some(c) = results.first_child() {
                        results.remove(&c);
                    }
                    let hits = match res {
                        Ok(h) => h,
                        Err(e) => {
                            results.append(&widgets::label(&e, "danger-text"));
                            return;
                        }
                    };
                    if hits.is_empty() {
                        results.append(&widgets::label("Nothing found. Try fewer words, or the original title.", "dim"));
                    }
                    for h in hits.into_iter().take(12) {
                        let b = gtk::Button::new();
                        b.add_css_class("flat");
                        b.add_css_class("match-row");
                        let col = widgets::vbox(2);
                        let title = match h.year {
                            Some(y) => format!("{} ({y})", h.title),
                            None => h.title.clone(),
                        };
                        col.append(&widgets::label(&title, "settings-option-title"));
                        if !h.overview.is_empty() {
                            let o = widgets::label(&h.overview, "settings-option-description");
                            o.set_wrap(true);
                            o.set_lines(2);
                            o.set_ellipsize(gtk::pango::EllipsizeMode::End);
                            o.set_max_width_chars(60);
                            col.append(&o);
                        }
                        b.set_child(Some(&col));
                        let (dialog, target) = (dialog.clone(), target.clone());
                        b.connect_clicked(move |_| {
                            dialog.close();
                            let t = target.clone();
                            cmd::background(
                                move || tmdb::set_match(&t, h.id),
                                |r| match r {
                                    Ok(()) => {
                                        window::toast("Looking it up again…");
                                        store::rescan(false);
                                    }
                                    Err(e) => window::toast(&format!("Couldn't save the match: {e}")),
                                },
                            );
                        });
                        results.append(&b);
                    }
                },
            );
        })
    };
    run(query.to_string(), year);
    search.connect_activate(move |e| run(e.text().trim().to_string(), None));
    dialog.present();
    search.grab_focus();
}
