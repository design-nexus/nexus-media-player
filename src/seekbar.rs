//! The seek bar: elapsed time, the slider and the length, with a preview of
//! the frame (and chapter) under the pointer. Used by the player and the
//! player bar.

use crate::player::{self, Event, scrub};
use crate::{fmt, prefs, widgets};
use gtk::glib;
use gtk::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

pub struct SeekBar {
    pub root: gtk::Box,
}

/// The chapter at `secs`, if the video has named chapters.
fn chapter_at(secs: f64) -> Option<String> {
    let chapters = player::chapters();
    chapters.iter().rev().find(|(t, _)| *t <= secs).map(|(_, n)| n.clone()).filter(|n| !n.is_empty())
}

impl SeekBar {
    pub fn new() -> SeekBar {
        let root = widgets::hbox(10);
        root.add_css_class("seek-row");
        let pos = widgets::label("0:00", "time-readout");
        pos.add_css_class("mono");
        pos.set_xalign(1.0);
        pos.set_width_chars(7);
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 1.0);
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.add_css_class("seek");
        seek.set_focus_on_click(false);
        let dur = widgets::label("0:00", "time-readout");
        dur.add_css_class("mono");
        dur.add_css_class("clickable");
        dur.set_width_chars(8);
        dur.set_xalign(0.0);
        dur.set_tooltip_text(Some("Show the time left or the length"));
        dur.set_cursor_from_name(Some("pointer"));
        root.append(&pos);
        root.append(&seek);
        root.append(&dur);

        // ----- Preview -----
        let pop = gtk::Popover::new();
        pop.set_autohide(false);
        pop.set_has_arrow(false);
        pop.set_can_focus(false);
        pop.set_can_target(false);
        pop.set_position(gtk::PositionType::Top);
        pop.add_css_class("scrub-popover");
        pop.set_parent(&seek);
        let col = widgets::vbox(4);
        let frame = scrub::FramePaintable::default();
        let picture = gtk::Picture::for_paintable(&frame);
        picture.add_css_class("scrub-frame");
        picture.set_size_request(192, 108);
        picture.set_content_fit(gtk::ContentFit::Contain);
        col.append(&picture);
        let when = widgets::label("", "scrub-time");
        when.add_css_class("mono");
        when.set_halign(gtk::Align::Center);
        col.append(&when);
        let chapter = widgets::label("", "scrub-chapter");
        chapter.set_halign(gtk::Align::Center);
        chapter.set_ellipsize(gtk::pango::EllipsizeMode::End);
        chapter.set_max_width_chars(28);
        col.append(&chapter);
        pop.set_child(Some(&col));
        // Popovers must be unparented before their parent goes.
        let p2 = pop.clone();
        seek.connect_destroy(move |_| p2.unparent());

        let motion = gtk::EventControllerMotion::new();
        {
            let leave_pop = pop.clone();
            let (seek, pop) = (seek.clone(), pop.clone());
            let hover = move |x: f64| {
                let d = player::duration();
                let w = seek.width() as f64;
                if d <= 0.0 || w <= 0.0 || player::current().is_none() {
                    pop.popdown();
                    return;
                }
                // The trough is inset by the knob's radius on each side.
                let inset = 8.0;
                let secs = ((x - inset) / (w - 2.0 * inset).max(1.0)).clamp(0.0, 1.0) * d;
                let has_frame = scrub::show(&frame, secs);
                picture.set_visible(has_frame);
                when.set_text(&fmt::time(secs));
                let ch = chapter_at(secs);
                chapter.set_visible(ch.is_some());
                chapter.set_text(&ch.unwrap_or_default());
                pop.set_pointing_to(Some(&gtk::gdk::Rectangle::new(x as i32, 0, 1, 1)));
                if !pop.is_visible() {
                    pop.popup();
                }
            };
            let h = hover.clone();
            motion.connect_enter(move |_, x, _| h(x));
            motion.connect_motion(move |_, x, _| hover(x));
            motion.connect_leave(move |_| leave_pop.popdown());
        }
        seek.add_controller(motion);

        // While the user drags, don't move the bar under them; seek to
        // keyframes while dragging and exactly once they stop.
        let dragging: Rc<Cell<u32>> = Rc::new(Cell::new(0));
        let d = dragging.clone();
        let pos_label = pos.clone();
        seek.connect_change_value(move |_, _, v| {
            let gen_ = d.get().wrapping_add(1);
            d.set(gen_);
            pos_label.set_text(&fmt::time(v));
            player::seek_fast(v);
            let d2 = d.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(250), move || {
                if d2.get() == gen_ {
                    d2.set(0);
                    player::seek(v);
                }
            });
            glib::Propagation::Proceed
        });

        let refresh = {
            let (seek, pos, dur) = (seek.clone(), pos.clone(), dur.clone());
            move |e: Event| {
                if !matches!(e, Event::Track | Event::Position | Event::Seeked | Event::Options) {
                    return;
                }
                if dragging.get() != 0 && e == Event::Position {
                    return;
                }
                let d = player::duration();
                if (seek.adjustment().upper() - d.max(1.0)).abs() > 0.5 {
                    seek.set_range(0.0, d.max(1.0));
                }
                let p = player::position();
                dur.set_text(&if prefs::get().time_left && d > 0.0 {
                    format!("−{}", fmt::time(d - p))
                } else {
                    fmt::time(d)
                });
                seek.set_value(p);
                pos.set_text(&fmt::time(p));
                seek.set_sensitive(player::current().is_some());
            }
        };
        refresh(Event::Track);
        // Chapter ticks under the bar; dragging snaps to them.
        let marks = {
            let seek = seek.clone();
            move |e: Event| {
                if matches!(e, Event::Tracks | Event::Track) {
                    seek.clear_marks();
                    for (t, _) in player::chapters().iter().filter(|(t, _)| *t > 0.5) {
                        seek.add_mark(*t, gtk::PositionType::Bottom, None);
                    }
                }
            }
        };
        marks(Event::Tracks);
        player::subscribe(&root, marks);
        let click = gtk::GestureClick::new();
        click.connect_released(|_, _, _, _| player::set_time_left(!prefs::get().time_left));
        dur.add_controller(click);
        player::subscribe(&root, refresh);
        SeekBar { root }
    }
}
