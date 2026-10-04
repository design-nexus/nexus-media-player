//! Movies: a poster grid, sorted and filtered, opening into a movie's page.

use super::{library_stack, scan_banner};
use crate::library::art::Art;
use crate::library::{Video, store, tmdb};
use crate::views::{self, Item, PosterGrid};
use crate::widgets::{self, Page};
use crate::{cmd, fmt, menu, paths, player, prefs, window};
use gtk::prelude::*;
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

struct Ui {
    inner: gtk::Stack,
    detail: gtk::Box,
    open: Option<PathBuf>,
}

thread_local! {
    static UI: RefCell<Option<Ui>> = const { RefCell::new(None) };
    static SORT: RefCell<String> = RefCell::new("title".into());
    static FILTER: RefCell<String> = RefCell::new("all".into());
}

fn sorted(mut v: Vec<Rc<Video>>) -> Vec<Rc<Video>> {
    let sort = SORT.with(|s| s.borrow().clone());
    match sort.as_str() {
        "year" => v.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| store::sort_key(&a.title).cmp(&store::sort_key(&b.title)))),
        "added" => v.sort_by_key(|x| std::cmp::Reverse(x.added)),
        "rating" => v.sort_by(|a, b| b.rating.unwrap_or(0.0).total_cmp(&a.rating.unwrap_or(0.0))),
        _ => v.sort_by_key(|x| (store::sort_key(&x.title), x.year)),
    }
    let filter = FILTER.with(|f| f.borrow().clone());
    v.retain(|x| match filter.as_str() {
        "unwatched" => !x.watch.get().watched,
        "watched" => x.watch.get().watched,
        _ => true,
    });
    v
}

pub fn build(page: &Page) {
    page.body.append(&scan_banner());
    let inner = gtk::Stack::new();
    inner.set_transition_type(gtk::StackTransitionType::Crossfade);
    inner.set_vexpand(true);

    // ----- Grid -----
    let browse = widgets::vbox(12);
    let toolbar = widgets::hbox(10);
    toolbar.add_css_class("toolbar");
    let count = widgets::label("", "dim");
    count.add_css_class("mono");
    count.set_hexpand(true);
    toolbar.append(&count);
    let grid = PosterGrid::new(|item| {
        if let Item::Video(v) = item {
            open(&v.path);
        }
    });
    let refresh = {
        let (grid, count) = (grid.clone(), count.clone());
        Rc::new(move || {
            let all = store::movies();
            let shown = sorted(all.clone());
            count.set_text(&if shown.len() == all.len() {
                fmt::count(all.len(), "movie", "movies")
            } else {
                format!("{} of {}", fmt::thousands(shown.len()), fmt::count(all.len(), "movie", "movies"))
            });
            grid.set(shown.into_iter().map(Item::Video).collect());
        })
    };
    let r = refresh.clone();
    let filter = widgets::segmented(
        &widgets::opts(&[("all", "All"), ("unwatched", "Unwatched"), ("watched", "Watched")]),
        "all",
        move |id| {
            FILTER.with(|f| *f.borrow_mut() = id);
            r();
        },
    );
    toolbar.append(&filter);
    let sort_opts = widgets::opts(&[("title", "Title"), ("year", "Year"), ("added", "Recently added"), ("rating", "Rating")]);
    let sort = widgets::dropdown(&sort_opts, "title");
    sort.set_tooltip_text(Some("Sort by"));
    let r = refresh.clone();
    sort.connect_selected_notify(move |d| {
        if let Some((id, _)) = sort_opts.get(d.selected() as usize) {
            SORT.with(|s| *s.borrow_mut() = id.clone());
            r();
        }
    });
    toolbar.append(&sort);
    browse.append(&toolbar);
    browse.append(&grid.root);
    inner.add_named(&browse, Some("grid"));

    // ----- Detail -----
    let detail = widgets::vbox(0);
    let scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::External)
        .child(&detail)
        .vexpand(true)
        .build();
    inner.add_named(&scroll, Some("detail"));

    let empty = widgets::empty_state(
        "nmp-movie-symbolic",
        "No movies",
        "Videos longer than 40 minutes that aren't episodes show up here, named from their file or folder (<tt>Heat (1995)/heat.mkv</tt>) or a <tt>movie.nfo</tt>.",
        None,
    );
    page.body.append(&library_stack(&inner, || !store::movies().is_empty(), empty));
    UI.with(|u| *u.borrow_mut() = Some(Ui { inner: inner.clone(), detail, open: None }));

    refresh();
    let g = grid.clone();
    store::subscribe(&inner, move |c| match c {
        store::Change::Library => {
            refresh();
            // The open movie may have new details.
            if let Some(p) = UI.with(|u| u.borrow().as_ref().and_then(|u| u.open.clone())) {
                show_detail(&p);
            }
        }
        store::Change::Watch => g.refresh(),
        _ => {}
    });
}

/// Open a movie's page.
pub fn open(path: &Path) {
    window::navigate("movies");
    show_detail(path);
}

fn back() {
    UI.with(|u| {
        if let Some(u) = u.borrow_mut().as_mut() {
            u.open = None;
            u.inner.set_visible_child_name("grid");
        }
    });
}

