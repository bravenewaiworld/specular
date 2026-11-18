//! The Brick's Slint platform: Specular's `GlPlatform` on the PowerVR driver's default EGL display.
//! TrimUI's closed driver has only PowerVR's "NULL window system": there is no windowing at all,
//! and a window surface created without a native window renders straight to the panel
//! (/dev/fb0, whose buffers the display driver swaps on vsync). So there is no DRM, Wayland or X11.

use slint::PhysicalSize;
use specular_core::device::Vsync;
use specular_linux::egl::{EglDisplay, Native};
use specular_linux::input::Input;
use specular_ui::platform::GlPlatform;

type Error = Box<dyn std::error::Error + Send + Sync>;

/// The panel's size in pixels.
pub const SCREEN: PhysicalSize = PhysicalSize::new(1024, 768);

/// The input devices read: padbridge's virtual pad, and the power key (`axp2202-pek`: the power
/// key input of the AXP2202 power-management chip).
const INPUT_DEVICES: &[&str] = &["TRIMUI Player1", "axp2202-pek"];

/// Specular's platform on the Brick's EGL display.
pub type BrickPlatform = GlPlatform<EglDisplay>;

/// The platform, on the thread that will run the event loop (the UI context is made current
/// there), before any other thread starts (see `fonts`). Animations are paced by `vsync`; without
/// it, by the swap alone, which waits for the frame before the last to be on screen (a frame takes
/// two refreshes to show; see docs/brick.md).
pub fn open(vsync: Option<Box<dyn Vsync>>) -> Result<BrickPlatform, Error> {
    crate::fonts::work_around_empty_fontconfig()?;
    GlPlatform::new(EglDisplay::open(Native::Default)?, SCREEN, Input::new(INPUT_DEVICES), vsync)
}
