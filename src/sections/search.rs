//! Search results: shows, movies and videos matching every word typed in the
//! sidebar's search field.

use crate::fmt;
use crate::library::art::Art;
use crate::library::{Kind, store};
use crate::videotable::{Options, VideoTable};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct Ui {
    shows_group: gtk::Box,
    shows: gtk::FlowBox,
    movies_group: gtk::Box,
    movies: gtk::FlowBox,
    videos_title: gtk::Label,
    table: VideoTable,
    stack: gtk::Stack,
    nothing: gtk::Label,
    query: String,
}

thread_local! {
    static UI: RefCell<Option<Rc<RefCell<Ui>>>> = const { RefCell::new(None) };
}

const MAX_SHOWS: usize = 12;
const MAX_MOVIES: usize = 12;

fn flow() -> gtk::FlowBox {
    let f = gtk::FlowBox::new();
    f.set_selection_mode(gtk::SelectionMode::None);
    f.set_row_spacing(8);
    f.set_column_spacing(8);
    f.set_homogeneous(false);
    f.set_max_children_per_line(12);
    f
}

pub fn build(page: &Page) {
    let content = widgets::vbox(18);
    content.set_vexpand(true);

    let shows_group = widgets::vbox(8);
    shows_group.append(&widgets::label("SHOWS", "group-title"));
    let shows = flow();
    shows_group.append(&shows);
    content.append(&shows_group);

    let movies_group = widgets::vbox(8);
    movies_group.append(&widgets::label("MOVIES", "group-title"));
    let movies = flow();
    movies_group.append(&movies);
    content.append(&movies_group);

    let videos_group = widgets::vbox(8);
    videos_group.set_vexpand(true);
    let videos_title = widgets::label("VIDEOS", "group-title");
    videos_group.append(&videos_title);
    let table = VideoTable::new(Options::default());
    table.root.set_size_request(-1, 240);
    videos_group.append(&table.root);
    content.append(&videos_group);

    let stack = gtk::Stack::new();
    stack.set_vexpand(true);
    stack.add_named(&content, Some("results"));
    let nothing = widgets::label("", "dim");
    nothing.set_halign(gtk::Align::Center);
    nothing.set_valign(gtk::Align::Center);
    stack.add_named(&nothing, Some("nothing"));
    page.body.append(&stack);

    let ui = Rc::new(RefCell::new(Ui {
        shows_group,
        shows,
        movies_group,
        movies,
        videos_title,
        table,
        stack: stack.clone(),
        nothing,
        query: String::new(),
    }));
    UI.with(|u| *u.borrow_mut() = Some(ui));
    store::subscribe(&stack, |c| {
        if c == store::Change::Library {
            let q = UI.with(|u| u.borrow().as_ref().map(|u| u.borrow().query.clone())).unwrap_or_default();
            if !q.is_empty() {
                set_query(&q);
            }
        }
    });
}

fn clear(f: &gtk::FlowBox) {
    while let Some(c) = f.first_child() {
        f.remove(&c);
    }
}

/// A small poster with a title and a second line, as a button.
fn chip(art: &str, title: &str, meta: &str, on: impl Fn() + 'static) -> gtk::Button {
    let card = gtk::Button::new();
    card.add_css_class("flat");
    card.add_css_class("result-chip");
    let row = widgets::hbox(10);
    let a = Art::new(40, 60, true, "nmp-movie-symbolic");
    a.set_key(art);
    row.append(&a.root);
    let text = widgets::vbox(1);
    text.set_valign(gtk::Align::Center);
    let t = widgets::label(title, "card-title");
    t.set_ellipsize(gtk::pango::EllipsizeMode::End);
    t.set_max_width_chars(24);
    let m = widgets::label(meta, "card-meta");
    m.set_ellipsize(gtk::pango::EllipsizeMode::End);
    m.set_max_width_chars(24);
    text.append(&t);
    text.append(&m);
    row.append(&text);
    card.set_child(Some(&row));
    card.connect_clicked(move |_| on());
    card
}

pub fn set_query(q: &str) {
    let Some(ui) = UI.with(|u| u.borrow().clone()) else { return };
    let mut u = ui.borrow_mut();
    u.query = q.to_string();
    let terms: Vec<String> = q.to_lowercase().split_whitespace().map(str::to_string).collect();
    let hit = |hay: &str| terms.iter().all(|t| hay.contains(t.as_str()));

    let videos: Vec<_> = store::videos().into_iter().filter(|v| hit(&v.haystack())).collect();
    let movies: Vec<_> = videos.iter().filter(|v| v.kind == Kind::Movie).take(MAX_MOVIES).cloned().collect();
    let shows: Vec<_> = store::shows()
        .into_iter()
        .filter(|s| hit(&format!("{} {} {}", s.name, s.genres, s.plot).to_lowercase()))
        .take(MAX_SHOWS)
        .collect();

    clear(&u.shows);
    for s in &shows {
        let key = s.key.clone();
        u.shows.append(&chip(&s.art(), &s.name, &fmt::count(s.count(), "episode", "episodes"), move || super::shows::open(&key)));
    }
    u.shows_group.set_visible(!shows.is_empty());

    clear(&u.movies);
    for m in &movies {
        let path = m.path.clone();
        let meta = m.year.map(|y| y.to_string()).unwrap_or_default();
        u.movies.append(&chip(m.card_art(), &m.title, &meta, move || super::movies::open(&path)));
    }
    u.movies_group.set_visible(!movies.is_empty());

    u.videos_title.set_text(&format!("VIDEOS · {}", fmt::thousands(videos.len())));
    u.videos_title.set_visible(!videos.is_empty());
    u.table.root.set_visible(!videos.is_empty());
    u.table.set(&videos);

    let any = !videos.is_empty() || !shows.is_empty();
    u.nothing.set_text(&format!("Nothing in your library matches “{q}”."));
    u.stack.set_visible_child_name(if any { "results" } else { "nothing" });
}

/// Enter in the search field: move into the results.
pub fn focus_results() {
    if let Some(ui) = UI.with(|u| u.borrow().clone()) {
        let table = ui.borrow().table.clone();
        table.focus();
    }
}
