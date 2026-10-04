//! TV shows: a poster grid opening into a show's page, with its seasons and
//! episodes.

use super::{library_stack, scan_banner};
use crate::library::art::Art;
use crate::library::store::{self, Show};
use crate::views::{self, Item, PosterGrid};
use crate::widgets::{self, Page};
use crate::{fmt, menu, prefs, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::rc::Rc;

struct Ui {
    inner: gtk::Stack,
    detail: gtk::Box,
    /// The open show and season (index into its seasons).
    open: Option<(String, usize)>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    /// The open page came from this section's own grid (so Back returns there).
    static FROM_GRID: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SORT: RefCell<String> = RefCell::new(prefs::get().shows_sort);
    static FILTER: RefCell<String> = RefCell::new(prefs::get().shows_filter);
}

fn sorted(mut v: Vec<Rc<Show>>) -> Vec<Rc<Show>> {
    match SORT.with(|s| s.borrow().clone()).as_str() {
        "added" => v.sort_by_key(|s| std::cmp::Reverse(s.added())),
        "watched" => v.sort_by_key(|s| std::cmp::Reverse(s.last_played())),
        _ => v.sort_by_key(|s| store::sort_key(&s.name)),
    }
    if FILTER.with(|f| f.borrow().as_str() == "unwatched") {
        v.retain(|s| s.unwatched() > 0);
    }
    v
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let inner = gtk::Stack::new();
    inner.set_transition_type(gtk::StackTransitionType::Crossfade);
    inner.set_vexpand(true);

    let browse = widgets::vbox(12);
    let toolbar = widgets::hbox(10);
    toolbar.add_css_class("toolbar");
    let count = widgets::label("", "dim");
    count.add_css_class("mono");
    count.set_hexpand(true);
    toolbar.append(&count);
    let grid = PosterGrid::new(|item| {
        if let Item::Show(s) = item {
            open(&s.key);
        }
    });
    let refresh = {
        let (grid, count) = (grid.clone(), count.clone());
        Rc::new(move || {
            let all = store::shows();
            let shown = sorted(all.clone());
            let eps: usize = all.iter().map(|s| s.count()).sum();
            count.set_text(&format!("{} · {}", fmt::count(all.len(), "show", "shows"), fmt::count(eps, "episode", "episodes")));
            grid.set(shown.into_iter().map(Item::Show).collect());
        })
    };
    let r = refresh.clone();
    toolbar.append(&widgets::segmented(
        &widgets::opts(&[("all", "All"), ("unwatched", "Unwatched")]),
        &FILTER.with(|f| f.borrow().clone()),
        move |id| {
            prefs::update(|p| p.shows_filter = id.clone());
            FILTER.with(|f| *f.borrow_mut() = id);
            r();
        },
    ));
    let sort_opts = widgets::opts(&[("title", "Title"), ("added", "Recently added"), ("watched", "Recently watched")]);
    let sort = widgets::dropdown(&sort_opts, &SORT.with(|s| s.borrow().clone()));
    sort.set_tooltip_text(Some("Sort by"));
    let r = refresh.clone();
    sort.connect_selected_notify(move |d| {
        if let Some((id, _)) = sort_opts.get(d.selected() as usize) {
            prefs::update(|p| p.shows_sort = id.clone());
            SORT.with(|s| *s.borrow_mut() = id.clone());
            r();
        }
    });
    toolbar.append(&sort);
    browse.append(&toolbar);
    browse.append(&grid.root);
    inner.add_named(&browse, Some("grid"));

    let detail = widgets::vbox(0);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&detail)
        .vexpand(true)
        .build();
    inner.add_named(&scroll, Some("detail"));

    let empty = widgets::empty_state(
        "nmp-tv-symbolic",
        "No shows",
        "Episodes are found by name (<tt>Show S01E02.mkv</tt>, <tt>Show 1x02.mkv</tt>) or by folder (<tt>Show/Season 1/02.mkv</tt>).",
        None,
    );
    page.body.append(&library_stack(&inner, || !store::shows().is_empty(), empty));
    UI.with(|u| *u.borrow_mut() = Some(Ui { inner: inner.clone(), detail, open: None }));

    refresh();
    let g = grid.clone();
    store::subscribe(&inner, move |c| match c {
        store::Change::Library => {
            refresh();
            reopen();
        }
        store::Change::Watch => {
            g.refresh();
            // Positions move every few seconds while playing; don't rebuild
            // the open show under the pointer for those.
            if window::current() != "tv" || crate::player::state() != crate::player::State::Playing {
                reopen();
            }
        }
        _ => {}
    });
}

fn reopen() {
    if let Some((key, season)) = UI.with(|u| u.borrow().as_ref().and_then(|u| u.open.clone())) {
        show_detail(&key, Some(season));
    }
}

/// Open a show's page.
pub fn open(key: &str) {
    FROM_GRID.with(|f| f.set(window::current() == "tv"));
    window::navigate("tv");
    show_detail(key, None);
}

/// Back from an open page: true when that's all Back should do here. A page
/// opened from elsewhere closes too, but Back goes on to where it came from.
pub fn back_out() -> bool {
    let open = UI.with(|u| u.borrow().as_ref().is_some_and(|u| u.open.is_some()));
    if open {
        back();
    }
    open && FROM_GRID.with(|f| f.get())
}

fn back() {
    UI.with(|u| {
        if let Some(u) = u.borrow_mut().as_mut() {
            u.open = None;
            u.inner.set_visible_child_name("grid");
        }
    });
}

