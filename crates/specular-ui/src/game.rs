//! The running game, as the UI shows it: its session (`specular_runtime`), the buttons held for
//! it, and the texture its frames are drawn into. Frames are uploaded on the UI thread just before
//! Slint draws (the only time its GL context is sure to be current), into one texture that the
//! window shows as a borrowed GL texture.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use glow::HasContext;
use specular_core::device::{Device, Output, SoundOut};
use specular_core::app::Msg;
use specular_core::effects::Event;
use specular_core::input::{Button, ButtonEvent};
use specular_core::launch::{self, LaunchRecord};
use specular_core::library::{CoreDecl, Game, user_cores};
use specular_runtime::{FrameSlot, LaunchSpec, Outputs, Session};

use crate::{AppWindow, Runner};

/// Where Specular's own cores are installed: `$SPECULAR_CORES`, else `cores/` next to Specular's
/// binary.
fn cores_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("SPECULAR_CORES") { return d.into(); }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().unwrap_or(Path::new(".")).join("cores")
}

/// A game being played.
pub struct Playing {
    pub session: Session,
    pub frames: Arc<FrameSlot>,
    /// The file of the save state the menu saves and loads (one slot for now).
    pub state: PathBuf,
    /// Where its launch record is kept, and the record (`LaunchSpec::record`): `watch_game`
    /// rewrites it with the reason before ending Specular.
    pub record: (PathBuf, LaunchRecord),
    /// The output its sound was last sent to; if that couldn't be opened, the game is silent.
    pub output: RefCell<Output>,
    /// Whether `output` couldn't be opened at launch, so the game started silent.
    pub sound_failed: bool,
    /// The buttons held, as libretro joypad bits.
    held: Cell<u16>,
}

/// A device the UI can use from other threads too (to open sound outputs, which can block).
pub type SharedDevice = Arc<dyn Device + Send + Sync>;

