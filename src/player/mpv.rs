//! A small, safe-ish wrapper over libmpv's client API (`libmpv.so.2`).
//! Only what the player needs: options, commands, properties, observed
//! properties and events. Events are drained on the UI thread; mpv's wakeup
//! callback (any thread) just pokes a channel.

use serde_json::{Map, Value};
use std::ffi::{CStr, CString, c_char, c_double, c_int, c_void};

#[repr(C)]
pub struct Handle {
    _private: [u8; 0],
}

pub const FORMAT_STRING: c_int = 1;
pub const FORMAT_FLAG: c_int = 3;
pub const FORMAT_INT64: c_int = 4;
pub const FORMAT_DOUBLE: c_int = 5;
pub const FORMAT_NODE: c_int = 6;
const FORMAT_NODE_ARRAY: c_int = 7;
const FORMAT_NODE_MAP: c_int = 8;

const EVENT_SHUTDOWN: c_int = 1;
const EVENT_START_FILE: c_int = 6;
const EVENT_END_FILE: c_int = 7;
const EVENT_FILE_LOADED: c_int = 8;
const EVENT_VIDEO_RECONFIG: c_int = 17;
const EVENT_SEEK: c_int = 20;
const EVENT_PLAYBACK_RESTART: c_int = 21;
const EVENT_PROPERTY_CHANGE: c_int = 22;

#[repr(C)]
union NodeValue {
    string: *mut c_char,
    flag: c_int,
    int64: i64,
    double: c_double,
    list: *mut NodeList,
    ba: *mut c_void,
}

#[repr(C)]
struct Node {
    u: NodeValue,
    format: c_int,
}

#[repr(C)]
struct NodeList {
    num: c_int,
    values: *mut Node,
    keys: *mut *mut c_char,
}

#[repr(C)]
struct RawEvent {
    event_id: c_int,
    error: c_int,
    reply_userdata: u64,
    data: *mut c_void,
}

