//! Home: Continue watching, Next up and Recently added.

use super::{library_stack, scan_banner};
use crate::fmt;
use crate::library::art;
use crate::library::{Kind, Video, store};
use crate::views::{self, card_row, wide_card};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

const CARD: i32 = 240;

/// The banner at the top: the video you were last watching, over its backdrop.
struct Hero {
    root: gtk::Overlay,
    picture: gtk::Picture,
    title: gtk::Label,
    detail: gtk::Label,
    progress: gtk::ProgressBar,
    video: Rc<std::cell::RefCell<Option<Rc<Video>>>>,
}

impl Hero {
    fn new() -> Hero {
        let root = gtk::Overlay::new();
        root.add_css_class("home-hero");
        root.set_overflow(gtk::Overflow::Hidden);
        let picture = gtk::Picture::new();
        picture.set_content_fit(gtk::ContentFit::Cover);
        picture.set_can_shrink(true);
        // The text sizes the banner; the picture (an unmeasured overlay)
        // only fills it.
        root.set_child(Some(&widgets::vbox(0)));
        root.add_overlay(&picture);
        let scrim = widgets::hbox(0);
        scrim.add_css_class("detail-scrim");
        scrim.set_can_target(false);
        root.add_overlay(&scrim);
        let text = widgets::vbox(6);
        text.add_css_class("home-hero-text");
        text.set_valign(gtk::Align::End);
        text.append(&widgets::label("PICK UP WHERE YOU LEFT OFF", "group-title"));
        let title = widgets::label("", "detail-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::End);
        text.append(&title);
        let detail = widgets::label("", "dim");
        text.append(&detail);
        let progress = gtk::ProgressBar::new();
        progress.add_css_class("hero-progress");
        progress.set_size_request(260, -1);
        progress.set_halign(gtk::Align::Start);
        text.append(&progress);
        let video: Rc<std::cell::RefCell<Option<Rc<Video>>>> = Rc::default();
        let play = widgets::labeled_button("media-playback-start-symbolic", "Resume");
        play.add_css_class("suggested-action");
        play.set_halign(gtk::Align::Start);
        play.set_margin_top(6);
        let v = video.clone();
        play.connect_clicked(move |_| {
            if let Some(v) = v.borrow().clone() {
                views::play_video(&v);
            }
        });
        text.append(&play);
        root.add_overlay(&text);
        root.set_measure_overlay(&text, true);
        Hero { root, picture, title, detail, progress, video }
    }

    fn set(&self, v: Option<&Rc<Video>>) {
        self.root.set_visible(v.is_some());
        *self.video.borrow_mut() = v.cloned();
        let Some(v) = v else { return };
        let left = fmt::left(v.duration - v.watch.get().position);
        let (title, detail) = match v.kind {
            Kind::Episode => (v.show.clone(), format!("{} · {left}", v.episode_line())),
            _ => (v.title.clone(), left),
        };
        self.title.set_text(&title);
        self.detail.set_text(&detail);
        self.progress.set_fraction(v.progress());
        // The show's or movie's backdrop, else a frame from the video.
        let backdrop = match v.kind {
            Kind::Episode => store::show_of(v).map(|s| s.backdrop.clone()).unwrap_or_default(),
            _ => v.backdrop.clone(),
        };
        let key = if backdrop.is_empty() { v.wide_art().to_string() } else { backdrop };
        let picture = self.picture.clone();
        art::load(&key, false, move |tex| picture.set_paintable(tex.as_ref()));
    }
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let content = widgets::vbox(26);
    let hero = Hero::new();
    content.append(&hero.root);
    let (cont, cont_cards) = card_row("Continue watching");
    let (next, next_cards) = card_row("Next up");
    let (recent, recent_cards) = card_row("Recently added");
    content.append(&cont);
    content.append(&next);
    content.append(&recent);
    let empty = widgets::empty_state(
        "nmp-home-symbolic",
        "Nothing here yet",
        "Videos you start, shows you're following and what you add show up here.",
        None,
    );
    page.body.append(&library_stack(&content, || !store::videos().is_empty(), empty));

    let fill = move || {
        for b in [&cont_cards, &next_cards, &recent_cards] {
            while let Some(c) = b.first_child() {
                b.remove(&c);
            }
        }
        let c = store::continue_watching();
        // The first is the banner; the row has the rest.
        hero.set(c.first());
        let c: Vec<_> = c.into_iter().skip(1).collect();
        for v in &c {
            let left = fmt::left(v.duration - v.watch.get().position);
            let sub = match v.kind {
                Kind::Episode => format!("{} · {left}", v.code()),
                _ => left,
            };
            cont_cards.append(&wide_card(v, CARD, &sub));
        }
        cont.set_visible(!c.is_empty());
        let n = store::next_up();
        for v in &n {
            next_cards.append(&wide_card(v, CARD, ""));
        }
        next.set_visible(!n.is_empty());
        let r = store::recently_added();
        for v in &r {
            let sub = match v.kind {
                Kind::Episode => {
                    store::show_of(v).map(|s| fmt::count(s.count(), "episode", "episodes")).unwrap_or_else(|| v.code())
                }
                _ => v.year.map(|y| y.to_string()).unwrap_or_default(),
            };
            recent_cards.append(&wide_card(v, CARD, &sub));
        }
        recent.set_visible(!r.is_empty());
    };
    let fill = Rc::new(fill);
    fill();
    // Positions move every few seconds while playing; rebuild when the page
    // is next shown rather than under the pointer.
    let dirty = Rc::new(Cell::new(false));
    let (f, d, body) = (fill.clone(), dirty.clone(), page.body.clone());
    store::subscribe(&page.body, move |c| match c {
        store::Change::Library => f(),
        store::Change::Watch if !body.is_mapped() => d.set(true),
        // Not while playing: positions move every few seconds.
        store::Change::Watch if crate::player::state() != crate::player::State::Playing => f(),
        _ => {}
    });
    page.body.connect_map(move |_| {
        if dirty.replace(false) {
            fill();
        }
    });
}