impl Playing {
    /// Starts `game` with `core` (`App::playing_core`), given these `options` (`App::core_options`),
    /// with `r`'s data folders, device, sound output and rumble, on another thread, since reading a
    /// large ROM or opening a Bluetooth output can take a second or more; then calls `started`
    /// there with the result. The caller checks the BIOS first (`CoreDecl::check_bios`). The UI is told of each new frame (`Runner::game_frame`) and each in-game
    /// save, and the app if the core's built-in BIOS halts the game (`Msg::GameStopped`). Before opening the game's output, that thread waits for the interface sounds to close
    /// theirs. A core the user added has the options it declares learned, once the game has
    /// started (`user_cores::learn`).
    pub fn start(r: &Runner, game: &Game, core: &'static CoreDecl, options: Vec<(String, String)>, started: impl FnOnce(Result<Self, String>) + Send + 'static) {
        let (data, output, rumble) = (&r.data, r.output.borrow().clone(), r.rumble.clone());
        let library = core.user.map_or_else(|| cores_dir().join(format!("{}.so", core.library)), |u| u.path.clone());
        let record = (launch::path(&data.cache()), LaunchRecord::new(core.name, &library, game));
        let spec = LaunchSpec {
            core: library,
            rom: data.roms().join(&game.path),
            save: game.save_path(&data.saves(), "srm"),
            autosave: Some(game.state_path(&data.roms(), "autosave")),
            system_dir: data.bios(),
            options,
            // Only to log how a core differs from its list: a user core's list is what it declared
            // itself, and a change is learned again (`user_cores::learn`) rather than logged.
            option_list: core.user.is_none().then(|| specular_core::settings::core_options::list(core)),
            rom_by_path: core.rom_by_path,
            record: Some(record.clone()),
            states_restore_save: core.states_restore_save,
        };
        let frames = Arc::new(FrameSlot::default());
        // One message to the UI at a time: frames arriving while one is pending are drawn by it.
        let pending = Arc::new(AtomicBool::new(false));
        let on_frame = Box::new(move || {
            if pending.swap(true, Relaxed) { return; }
            let pending = pending.clone();
            crate::on_ui(move |r| { pending.store(false, Relaxed); r.game_frame(); });
        });
        let on_save = Box::new(|| crate::on_ui(|r| r.tell_effects(Event::GameSaved)));
        // The app is told to quit the game, unless another ROM is playing by the time the UI
        // thread has this (the same one quit and started again would be quit too).
        let rom = game.path.clone();
        let on_bios_halt = Box::new(move || {
            let rom = rom.clone();
            crate::on_ui(move |r| if r.playing.borrow().as_ref().is_some_and(|p| p.record.1.game == rom) {
                let why = core.bios_halt_notice();
                eprintln!("game: stopped: {why}");
                crate::post(Msg::GameStopped(why));
            });
        });
        let (vsync, display_hz) = (r.device.open_vsync(), r.profile.refresh_hz);
        let state = game.state_path(&data.roms(), "state1");
        let (roms, saves, learned, played, device, sounds) = (data.roms(), data.saves(), data.learned_options(), game.clone(), r.device.clone(), r.sounds.clone());
        std::thread::spawn(move || {
            sounds.close();
            let sound = open_sound(device.as_ref(), &output);
            let sound_failed = sound.is_none();
            let out = Outputs { sound, vsync, display_hz, frames: frames.clone(), on_frame, rumble, on_save, on_bios_halt };
            // So its saves can follow it if it is renamed (see `library::saves`).
            if let Err(e) = specular_core::library::saves::note_played(&roms, &saves, &played) { eprintln!("saves: {e}"); }
            if let Err(e) = specular_core::library::saves::move_old_state(&roms, &saves, &played) { eprintln!("saves: {e}"); }
            let session = Session::start(spec, out);
            let declared = session.as_ref().ok().map(|s| s.info().options.clone());
            started(session.map(|session| Self { session, frames, state, record, output: RefCell::new(output), sound_failed, held: Cell::new(0) }));
            let Some(d) = declared else { return };
            match user_cores::learn(core, &learned, &d) {
                Ok(true) => eprintln!("cores: learned {}'s options ({}), kept in {}", core.name, d.options.len(), learned.display()),
                Err(e) => eprintln!("cores: can't keep {}'s options in {}: {e}", core.name, learned.display()),
                Ok(false) => {}
            }
        });
    }

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
        self.session.pause(paused);
    }
}

/// Opens `output` for a game's sound; None (having said why) if it can't be. May block: a
/// Bluetooth output can take a second.
pub fn open_sound(device: &dyn Device, output: &Output) -> Option<Box<dyn SoundOut>> {
    device.open_sound(output).map_err(|e| eprintln!("sound: opening {output:?}: {e}")).ok()
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
    /// (making a new one, and showing it, when the size changes); returns whether a frame was
    /// uploaded. With no
    /// game, frees the texture (here, where the GL context is current) and stops showing it.
    pub fn before_render(&mut self, api: &slint::GraphicsAPI, frames: Option<&FrameSlot>, window: &AppWindow) -> bool {
        let slint::GraphicsAPI::NativeOpenGL { get_proc_address } = api else { return false };
        let gl = self.gl.get_or_insert_with(|| unsafe { glow::Context::from_loader_function_cstr(|n| get_proc_address(n)) });
        let Some(frames) = frames else {
            if let Some((t, _, _)) = self.texture.take() {
                window.set_game_frame(slint::Image::default());
                unsafe { gl.delete_texture(t) };
            }
            return false;
        };
        let Some(frame) = frames.take() else { return false };
        unsafe {
            if self.texture.is_none_or(|(_, w, h)| (w, h) != (frame.width, frame.height)) {
                if let Some((t, _, _)) = self.texture.take() { gl.delete_texture(t); }
                let Ok(t) = gl.create_texture() else { frames.give_back(frame); return false };
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
        true
    }
}