#[repr(C)]
struct RawEventProperty {
    name: *const c_char,
    format: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct RawEventEndFile {
    reason: c_int,
    error: c_int,
    playlist_entry_id: i64,
    playlist_insert_id: i64,
    playlist_insert_num_entries: c_int,
}

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_create() -> *mut Handle;
    fn mpv_initialize(ctx: *mut Handle) -> c_int;
    fn mpv_terminate_destroy(ctx: *mut Handle);
    fn mpv_set_option_string(ctx: *mut Handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_command(ctx: *mut Handle, args: *mut *const c_char) -> c_int;
    fn mpv_set_property(ctx: *mut Handle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_set_property_string(ctx: *mut Handle, name: *const c_char, data: *const c_char) -> c_int;
    fn mpv_get_property(ctx: *mut Handle, name: *const c_char, format: c_int, data: *mut c_void) -> c_int;
    fn mpv_free(data: *mut c_void);
    fn mpv_observe_property(ctx: *mut Handle, reply_userdata: u64, name: *const c_char, format: c_int) -> c_int;
    fn mpv_wait_event(ctx: *mut Handle, timeout: c_double) -> *mut RawEvent;
    fn mpv_set_wakeup_callback(ctx: *mut Handle, cb: Option<unsafe extern "C" fn(*mut c_void)>, d: *mut c_void);
    fn mpv_error_string(error: c_int) -> *const c_char;
}

unsafe extern "C" {
    fn setlocale(category: c_int, locale: *const c_char) -> *mut c_char;
}

/// glibc's LC_NUMERIC.
const LC_NUMERIC: c_int = 1;

/// A property value as observed.
#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    None,
    Flag(bool),
    Int(i64),
    Double(f64),
    Str(String),
    Node(Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    Eof,
    Stop,
    Quit,
    Error,
    Redirect,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    StartFile,
    /// A file finished; `error` is mpv's message when it failed.
    EndFile {
        reason: EndReason,
        error: Option<String>,
    },
    FileLoaded,
    VideoReconfig,
    Seek,
    PlaybackRestart,
    Property {
        name: String,
        data: Data,
    },
    Shutdown,
}

pub fn error_string(code: c_int) -> String {
    // SAFETY: mpv returns a static string for every code.
    unsafe { CStr::from_ptr(mpv_error_string(code)).to_string_lossy().into_owned() }
}

fn check(code: c_int) -> Result<(), String> {
    if code < 0 { Err(error_string(code)) } else { Ok(()) }
}

fn cstr(s: &str) -> CString {
    CString::new(s.replace('\0', "")).unwrap_or_default()
}

/// Read an mpv node into JSON (nodes are how lists like `track-list` arrive).
unsafe fn node_to_json(n: &Node) -> Value {
    unsafe {
        match n.format {
            FORMAT_STRING => {
                if n.u.string.is_null() {
                    Value::Null
                } else {
                    Value::String(CStr::from_ptr(n.u.string).to_string_lossy().into_owned())
                }
            }
            FORMAT_FLAG => Value::Bool(n.u.flag != 0),
            FORMAT_INT64 => Value::from(n.u.int64),
            FORMAT_DOUBLE => serde_json::Number::from_f64(n.u.double).map(Value::Number).unwrap_or(Value::Null),
            FORMAT_NODE_ARRAY | FORMAT_NODE_MAP => {
                let list = n.u.list;
                if list.is_null() {
                    return Value::Null;
                }
                let l = &*list;
                // An empty list may come with null arrays.
                if l.num <= 0 || l.values.is_null() || (n.format == FORMAT_NODE_MAP && l.keys.is_null()) {
                    return if n.format == FORMAT_NODE_ARRAY { Value::Array(Vec::new()) } else { Value::Object(Map::new()) };
                }
                let values = std::slice::from_raw_parts(l.values, l.num as usize);
                if n.format == FORMAT_NODE_ARRAY {
                    Value::Array(values.iter().map(|v| node_to_json(v)).collect())
                } else {
                    let keys = std::slice::from_raw_parts(l.keys, l.num as usize);
                    let mut m = Map::new();
                    for (k, v) in keys.iter().zip(values) {
                        m.insert(CStr::from_ptr(*k).to_string_lossy().into_owned(), node_to_json(v));
                    }
                    Value::Object(m)
                }
            }
            _ => Value::Null,
        }
    }
}

pub struct Mpv {
    ctx: *mut Handle,
    /// Keeps the wakeup sender alive for as long as mpv may call it.
    _wakeup: Box<async_channel::Sender<()>>,
}

unsafe extern "C" fn on_wakeup(d: *mut c_void) {
    // SAFETY: `d` is the boxed sender owned by `Mpv`, which outlives the handle.
    let tx = unsafe { &*(d as *const async_channel::Sender<()>) };
    let _ = tx.try_send(());
}

impl Mpv {
    /// Create and initialize a player. `options` are set before init.
    pub fn new(options: &[(&str, &str)]) -> Result<(Mpv, async_channel::Receiver<()>), String> {
        // mpv parses numbers with the C library and refuses to start under a
        // locale that writes 1,5 for 1.5. GTK set the user's locale; numbers
        // in this app are formatted by Rust, so this changes nothing visible.
        // SAFETY: called on the main thread before mpv or any other thread
        // that reads the locale starts.
        unsafe { setlocale(LC_NUMERIC, c"C".as_ptr()) };
        // SAFETY: plain constructor; null on failure.
        let ctx = unsafe { mpv_create() };
        if ctx.is_null() {
            return Err("libmpv couldn't be started".into());
        }
        for (k, v) in options {
            let (k, v) = (cstr(k), cstr(v));
            // SAFETY: valid handle and C strings.
            let r = unsafe { mpv_set_option_string(ctx, k.as_ptr(), v.as_ptr()) };
            if r < 0 {
                eprintln!("media-player: mpv option {}: {}", k.to_string_lossy(), error_string(r));
            }
        }
        // SAFETY: valid handle.
        if let Err(e) = check(unsafe { mpv_initialize(ctx) }) {
            unsafe { mpv_terminate_destroy(ctx) };
            return Err(e);
        }
        // One pending wakeup is enough: the loop drains every queued event.
        let (tx, rx) = async_channel::bounded::<()>(1);
        let wakeup = Box::new(tx);
        // SAFETY: the sender lives in `self` until after terminate_destroy.
        unsafe { mpv_set_wakeup_callback(ctx, Some(on_wakeup), &*wakeup as *const _ as *mut c_void) };
        Ok((Mpv { ctx, _wakeup: wakeup }, rx))
    }

    pub fn raw(&self) -> *mut Handle {
        self.ctx
    }

    pub fn command(&self, args: &[&str]) -> Result<(), String> {
        let owned: Vec<CString> = args.iter().map(|a| cstr(a)).collect();
        let mut ptrs: Vec<*const c_char> = owned.iter().map(|c| c.as_ptr()).collect();
        ptrs.push(std::ptr::null());
        // SAFETY: null-terminated array of valid C strings.
        check(unsafe { mpv_command(self.ctx, ptrs.as_mut_ptr()) })
    }

    pub fn set_str(&self, name: &str, value: &str) -> Result<(), String> {
        let (n, v) = (cstr(name), cstr(value));
        // SAFETY: valid C strings.
        check(unsafe { mpv_set_property_string(self.ctx, n.as_ptr(), v.as_ptr()) })
    }

    pub fn set_f64(&self, name: &str, mut value: f64) -> Result<(), String> {
        let n = cstr(name);
        // SAFETY: FORMAT_DOUBLE takes a double*.
        check(unsafe { mpv_set_property(self.ctx, n.as_ptr(), FORMAT_DOUBLE, &mut value as *mut f64 as *mut c_void) })
    }

    pub fn set_i64(&self, name: &str, mut value: i64) -> Result<(), String> {
        let n = cstr(name);
        // SAFETY: FORMAT_INT64 takes an int64_t*.
        check(unsafe { mpv_set_property(self.ctx, n.as_ptr(), FORMAT_INT64, &mut value as *mut i64 as *mut c_void) })
    }

    pub fn set_flag(&self, name: &str, value: bool) -> Result<(), String> {
        let n = cstr(name);
        let mut v: c_int = value.into();
        // SAFETY: FORMAT_FLAG takes an int*.
        check(unsafe { mpv_set_property(self.ctx, n.as_ptr(), FORMAT_FLAG, &mut v as *mut c_int as *mut c_void) })
    }

    pub fn get_f64(&self, name: &str) -> Option<f64> {
        let n = cstr(name);
        let mut v: f64 = 0.0;
        // SAFETY: FORMAT_DOUBLE writes a double.
        let r = unsafe { mpv_get_property(self.ctx, n.as_ptr(), FORMAT_DOUBLE, &mut v as *mut f64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn get_i64(&self, name: &str) -> Option<i64> {
        let n = cstr(name);
        let mut v: i64 = 0;
        // SAFETY: FORMAT_INT64 writes an int64_t.
        let r = unsafe { mpv_get_property(self.ctx, n.as_ptr(), FORMAT_INT64, &mut v as *mut i64 as *mut c_void) };
        (r >= 0).then_some(v)
    }

    pub fn get_str(&self, name: &str) -> Option<String> {
        let n = cstr(name);
        let mut p: *mut c_char = std::ptr::null_mut();
        // SAFETY: FORMAT_STRING writes a char* we must mpv_free.
        let r = unsafe { mpv_get_property(self.ctx, n.as_ptr(), FORMAT_STRING, &mut p as *mut *mut c_char as *mut c_void) };
        if r < 0 || p.is_null() {
            return None;
        }
        let s = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
        unsafe { mpv_free(p as *mut c_void) };
        Some(s)
    }

    pub fn observe(&self, name: &str, format: c_int) {
        let n = cstr(name);
        // SAFETY: valid handle and name.
        unsafe { mpv_observe_property(self.ctx, 0, n.as_ptr(), format) };
    }

    /// The next queued event, without waiting.
    pub fn next_event(&self) -> Option<Event> {
        loop {
            // SAFETY: the returned event is valid until the next wait_event call;
            // everything is copied out before returning.
            let ev = unsafe { &*mpv_wait_event(self.ctx, 0.0) };
            let e = match ev.event_id {
                0 => return None,
                EVENT_SHUTDOWN => Event::Shutdown,
                EVENT_START_FILE => Event::StartFile,
                EVENT_FILE_LOADED => Event::FileLoaded,
                EVENT_VIDEO_RECONFIG => Event::VideoReconfig,
                EVENT_SEEK => Event::Seek,
                EVENT_PLAYBACK_RESTART => Event::PlaybackRestart,
                EVENT_END_FILE => {
                    let ef = unsafe { &*(ev.data as *const RawEventEndFile) };
                    let reason = match ef.reason {
                        0 => EndReason::Eof,
                        2 => EndReason::Stop,
                        3 => EndReason::Quit,
                        4 => EndReason::Error,
                        _ => EndReason::Redirect,
                    };
                    let error = (reason == EndReason::Error).then(|| error_string(ef.error));
                    Event::EndFile { reason, error }
                }
                EVENT_PROPERTY_CHANGE => {
                    let p = unsafe { &*(ev.data as *const RawEventProperty) };
                    let name = unsafe { CStr::from_ptr(p.name).to_string_lossy().into_owned() };
                    let data = unsafe {
                        if p.data.is_null() {
                            Data::None
                        } else {
                            match p.format {
                                FORMAT_FLAG => Data::Flag(*(p.data as *const c_int) != 0),
                                FORMAT_INT64 => Data::Int(*(p.data as *const i64)),
                                FORMAT_DOUBLE => Data::Double(*(p.data as *const f64)),
                                FORMAT_STRING => {
                                    let s = *(p.data as *const *const c_char);
                                    Data::Str(if s.is_null() {
                                        String::new()
                                    } else {
                                        CStr::from_ptr(s).to_string_lossy().into_owned()
                                    })
                                }
                                FORMAT_NODE => Data::Node(node_to_json(&*(p.data as *const Node))),
                                _ => Data::None,
                            }
                        }
                    };
                    Event::Property { name, data }
                }
                // Log messages, replies, hooks and the rest: not used.
                _ => continue,
            };
            return Some(e);
        }
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        // SAFETY: the render context (if any) is freed before this, see `video`.
        unsafe {
            mpv_set_wakeup_callback(self.ctx, None, std::ptr::null_mut());
            mpv_terminate_destroy(self.ctx);
        }
    }
}
