//! The running game, as the UI shows it: its session (`specular_runtime`), the buttons held for
//! it, and the texture its frames are drawn into. Frames are uploaded on the UI thread just before
//! Slint draws (the only time its GL context is sure to be current), into one texture that the
//! window shows as a borrowed GL texture.

use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use glow::HasContext;
use slint::ComponentHandle;
use specular_core::device::Device;
use specular_core::input::{Button, ButtonEvent};
use specular_core::library::{Game, SystemDecl};
use specular_runtime::{FrameSlot, LaunchSpec, Outputs, Session};

use crate::{AppWindow, DataRoot};

/// Where cores are installed: `$SPECULAR_CORES`, else `cores/` next to Specular's binary.
fn cores_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SPECULAR_CORES") { return d.into(); }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().unwrap_or(Path::new(".")).join("cores")
}

/// A game being played.
pub struct Playing {
    pub session: Session,
    pub frames: Arc<FrameSlot>,
    /// The save state's file (one slot for now).
    pub state: PathBuf,
    /// The buttons held, as libretro joypad bits.
    held: Cell<u16>,
    paused: Cell<bool>,
}

impl Playing {
    /// Starts `game` with its system's core, loading it on another thread (reading a large ROM
    /// takes a while), then calls `started` there with the result. `window` is redrawn whenever
    /// a new frame arrives.
    pub fn start(game: &Game, system: &SystemDecl, data: &DataRoot, device: &dyn Device, window: slint::Weak<AppWindow>,
                 started: impl FnOnce(Result<Self, String>) + Send + 'static) {
        let Some(core) = system.core else { return started(Err(format!("No emulator for {} yet", system.name))) };
        let spec = LaunchSpec {
            core: cores_dir().join(format!("{core}.so")),
            rom: data.roms().join(&game.path),
            save: game.save_path(&data.saves(), "srm"),
            system_dir: data.bios(),
            options: system.core_options.iter().map(|&(k, v)| (k.to_string(), v.to_string())).collect(),
        };
        let sound = device.open_sound().map_err(|e| eprintln!("sound: {e}; playing without")).ok();
        let frames = Arc::new(FrameSlot::default());
        // One redraw request at a time: frames arriving while one is pending are drawn by it.
        let pending = Arc::new(AtomicBool::new(false));
        let on_frame = Box::new(move || {
            if pending.swap(true, Relaxed) { return; }
            let (window, pending) = (window.clone(), pending.clone());
            let _ = slint::invoke_from_event_loop(move || {
                pending.store(false, Relaxed);
                if let Some(w) = window.upgrade() { w.window().request_redraw(); }
            });
        });
        let out = Outputs { sound, vsync: device.open_vsync(), display_hz: device.profile().refresh_hz, frames: frames.clone(), on_frame };
        let state = game.save_path(&data.saves(), "state1");
        std::thread::spawn(move || started(Session::start(spec, out).map(|session| Self { session, frames, state, held: Cell::new(0), paused: Cell::new(false) })));
    }

    /// Whether any of the game's buttons is held down.
    pub fn holding(&self) -> bool { self.held.get() != 0 }

    /// A button for the game.
    pub fn button(&self, e: &ButtonEvent) {
        let Some(bit) = specular_runtime::joypad_bit(e.button) else { return };
        let held = if e.pressed { self.held.get() | bit } else { self.held.get() & !bit };
        self.held.set(held);
        self.session.set_buttons(held);
    }

    /// Pauses or resumes the game. Pausing releases every button, so none stays held in the game.
    pub fn pause(&self, paused: bool) {
        if paused { self.held.set(0); self.session.set_buttons(0); }
        self.paused.set(paused);
        self.session.pause(paused);
    }

    pub fn is_paused(&self) -> bool { self.paused.get() }
}

/// Whether `b` goes to the game while it is played (the app keeps Menu, Power and volume).
pub fn for_game(b: Button) -> bool { specular_runtime::joypad_bit(b).is_some() }

/// The GL side: the texture frames go into. Lives in the window's rendering notifier.
#[derive(Default)]
pub struct Screen {
    gl: Option<glow::Context>,
    texture: Option<(glow::Texture, u32, u32)>,
}

impl Screen {
    /// Before Slint draws: uploads the newest of the game's `frames`, if any, into the texture
    /// (making a new one, and showing it, when the size changes). With no game, frees the
    /// texture (here, where the GL context is current) and stops showing it.
    pub fn before_render(&mut self, api: &slint::GraphicsAPI, frames: Option<&FrameSlot>, window: &AppWindow) {
        let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api else { return };
        let gl = self.gl.get_or_insert_with(|| unsafe { glow::Context::from_loader_function_cstr(|n| get_proc_address(n)) });
        let Some(frames) = frames else {
            if let Some((t, _, _)) = self.texture.take() {
                window.set_game_frame(slint::Image::default());
                unsafe { gl.delete_texture(t) };
            }
            return;
        };
        let Some(frame) = frames.take() else { return };
        unsafe {
            if self.texture.is_none_or(|(_, w, h)| (w, h) != (frame.width, frame.height)) {
                if let Some((t, _, _)) = self.texture.take() { gl.delete_texture(t); }
                let Ok(t) = gl.create_texture() else { frames.give_back(frame); return };
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA as i32, frame.width as i32, frame.height as i32, 0,
                    glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(None));
                // Sharp pixels when scaled up.
                for p in [glow::TEXTURE_MIN_FILTER, glow::TEXTURE_MAG_FILTER] { gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::NEAREST as i32); }
                for p in [glow::TEXTURE_WRAP_S, glow::TEXTURE_WRAP_T] { gl.tex_parameter_i32(glow::TEXTURE_2D, p, glow::CLAMP_TO_EDGE as i32); }
                self.texture = Some((t, frame.width, frame.height));
                let image = slint::BorrowedOpenGLTextureBuilder::new_gl_2d_rgba_texture(t.0, (frame.width, frame.height).into()).build();
                window.set_game_frame(image);
            }
            let (t, w, h) = self.texture.unwrap();
            gl.bind_texture(glow::TEXTURE_2D, Some(t));
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, w as i32, h as i32, glow::RGBA, glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&frame.rgba)));
            gl.bind_texture(glow::TEXTURE_2D, None);
        }
        frames.give_back(frame);
    }
}
