//! All videos: one sortable table of everything, filtered by kind.

use super::{library_stack, scan_banner};
use crate::fmt;
use crate::library::{Kind, store};
use crate::videotable::{Options, VideoTable};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static FILTER: RefCell<String> = RefCell::new("all".into());
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let content = widgets::vbox(0);
    let toolbar = widgets::hbox(10);
    toolbar.add_css_class("toolbar");
    let count = widgets::label("", "dim");
    count.add_css_class("mono");
    count.set_hexpand(true);
    toolbar.append(&count);
    let table = VideoTable::new(Options::default());
    let refresh = {
        let (table, count) = (table.clone(), count.clone());
        Rc::new(move || {
            let filter = FILTER.with(|f| f.borrow().clone());
            let all = store::videos();
            let shown: Vec<_> = all
                .iter()
                .filter(|v| match filter.as_str() {
                    "movies" => v.kind == Kind::Movie,
                    "episodes" => v.kind == Kind::Episode,
                    "other" => v.kind == Kind::Other,
                    _ => true,
                })
                .cloned()
                .collect();
            let secs: f64 = shown.iter().map(|v| v.duration).sum();
            count.set_text(&format!("{} · {}", fmt::count(shown.len(), "video", "videos"), fmt::total(secs)));
            table.set(&shown);
        })
    };
    let r = refresh.clone();
    toolbar.append(&widgets::segmented(
        &widgets::opts(&[("all", "All"), ("movies", "Movies"), ("episodes", "Episodes"), ("other", "Other")]),
        "all",
        move |id| {
            FILTER.with(|f| *f.borrow_mut() = id);
            r();
        },
    ));
    content.append(&toolbar);
    content.append(&table.root);
    let empty = widgets::vbox(0);
    page.body.append(&library_stack(&content, || true, empty));
    refresh();
    store::subscribe(&content, move |c| {
        if c == store::Change::Library {
            refresh();
        }
    });
}
