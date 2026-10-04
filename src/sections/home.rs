//! Home: Continue watching, Next up and Recently added.

use super::{library_stack, scan_banner};
use crate::fmt;
use crate::library::{Kind, store};
use crate::views::{card_row, wide_card};
use crate::widgets::{self, Page};
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

const CARD: i32 = 240;

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let content = widgets::vbox(26);
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
