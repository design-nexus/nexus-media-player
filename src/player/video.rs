//! Video drawing. mpv's render API draws each frame into an OpenGL texture we
//! own (in a GDK GL context made from the window), and the texture is handed to
//! GTK as a `GdkGLTexture`. Every view of the video shares one [`paintable`], so
//! the player page, the player bar and fullscreen all show the same frames, and
//! frames keep being drawn while the player page isn't on screen.

use super::mpv;
use gtk::glib::subclass::prelude::*;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk::{gdk, glib};
use std::cell::RefCell;
use std::ffi::{CString, c_char, c_int, c_void};
use std::sync::{Arc, Mutex};

// ---------- libmpv render API ----------

#[repr(C)]
struct RenderContext {
    _private: [u8; 0],
}

#[repr(C)]
struct RenderParam {
    kind: c_int,
    data: *mut c_void,
}

#[repr(C)]
struct OpenGlInitParams {
    get_proc_address: unsafe extern "C" fn(*mut c_void, *const c_char) -> *mut c_void,
    get_proc_address_ctx: *mut c_void,
}

#[repr(C)]
struct OpenGlFbo {
    fbo: c_int,
    w: c_int,
    h: c_int,
    internal_format: c_int,
}

const PARAM_INVALID: c_int = 0;
const PARAM_API_TYPE: c_int = 1;
const PARAM_OPENGL_INIT_PARAMS: c_int = 2;
const PARAM_OPENGL_FBO: c_int = 3;
const PARAM_FLIP_Y: c_int = 4;
const PARAM_WL_DISPLAY: c_int = 9;
const PARAM_BLOCK_FOR_TARGET_TIME: c_int = 12;
const UPDATE_FRAME: u64 = 1;

