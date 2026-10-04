//! Keep the screen awake while a video plays. mpv draws through its render API
//! and owns no window, so its own `stop-screensaver` does nothing; instead the
//! window holds an idle inhibitor (Wayland's idle-inhibit, through GTK), which
//! the idle daemon respects while the window is on screen.

use crate::player::{self, Event, State};
use crate::window;
use gtk::prelude::*;
use std::cell::Cell;

thread_local! {
    static COOKIE: Cell<Option<u32>> = const { Cell::new(None) };
}

fn update() {
    let want = player::state() == State::Playing && player::has_video();
    let Some(win) = window::window() else { return };
    let Some(app) = win.application() else { return };
    match (want, COOKIE.with(|c| c.get())) {
        (true, None) => {
            let cookie = app.inhibit(Some(&win), gtk::ApplicationInhibitFlags::IDLE, Some("Playing a video"));
            COOKIE.with(|c| c.set((cookie != 0).then_some(cookie)));
        }
        (false, Some(cookie)) => {
            app.uninhibit(cookie);
            COOKIE.with(|c| c.set(None));
        }
        _ => {}
    }
}

pub fn start() {
    player::subscribe_global(|e| {
        if matches!(e, Event::State | Event::Track | Event::Video) {
            update();
        }
    });
}
