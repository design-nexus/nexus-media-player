//! The bar under the library pages: a small live picture of what's playing,
//! transport, seek and volume. It hides on the player page, whose controls
//! float over the video.

use crate::library::Kind;
use crate::library::art::Art;
use crate::player::{self, Event, State, video};
use crate::seekbar::SeekBar;
use crate::{prefs, widgets, window};
use gtk::glib;
use gtk::prelude::*;
use std::cell::RefCell;

thread_local! {
    static NARROW_PARTS: RefCell<Vec<gtk::Widget>> = const { RefCell::new(Vec::new()) };
    /// Shown only in narrow windows, in place of the parts above.
    static NARROW_ONLY: RefCell<Vec<gtk::Widget>> = const { RefCell::new(Vec::new()) };
}

pub fn set_narrow(narrow: bool) {
    NARROW_PARTS.with(|p| {
        for w in p.borrow().iter() {
            w.set_visible(!narrow);
        }
    });
    NARROW_ONLY.with(|p| {
        for w in p.borrow().iter() {
            w.set_visible(narrow);
        }
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

pub fn build() -> gtk::Box {
    let bar = widgets::hbox(16);
    bar.add_css_class("player-bar");

    // ----- Now playing -----
    let info = widgets::hbox(12);
    info.add_css_class("player-info");
    info.set_size_request(240, -1);
    // The art sizes the thumbnail; the video sits over it as an overlay, which
    // isn't measured, so a full-size frame can't stretch the bar.
    let thumb = gtk::Overlay::new();
    thumb.add_css_class("mini-video");
    thumb.set_valign(gtk::Align::Center);
    thumb.set_overflow(gtk::Overflow::Hidden);
    let art = Art::wide(96);
    thumb.set_child(Some(&art.root));
    let picture = gtk::Picture::for_paintable(&video::paintable());
    picture.add_css_class("mini-video");
    picture.set_content_fit(gtk::ContentFit::Contain);
    picture.set_can_shrink(true);
    picture.set_visible(false);
    thumb.add_overlay(&picture);
    info.append(&thumb);
    let text = widgets::vbox(2);
    text.set_valign(gtk::Align::Center);
    text.set_hexpand(true);
    let title = widgets::label("Nothing playing", "player-title");
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    title.set_max_width_chars(26);
    let sub = widgets::label("", "player-artist");
    sub.set_ellipsize(gtk::pango::EllipsizeMode::End);
    sub.set_max_width_chars(26);
    text.append(&title);
    text.append(&sub);
    info.append(&text);
    // Close sits with what it closes, away from the volume.
    let close = widgets::icon_button("window-close-symbolic", "Close video");
    close.add_css_class("bar-close");
    close.set_valign(gtk::Align::Center);
    close.set_focus_on_click(false);
    info.append(&close);
    let click = gtk::GestureClick::new();
    click.connect_released(|_, _, _, _| window::navigate("now-playing"));
    info.add_controller(click);
    info.set_cursor_from_name(Some("pointer"));
    info.set_tooltip_text(Some("Open the player"));
    bar.append(&info);

    // ----- Transport + seek -----
    let center = widgets::vbox(2);
    center.set_hexpand(true);
    center.set_size_request(260, -1);
    center.set_valign(gtk::Align::Center);
    let controls = widgets::hbox(6);
    controls.set_halign(gtk::Align::Center);
    let prev = widgets::icon_button("media-skip-backward-symbolic", "Previous (Ctrl+←)");
    let play = gtk::Button::from_icon_name("media-playback-start-symbolic");
    play.add_css_class("play-button");
    play.set_tooltip_text(Some("Play (Space)"));
    let next = widgets::icon_button("media-skip-forward-symbolic", "Next (Ctrl+→)");
    for w in [prev.upcast_ref::<gtk::Widget>(), play.upcast_ref(), next.upcast_ref()] {
        w.set_focus_on_click(false);
        controls.append(w);
    }
    center.append(&controls);
    center.append(&SeekBar::new().root);
    bar.append(&center);

    // ----- Volume -----
    let right = widgets::hbox(8);
    right.set_valign(gtk::Align::Center);
    right.set_halign(gtk::Align::End);
    let p = prefs::get();
    let mute = widgets::icon_button(volume_icon(p.volume, p.muted), "Mute");
    mute.set_focus_on_click(false);
    right.append(&mute);
    let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, player::max_volume(), 0.01);
    volume.set_draw_value(false);
    volume.set_value(p.volume);
    volume.set_size_request(100, -1);
    volume.add_css_class("volume");
    volume.set_focus_on_click(false);
    volume.set_tooltip_text(Some(&format!("{}%", (p.volume * 100.0).round())));
    right.append(&volume);
    // Narrow windows: one button that opens the volume.
    let compact = gtk::MenuButton::new();
    compact.set_icon_name(volume_icon(p.volume, p.muted));
    compact.set_tooltip_text(Some("Volume"));
    compact.add_css_class("icon-button");
    compact.set_visible(false);
    let pop = gtk::Popover::new();
    let col = widgets::vbox(8);
    col.add_css_class("volume-popover");
    let vertical = gtk::Scale::with_range(gtk::Orientation::Vertical, 0.0, player::max_volume(), 0.01);
    vertical.set_inverted(true);
    vertical.set_draw_value(false);
    vertical.set_value(p.volume);
    vertical.set_size_request(-1, 140);
    vertical.add_css_class("volume");
    vertical.set_halign(gtk::Align::Center);
    vertical.connect_change_value(|_, _, v| {
        player::set_volume(v);
        if prefs::get().muted && v > 0.0 {
            player::set_muted(false);
        }
        glib::Propagation::Proceed
    });
    let pop_mute = widgets::icon_button(volume_icon(p.volume, p.muted), "Mute");
    pop_mute.connect_clicked(|_| player::set_muted(!prefs::get().muted));
    col.append(&vertical);
    col.append(&pop_mute);
    pop.set_child(Some(&col));
    compact.set_popover(Some(&pop));
    right.append(&compact);
    bar.append(&right);
    NARROW_PARTS.with(|n| n.borrow_mut().extend([volume.clone().upcast(), mute.clone().upcast()]));
    NARROW_ONLY.with(|n| n.borrow_mut().push(compact.clone().upcast()));

    // ----- Wiring -----
    play.connect_clicked(|_| player::toggle());
    prev.connect_clicked(|_| player::previous());
    next.connect_clicked(|_| player::next());
    close.connect_clicked(|_| player::close());
    mute.connect_clicked(|_| player::set_muted(!prefs::get().muted));
    volume.connect_change_value(|s, _, v| {
        let v = v.clamp(0.0, player::max_volume());
        player::set_volume(v);
        if prefs::get().muted && v > 0.0 {
            player::set_muted(false);
        }
        s.set_tooltip_text(Some(&format!("{}%", (v * 100.0).round())));
        glib::Propagation::Proceed
    });
    let scroll = gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
    scroll.connect_scroll(|_, _, dy| {
        player::set_volume(prefs::get().volume - dy * 0.05);
        glib::Propagation::Stop
    });
    mute.add_controller(scroll);

    let refresh = {
        let (picture, art, title, sub, play, mute, volume, prev, next, close) = (
            picture.clone(),
            art.clone(),
            title.clone(),
            sub.clone(),
            play.clone(),
            mute.clone(),
            volume.clone(),
            prev.clone(),
            next.clone(),
            close.clone(),
        );
        let (compact, pop_mute, vertical) = (compact.clone(), pop_mute.clone(), vertical.clone());
        move |e: Event| match e {
            Event::Track | Event::Video => {
                match player::current() {
                    Some(v) => {
                        let (t, s) = match v.kind {
                            Kind::Episode => (v.show.clone(), v.episode_line()),
                            _ => (v.title.clone(), v.year.map(|y| y.to_string()).unwrap_or_default()),
                        };
                        title.set_text(&t);
                        sub.set_text(&s);
                        title.set_tooltip_text(Some(&v.label()));
                        art.set_key(v.wide_art());
                    }
                    None => {
                        title.set_text("Nothing playing");
                        sub.set_text("");
                        art.set_key("");
                    }
                }
                let live = player::has_video() || video::paintable().has_frame();
                picture.set_visible(live);
                close.set_sensitive(player::current().is_some());
                prev.set_sensitive(player::can_previous());
                next.set_sensitive(player::can_next());
            }
            Event::State => {
                let playing = player::state() == State::Playing;
                play.set_icon_name(if playing { "media-playback-pause-symbolic" } else { "media-playback-start-symbolic" });
                play.set_tooltip_text(Some(if playing { "Pause (Space)" } else { "Play (Space)" }));
            }
            Event::Queue => {
                prev.set_sensitive(player::can_previous());
                next.set_sensitive(player::can_next());
            }
            Event::Options => {
                let p = prefs::get();
                mute.set_icon_name(volume_icon(p.volume, p.muted));
                compact.set_icon_name(volume_icon(p.volume, p.muted));
                pop_mute.set_icon_name(volume_icon(p.volume, p.muted));
                if (vertical.adjustment().upper() - player::max_volume()).abs() > 0.001 {
                    vertical.set_range(0.0, player::max_volume());
                }
                if (vertical.value() - p.volume).abs() > 0.005 {
                    vertical.set_value(p.volume);
                }
                mute.set_tooltip_text(Some(if p.muted { "Unmute" } else { "Mute" }));
                if (volume.adjustment().upper() - player::max_volume()).abs() > 0.001 {
                    volume.set_range(0.0, player::max_volume());
                }
                if (volume.value() - p.volume).abs() > 0.005 {
                    volume.set_value(p.volume);
                }
            }
            _ => {}
        }
    };
    for e in [Event::Track, Event::State, Event::Queue, Event::Options] {
        refresh(e);
    }
    player::subscribe(&bar, refresh);
    video::paintable().connect_invalidate_contents(move |p| {
        if p.has_frame() && !picture.is_visible() {
            picture.set_visible(true);
        }
    });
    bar
}