#[link(name = "mpv")]
unsafe extern "C" {
    fn mpv_render_context_create(res: *mut *mut RenderContext, mpv: *mut mpv::Handle, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_set_update_callback(
        ctx: *mut RenderContext,
        cb: Option<unsafe extern "C" fn(*mut c_void)>,
        cb_ctx: *mut c_void,
    );
    fn mpv_render_context_update(ctx: *mut RenderContext) -> u64;
    fn mpv_render_context_render(ctx: *mut RenderContext, params: *mut RenderParam) -> c_int;
    fn mpv_render_context_report_swap(ctx: *mut RenderContext);
    fn mpv_render_context_free(ctx: *mut RenderContext);
}

#[link(name = "EGL")]
unsafe extern "C" {
    fn eglGetProcAddress(name: *const c_char) -> *mut c_void;
}

unsafe extern "C" {
    // From GTK itself (the Wayland backend); lets mpv share the display for VA-API.
    fn gdk_wayland_display_get_wl_display(display: *mut c_void) -> *mut c_void;
}

unsafe extern "C" fn get_proc_address(_ctx: *mut c_void, name: *const c_char) -> *mut c_void {
    // SAFETY: `name` is a valid C string from mpv.
    unsafe { eglGetProcAddress(name) }
}

// ---------- The few GL calls we make ----------

const GL_TEXTURE_2D: u32 = 0x0DE1;
const GL_RGBA8: i32 = 0x8058;
const GL_RGBA: u32 = 0x1908;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_TEXTURE_MIN_FILTER: u32 = 0x2801;
const GL_TEXTURE_MAG_FILTER: u32 = 0x2800;
const GL_TEXTURE_WRAP_S: u32 = 0x2802;
const GL_TEXTURE_WRAP_T: u32 = 0x2803;
const GL_LINEAR: i32 = 0x2601;
const GL_CLAMP_TO_EDGE: i32 = 0x812F;
const GL_FRAMEBUFFER: u32 = 0x8D40;
const GL_COLOR_ATTACHMENT0: u32 = 0x8CE0;
const GL_SYNC_GPU_COMMANDS_COMPLETE: u32 = 0x9117;

type GlSync = *const c_void;

struct Gl {
    gen_textures: unsafe extern "C" fn(i32, *mut u32),
    delete_textures: unsafe extern "C" fn(i32, *const u32),
    bind_texture: unsafe extern "C" fn(u32, u32),
    tex_image_2d: unsafe extern "C" fn(u32, i32, i32, i32, i32, i32, u32, u32, *const c_void),
    tex_parameteri: unsafe extern "C" fn(u32, u32, i32),
    gen_framebuffers: unsafe extern "C" fn(i32, *mut u32),
    delete_framebuffers: unsafe extern "C" fn(i32, *const u32),
    bind_framebuffer: unsafe extern "C" fn(u32, u32),
    framebuffer_texture_2d: unsafe extern "C" fn(u32, u32, u32, u32, i32),
    fence_sync: unsafe extern "C" fn(u32, u32) -> GlSync,
    delete_sync: unsafe extern "C" fn(GlSync),
    flush: unsafe extern "C" fn(),
}

impl Gl {
    fn load() -> Option<Gl> {
        fn f<T>(name: &str) -> Option<T> {
            let c = CString::new(name).ok()?;
            // SAFETY: looking up a GL entry point by name.
            let p = unsafe { eglGetProcAddress(c.as_ptr()) };
            if p.is_null() {
                return None;
            }
            // SAFETY: T is the matching fn pointer type for this entry point.
            Some(unsafe { std::mem::transmute_copy::<*mut c_void, T>(&p) })
        }
        Some(Gl {
            gen_textures: f("glGenTextures")?,
            delete_textures: f("glDeleteTextures")?,
            bind_texture: f("glBindTexture")?,
            tex_image_2d: f("glTexImage2D")?,
            tex_parameteri: f("glTexParameteri")?,
            gen_framebuffers: f("glGenFramebuffers")?,
            delete_framebuffers: f("glDeleteFramebuffers")?,
            bind_framebuffer: f("glBindFramebuffer")?,
            framebuffer_texture_2d: f("glFramebufferTexture2D")?,
            fence_sync: f("glFenceSync")?,
            delete_sync: f("glDeleteSync")?,
            flush: f("glFlush")?,
        })
    }
}

// ---------- The shared paintable ----------

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct VideoPaintable {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub size: std::cell::Cell<(i32, i32)>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for VideoPaintable {
        const NAME: &'static str = "NmpVideoPaintable";
        type Type = super::VideoPaintable;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for VideoPaintable {}

    impl PaintableImpl for VideoPaintable {
        fn snapshot(&self, snapshot: &gdk::Snapshot, width: f64, height: f64) {
            if let Some(t) = self.texture.borrow().as_ref() {
                let snapshot = snapshot.downcast_ref::<gtk::Snapshot>().expect("gtk snapshot");
                let rect = gtk::graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
                snapshot.append_scaled_texture(t, gtk::gsk::ScalingFilter::Linear, &rect);
            }
        }

        fn intrinsic_width(&self) -> i32 {
            self.size.get().0
        }

        fn intrinsic_height(&self) -> i32 {
            self.size.get().1
        }

        fn intrinsic_aspect_ratio(&self) -> f64 {
            let (w, h) = self.size.get();
            if w > 0 && h > 0 { w as f64 / h as f64 } else { 0.0 }
        }

        fn current_image(&self) -> gdk::Paintable {
            match self.texture.borrow().as_ref() {
                Some(t) => t.clone().upcast(),
                None => gdk::Paintable::new_empty(0, 0),
            }
        }
    }
}

glib::wrapper! {
    pub struct VideoPaintable(ObjectSubclass<imp::VideoPaintable>) @implements gdk::Paintable;
}

impl VideoPaintable {
    fn set(&self, texture: Option<gdk::Texture>, size: (i32, i32)) {
        let imp = self.imp();
        let resized = imp.size.get() != size;
        imp.size.set(size);
        *imp.texture.borrow_mut() = texture;
        if resized {
            self.invalidate_size();
        }
        self.invalidate_contents();
    }

    pub fn has_frame(&self) -> bool {
        self.imp().texture.borrow().is_some()
    }
}

// ---------- The renderer ----------

/// A texture and the framebuffer that draws into it.
#[derive(Clone, Copy)]
struct Target {
    tex: u32,
    fbo: u32,
    size: (i32, i32),
}

/// Textures GTK has finished with, and their fences (as integers, to be Send).
type Released = Arc<Mutex<Vec<(Target, usize)>>>;

struct Renderer {
    gl_context: gdk::GLContext,
    gl: Gl,
    ctx: *mut RenderContext,
    free: Vec<Target>,
    released: Released,
    /// Keeps the update sender alive while mpv may call it.
    _update: Box<async_channel::Sender<()>>,
}

thread_local! {
    static PAINTABLE: VideoPaintable = glib::Object::new();
    static RENDERER: RefCell<Option<Renderer>> = const { RefCell::new(None) };
}

/// What every video view draws.
pub fn paintable() -> VideoPaintable {
    PAINTABLE.with(|p| p.clone())
}

unsafe extern "C" fn on_update(d: *mut c_void) {
    // SAFETY: `d` is the boxed sender owned by the renderer.
    let tx = unsafe { &*(d as *const async_channel::Sender<()>) };
    let _ = tx.try_send(());
}

/// Set up drawing for `mpv` on the window's surface. Call once the window is
/// realized and before any file is loaded.
pub fn init(window: &gtk::ApplicationWindow, mpv: &mpv::Mpv, size: impl Fn() -> (i32, i32) + 'static) -> Result<(), String> {
    // A context of our own, not bound to the window's surface: it only ever
    // draws into our textures, which GTK's own context shares.
    let gl_context = gtk::prelude::WidgetExt::display(window).create_gl_context().map_err(|e| e.to_string())?;
    gl_context.realize().map_err(|e| e.to_string())?;
    gl_context.make_current();
    let gl = Gl::load().ok_or("OpenGL functions are missing")?;

    let api = CString::new("opengl").expect("static string");
    let mut init = OpenGlInitParams { get_proc_address, get_proc_address_ctx: std::ptr::null_mut() };
    let mut params = vec![
        RenderParam { kind: PARAM_API_TYPE, data: api.as_ptr() as *mut c_void },
        RenderParam { kind: PARAM_OPENGL_INIT_PARAMS, data: &mut init as *mut _ as *mut c_void },
    ];
    let display = gtk::prelude::WidgetExt::display(window);
    if display.type_().name() == "GdkWaylandDisplay" {
        use gtk::glib::translate::ToGlibPtr;
        let raw: *mut gdk::ffi::GdkDisplay = display.to_glib_none().0;
        // SAFETY: `display` is a GdkWaylandDisplay.
        let wl = unsafe { gdk_wayland_display_get_wl_display(raw as *mut c_void) };
        if !wl.is_null() {
            params.push(RenderParam { kind: PARAM_WL_DISPLAY, data: wl });
        }
    }
    params.push(RenderParam { kind: PARAM_INVALID, data: std::ptr::null_mut() });

    let mut ctx: *mut RenderContext = std::ptr::null_mut();
    // SAFETY: valid handle and params, with our GL context current.
    let r = unsafe { mpv_render_context_create(&mut ctx, mpv.raw(), params.as_mut_ptr()) };
    if r < 0 || ctx.is_null() {
        return Err(mpv::error_string(r));
    }

    let (tx, rx) = async_channel::bounded::<()>(1);
    let update = Box::new(tx);
    // SAFETY: the sender is kept in the renderer until the context is freed.
    unsafe { mpv_render_context_set_update_callback(ctx, Some(on_update), &*update as *const _ as *mut c_void) };
    RENDERER.with(|r| {
        *r.borrow_mut() = Some(Renderer { gl_context, gl, ctx, free: Vec::new(), released: Arc::default(), _update: update })
    });
    glib::spawn_future_local(async move {
        while rx.recv().await.is_ok() {
            render(size());
        }
    });
    Ok(())
}

pub fn ready() -> bool {
    RENDERER.with(|r| r.borrow().is_some())
}

fn render(size: (i32, i32)) {
    let texture = RENDERER.with(|r| {
        let mut r = r.borrow_mut();
        let r = r.as_mut()?;
        // SAFETY: valid render context.
        let flags = unsafe { mpv_render_context_update(r.ctx) };
        if flags & UPDATE_FRAME == 0 {
            return None;
        }
        let size = (size.0.clamp(16, 7680), size.1.clamp(16, 4320));
        r.gl_context.make_current();
        r.reclaim(size);
        let target = r.target(size);
        let mut fbo = OpenGlFbo { fbo: target.fbo as c_int, w: size.0, h: size.1, internal_format: 0 };
        let mut flip: c_int = 0;
        // mpv already asks for each frame when it's due. Left on, it would also
        // wait for that moment inside render, stalling the main loop for most
        // of every frame so GTK misses vsyncs and the video stutters.
        let mut block: c_int = 0;
        let mut params = [
            RenderParam { kind: PARAM_OPENGL_FBO, data: &mut fbo as *mut _ as *mut c_void },
            RenderParam { kind: PARAM_FLIP_Y, data: &mut flip as *mut _ as *mut c_void },
            RenderParam { kind: PARAM_BLOCK_FOR_TARGET_TIME, data: &mut block as *mut _ as *mut c_void },
            RenderParam { kind: PARAM_INVALID, data: std::ptr::null_mut() },
        ];
        // SAFETY: our context is current and the FBO is complete.
        unsafe { mpv_render_context_render(r.ctx, params.as_mut_ptr()) };
        let sync = unsafe { (r.gl.fence_sync)(GL_SYNC_GPU_COMMANDS_COMPLETE, 0) };
        unsafe { (r.gl.flush)() };
        let released = r.released.clone();
        let sync_id = sync as usize;
        // SAFETY: the texture stays alive until the release function hands it back.
        let texture = unsafe {
            gdk::GLTextureBuilder::new()
                .set_context(Some(&r.gl_context))
                .set_id(target.tex)
                .set_width(size.0)
                .set_height(size.1)
                .set_format(gdk::MemoryFormat::R8g8b8a8Premultiplied)
                .set_sync(Some(sync))
                .build_with_release_func(move || {
                    if let Ok(mut v) = released.lock() {
                        v.push((target, sync_id));
                    }
                })
        };
        // SAFETY: valid render context.
        unsafe { mpv_render_context_report_swap(r.ctx) };
        // Leave no context of ours current for GTK to trip over.
        gdk::GLContext::clear_current();
        Some((texture, size))
    });
    if let Some((t, size)) = texture {
        paintable().set(Some(t), size);
    }
}

impl Renderer {
    /// Take back textures GTK released; keep the ones of the current size.
    fn reclaim(&mut self, size: (i32, i32)) {
        let back: Vec<(Target, usize)> = self.released.lock().map(|mut v| std::mem::take(&mut *v)).unwrap_or_default();
        for (t, sync) in back {
            if sync != 0 {
                // SAFETY: a fence we created; our context is current.
                unsafe { (self.gl.delete_sync)(sync as GlSync) };
            }
            self.free.push(t);
        }
        let gl = &self.gl;
        self.free.retain(|t| {
            if t.size == size {
                return true;
            }
            // SAFETY: names we generated.
            unsafe {
                (gl.delete_framebuffers)(1, &t.fbo);
                (gl.delete_textures)(1, &t.tex);
            }
            false
        });
    }

    fn target(&mut self, size: (i32, i32)) -> Target {
        if let Some(t) = self.free.pop() {
            return t;
        }
        let gl = &self.gl;
        let (mut tex, mut fbo) = (0u32, 0u32);
        // SAFETY: plain GL object creation with our context current.
        unsafe {
            (gl.gen_textures)(1, &mut tex);
            (gl.bind_texture)(GL_TEXTURE_2D, tex);
            (gl.tex_image_2d)(GL_TEXTURE_2D, 0, GL_RGBA8, size.0, size.1, 0, GL_RGBA, GL_UNSIGNED_BYTE, std::ptr::null());
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
            (gl.tex_parameteri)(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
            (gl.bind_texture)(GL_TEXTURE_2D, 0);
            (gl.gen_framebuffers)(1, &mut fbo);
            (gl.bind_framebuffer)(GL_FRAMEBUFFER, fbo);
            (gl.framebuffer_texture_2d)(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            (gl.bind_framebuffer)(GL_FRAMEBUFFER, 0);
        }
        Target { tex, fbo, size }
    }
}

/// Drop the last frame (a new file, or stopped).
pub fn clear() {
    paintable().set(None, (0, 0));
}

/// Free the render context. Must run before mpv itself is destroyed.
pub fn shutdown() {
    clear();
    RENDERER.with(|r| {
        if let Some(r) = r.borrow_mut().take() {
            r.gl_context.make_current();
            // SAFETY: the context is ours and no render is in progress.
            unsafe {
                mpv_render_context_set_update_callback(r.ctx, None, std::ptr::null_mut());
                mpv_render_context_free(r.ctx);
            }
        }
    });
}