fn info_row(grid: &gtk::Grid, row: i32, name: &str, value: &str) {
    if value.is_empty() {
        return;
    }
    let n = widgets::label(name, "info-name");
    n.set_valign(gtk::Align::Start);
    let v = widgets::label(value, "info-value");
    v.set_wrap(true);
    v.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    v.set_selectable(true);
    v.set_hexpand(true);
    grid.attach(&n, 0, row, 1, 1);
    grid.attach(&v, 1, row, 1, 1);
}

fn langs(list: &str) -> String {
    list.split(',').filter(|s| !s.is_empty()).map(player::language_name).collect::<Vec<_>>().join(", ")
}

/// The file details shared by movie and video pages.
pub fn file_info(v: &Video) -> gtk::Grid {
    let grid = gtk::Grid::new();
    grid.add_css_class("info-grid");
    grid.set_column_spacing(18);
    grid.set_row_spacing(6);
    let mut row = 0;
    let mut add = |n: &str, val: String| {
        info_row(&grid, row, n, &val);
        row += 1;
    };
    add("File", paths::pretty(&v.path));
    add("Size", if v.size > 0 { fmt::size(v.size) } else { String::new() });
    let mut video = Vec::new();
    if v.width > 0 {
        video.push(format!("{}×{}", v.width, v.height));
    }
    if !v.vcodec.is_empty() {
        video.push(v.vcodec.clone());
    }
    add("Picture", video.join(" · "));
    let mut audio = vec![langs(&v.audio_langs)];
    if !v.acodec.is_empty() {
        audio.push(v.acodec.clone());
    }
    audio.retain(|s| !s.is_empty());
    add("Sound", audio.join(" · "));
    add("Subtitles", langs(&v.sub_langs));
    grid
}

fn show_detail(path: &Path) {
    let Some(v) = store::find(path) else {
        back();
        return;
    };
    let Some((inner, detail)) = UI.with(|u| {
        let mut u = u.borrow_mut();
        let u = u.as_mut()?;
        u.open = Some(path.to_path_buf());
        Some((u.inner.clone(), u.detail.clone()))
    }) else {
        return;
    };
    while let Some(c) = detail.first_child() {
        detail.remove(&c);
    }
    detail.append(&views::back_button("Movies", back));
    let art = Art::poster(220);
    art.set_key(v.card_art());
    let h = views::detail_header(art, "Movie");
    h.set_text(&v.title, &views::video_meta(&v), &v.genres, &v.plot);
    h.set_backdrop(&v.backdrop);

    let resume = v.in_progress();
    let play = widgets::labeled_button(
        "media-playback-start-symbolic",
        &if resume { format!("Resume from {}", fmt::time(v.watch.get().position)) } else { "Play".into() },
    );
    play.add_css_class("suggested-action");
    let v2 = v.clone();
    play.connect_clicked(move |_| views::play_video(&v2));
    h.actions.append(&play);
    if resume {
        let b = widgets::labeled_button("media-skip-backward-symbolic", "From the start");
        let v2 = v.clone();
        b.connect_clicked(move |_| {
            store::update_watch(&v2, |w| w.position = 0.0);
            views::play_video(&v2);
        });
        h.actions.append(&b);
    }
    let watched = v.watch.get().watched;
    let mark = widgets::labeled_button(
        if watched { "edit-undo-symbolic" } else { "object-select-symbolic" },
        if watched { "Mark as unwatched" } else { "Mark as watched" },
    );
    let v2 = v.clone();
    let p2 = path.to_path_buf();
    mark.connect_clicked(move |_| {
        store::set_watched(std::slice::from_ref(&v2), !watched);
        show_detail(&p2);
    });
    h.actions.append(&mark);
    let more = widgets::icon_button("view-more-symbolic", "More");
    let v2 = v.clone();
    more.connect_clicked(move |b| {
        let extra: Vec<menu::Extra> = Vec::new();
        menu::video_menu(b.upcast_ref(), b.width() as f64 / 2.0, b.height() as f64, vec![v2.clone()], extra);
    });
    h.actions.append(&more);
    detail.append(&h.root);

    let g = widgets::vbox(8);
    g.add_css_class("settings-group");
    g.append(&widgets::label("FILE", "group-title"));
    g.append(&file_info(&v));
    let row = widgets::hbox(8);
    row.set_margin_top(6);
    let dir = v.path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let open_dir = widgets::labeled_button("folder-open-symbolic", "Open folder");
    open_dir.connect_clicked(move |_| cmd::spawn(&["xdg-open", &dir]));
    row.append(&open_dir);
    if tmdb::Settings::from_prefs(&prefs::get()).is_some() {
        let fix = widgets::labeled_button("system-search-symbolic", "Fix match…");
        let v2 = v.clone();
        fix.connect_clicked(move |_| menu::fix_match(&tmdb::movie_target(&v2.path), "movie", &v2.title, v2.year));
        row.append(&fix);
    }
    g.append(&row);
    detail.append(&g);
    inner.set_visible_child_name("detail");
}