fn show_detail(key: &str, season: Option<usize>) {
    let Some(show) = store::show(key) else {
        back();
        return;
    };
    // Start on the season of the next episode to watch.
    let next = show.next_episode();
    let season = season.unwrap_or_else(|| {
        next.as_ref().and_then(|n| show.seasons.iter().position(|s| s.episodes.iter().any(|e| e.path == n.path))).unwrap_or(0)
    });
    let season = season.min(show.seasons.len().saturating_sub(1));
    let Some((inner, detail)) = UI.with(|u| {
        let mut u = u.borrow_mut();
        let u = u.as_mut()?;
        u.open = Some((key.to_string(), season));
        Some((u.inner.clone(), u.detail.clone()))
    }) else {
        return;
    };
    while let Some(c) = detail.first_child() {
        detail.remove(&c);
    }
    detail.append(&views::back_button("TV shows", back));

    let art = Art::poster(220);
    art.set_key(&show.art());
    let h = views::detail_header(art, "Show");
    let mut meta = Vec::new();
    if let Some(y) = show.year {
        meta.push(y.to_string());
    }
    let real = show.seasons.iter().filter(|s| s.number.is_some_and(|n| n > 0)).count();
    if real > 0 {
        meta.push(fmt::count(real, "season", "seasons"));
    }
    meta.push(fmt::count(show.count(), "episode", "episodes"));
    let unwatched = show.unwatched();
    if unwatched > 0 && unwatched < show.count() {
        meta.push(format!("{} unwatched", fmt::thousands(unwatched)));
    }
    if let Some(r) = show.rating {
        meta.push(format!("★ {r:.1}"));
    }
    h.set_text(&show.name, &meta.join(" · "), &show.genres, &show.plot);
    h.set_backdrop(&show.backdrop);
    if let Some(n) = &next {
        let label = if n.in_progress() { format!("Resume {}", n.code()) } else { format!("Play {}", n.code()) };
        let b = widgets::labeled_button("media-playback-start-symbolic", label.trim());
        b.add_css_class("suggested-action");
        let n2 = n.clone();
        b.connect_clicked(move |_| views::play_video(&n2));
        h.actions.append(&b);
    }
    let all_watched = unwatched == 0;
    let mark = widgets::labeled_button(
        if all_watched { "edit-undo-symbolic" } else { "object-select-symbolic" },
        if all_watched { "Mark as unwatched" } else { "Mark all watched" },
    );
    let eps: Vec<_> = show.episodes().cloned().collect();
    let k = key.to_string();
    mark.connect_clicked(move |_| {
        store::set_watched(&eps, !all_watched);
        show_detail(&k, Some(season));
    });
    h.actions.append(&mark);
    let more = widgets::icon_button("view-more-symbolic", "More");
    let s2 = show.clone();
    more.connect_clicked(move |b| menu::show_menu(b.upcast_ref(), b.width() as f64 / 2.0, b.height() as f64, s2.clone()));
    h.actions.append(&more);
    detail.append(&h.root);

    // ----- Seasons -----
    let group = widgets::vbox(10);
    group.add_css_class("settings-group");
    let bar = widgets::hbox(12);
    let current = &show.seasons[season];
    let title = widgets::label(&current.name().to_uppercase(), "group-title");
    title.set_hexpand(true);
    title.set_valign(gtk::Align::Center);
    bar.append(&title);
    // The season from its first unwatched episode, going on through the show.
    let start = current.episodes.iter().find(|e| !e.watch.get().watched).or(current.episodes.first()).cloned();
    if let Some(start) = start {
        let b = widgets::labeled_button("media-playback-start-symbolic", "Play season");
        b.add_css_class("flat");
        b.set_valign(gtk::Align::Center);
        b.set_tooltip_text(Some(&format!("From {}", start.code())));
        b.connect_clicked(move |_| views::play_video(&start));
        bar.append(&b);
    }
    if show.seasons.len() > 1 {
        let opts: Vec<(String, String)> = show
            .seasons
            .iter()
            .enumerate()
            .map(|(i, s)| {
                (
                    i.to_string(),
                    match s.number {
                        Some(0) => "Specials".into(),
                        Some(n) => format!("{n}"),
                        None => "Other".into(),
                    },
                )
            })
            .collect();
        let k = key.to_string();
        let pick = move |id: String| {
            if let Ok(i) = id.parse::<usize>() {
                let k = k.clone();
                // After the click finishes: this rebuilds the picker.
                gtk::glib::idle_add_local_once(move || show_detail(&k, Some(i)));
            }
        };
        if show.seasons.len() <= 8 {
            let seg = widgets::segmented(&opts, &season.to_string(), pick);
            seg.set_tooltip_text(Some("Season"));
            bar.append(&seg);
        } else {
            let dd = widgets::dropdown(&opts, &season.to_string());
            dd.set_tooltip_text(Some("Season"));
            dd.connect_selected_notify(move |d| {
                if let Some((id, _)) = opts.get(d.selected() as usize) {
                    pick(id.clone());
                }
            });
            bar.append(&dd);
        }
    }
    group.append(&bar);
    let left: f64 = current.episodes.iter().filter(|e| !e.watch.get().watched).map(|e| e.duration).sum();
    let note = widgets::label(
        &format!(
            "{} · {} unwatched",
            fmt::count(current.episodes.len(), "episode", "episodes"),
            if left > 0.0 { fmt::total(left) } else { "nothing".into() }
        ),
        "group-note",
    );
    note.add_css_class("mono");
    group.append(&note);
    let list = widgets::vbox(6);
    for e in &current.episodes {
        list.append(&views::episode_row(e));
    }
    group.append(&list);
    detail.append(&group);
    inner.set_visible_child_name("detail");
}
