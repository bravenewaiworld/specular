# Look and motion

Status: In progress · Updated: 2026-10-19

How Specular looks, moves and feels under the thumbs: the Light Bar look, the theme and its device-relative sizes, the motion rules, the prefab components every screen is drawn with, the sounds, and the fonts. Specular should feel like part of the device it runs on: quiet, exact and quick. The interactive reference is `design/mockup.html`; `crates/specular-ui/ui/theme.slint` holds the actual values.

## Decisions

- **The Light Bar look** (2026-10-11). Monochrome, with a thin glowing bar along the top edge that mirrors the device's LED bar. Geist for text, Geist Mono for numbers. Focus plates, cards, art and toasts copy the Brick Hammer's case (the all-aluminium edition of the TrimUI Brick): sharp top corners, rounded bottom corners, and a 2 u highlight along a raised surface's top edge, like light on a machined chamfer. Motion is short, with a slight overshoot. Rejected: the Instrument direction (stepped motion, an orange accent, monospace labels, ruled lines, a dot-matrix clock), which was cluttered, used too many typefaces, and was hard to read at real size. `design/mockup.html` shows the Light Bar look only.
- **Two accent colours, each picked from a fixed palette** (2026-10-14). All UI colour comes from them. The accent colours the light bar, toggles and unfocused gauges; the second accent colours the focus marks, and can be set to follow the first. On devices with LEDs, the LEDs show them too. The only other colours are games' pictures, the low-battery warning, the effects' white flash, and each system's colour, which the light bar takes on that system's screens (below).
- **The light bar takes a system's colour on its screens** (2026-10-18). While "Tint by system" is on (the default), the light bar, and the LEDs that show the first accent, take a system's colour while its game list is the current screen or one of its games is played (its menu open or not). Search results span systems, so they keep the accent (a game played from them takes its system's colour), as does the keyboard over a game list. The second accent never changes, so the focus always looks the same. Each system has its own palette colour, after its hardware or logo, and none is teal, the default accent, so the change shows: GB lime (the original's green screen), GBC rose (its berry shell), GBA indigo, NES coral, SNES violet (the American pad's purple buttons), Mega Drive sky (Sega's blue), PS1 amber (the yellow of its logo). Rejected: tinting Home as the focus passes over each system, as the mockup does (the colour would flicker while scrolling); tinting search results when they all are one system's (the colour would depend on the query); colours outside the palette (its colours are checked to suit the dark UI and to read under dark text and on the LEDs).
- **The game is the point** (2026-10-12). A game starts in a few presses, and nothing gets between you and it: its picture zooms into the running game, and the game shrinks back into its picture when you quit.
- **A splash names the core while its game starts** (2026-10-18). "Powered by" the core's name, its copyright notice and its web address, on a raised plate at the bottom of the zooming picture, from the launch until the game's first frame arrives, and for at least 1 s; then the game shows as usual. The core loads and the game runs meanwhile: only showing it waits, so its first second plays unseen. A game that fails to start takes the splash with it, and the notice says why. The notice and the web address come from the credits (`data/credits.tsv`, and `tools/build-cores.sh` for the address), so each is written once. A core the user added isn't shipped, so it isn't in the credits: its splash shows its `.info`'s display name, and its authors and licence on the notice's line (2026-10-18). Rejected: a splash screen before the zoom (it would come between you and the game); starting the core once the splash ends (a slower start for every game); the notice in `CoreDecl` (a second copy of what the credits hold).
- **A game's picture is drawn in its kind's shape** (2026-10-18). Box art (whose shape varies) is fitted into a square; title screens, screenshots and where you left off (see [library-and-box-art.md](library-and-box-art.md)) into the shape of their system's screen (3:2 for the GBA), beside the list and in the grid, whose cells take that shape too. A picture shaped like the screen zooms into the running game without changing shape, and with "Where you left off" shown, the game shrinks back into the frame you left it on. One shape serves a whole screen: a screen whose games' shapes differ (search results across systems) uses squares. Rejected: each cell in its own picture's shape (a ragged grid, and the shape isn't known until the picture has loaded); square places for every kind (a 3:2 screenshot would fill two thirds of its place).
- **Nothing moves unless you asked it to** (2026-10-12). Animation follows input (focus, scrolling, opening or closing something) and then stops. An idle screen draws nothing, so the GPU can sleep. The one exception is the clock, once a minute.
- **Sizes are in u, where u = ppi / 400** (2026-10-12). 1 u is one pixel of the Brick's panel (1024×768 at about 400 ppi). On another device text keeps its physical size, and a bigger screen shows more rows rather than bigger ones.
- **A dark grey background, never pure black** (2026-10-12). On an IPS panel black saves no power, and it makes backlight bleed easier to see.
- **Focus never relies on colour alone** (2026-10-12). A focused row also gets a plate, heavier type and brighter text.
- **A note that would stay partly out of view takes the focus, to be read** (2026-10-19). Notes are passed over, except one wholly in view neither before nor after the move (such as one too tall to show whole together with the row next to it, or one above the first row the focus can land on that is out of view with the focus there): the focus stops on it, without a plate, and each Up or Down scrolls the list three lines, until the row below it (or its top, going up) is wholly in view; the next press moves on. A screen opens on such a note above its first row, read from the top. Notes right above the focused row (which usually explain it) stay in view as far as the row does. Paging (L1, R1) passes notes. See `list::read_step`. Rejected: notes that always take the focus (an extra press past every one); scrolling to a long note's top only (one taller than the view could never be read whole).
- **The power button: a tap sleeps, a 1 s hold opens a power menu** (2026-10-14). The tap acts on release. The menu offers Power off, Restart and Cancel, also during a game (over its menu), and the game is quit, its saves written, before powering off or restarting.
- **The device sleeps when left alone, never during a running game** (2026-10-13). After 30 s by default ("Sleep after").
- **Nor while a PC is connected over the USB network link** (2026-10-19). Someone is working on the device over it (SSH, file transfer), sleep would cut the link, and the device charges from the PC meanwhile. The link is the USB gadget interface (`usb…`) being up with a link, as the WiFi service reports the other interfaces every few seconds; pulling the cable restarts the idle count. Rejected: any wired interface (a USB Ethernet adapter on a dock is no sign of someone at the device); the USB controller's state in sysfs (Brick-specific, where the interface is any Linux gadget's).
- **Only effects Slint can reproduce** (2026-10-12): timed animations with a standard ease or a cubic-bezier curve (an overshoot curve for a spring-like feel); no backdrop blur, blend modes or filters; glows are drop shadows. The in-game menu's frosted backdrop is to be a blurred copy of the frame, made once when the menu opens (in V1).
- **No fonts for non-Latin scripts are bundled** (2026-10-18). CJK fonts cost 15 MB or more, so Specular uses any font files in `{sd card}/fonts/` as fallbacks, and users who need one add it there.
- **The card's fonts are named one by one, after the emoji font, at start-up** (2026-10-18). Specular puts each TrueType or OpenType file directly in `{sd card}/fonts/` on `$SLINT_FONT_PATH`, after Noto Emoji, and logs which it uses and which it skips, and why. Fonts added later are used from the next start. Rejected: putting the folder itself on `$SLINT_FONT_PATH` (Slint would read whole every file in it, font or not, and say nothing of those it can't use); looking in folders inside it (Slint doesn't either); the card's fonts before Noto Emoji (a CJK font's own symbols would replace its emoji).
- **L2 and R2 jump between first letters in game lists and grids, and show the letter** (2026-10-18). The letter shows large, on a raised square over the middle of the list, for 0.7 s after the last jump, like the volume gauge after the last change. Held, they repeat like directions; at either end a fresh press wraps and a held one stops, as in lists. Rejected: a strip of letters down the side (always on screen, for something used now and then), L1 and R1 (they already jump by a screen).
- **Interface sounds are a few short tones, made in code from the mockup's tick** (2026-10-18). A quiet tick as the focus moves or a value steps; two tones rising as a screen opens, and falling as one closes; one tone above or below the tick as a switch turns on or off. Other actions (joining a network, say) make none. "Interface sounds" turns them off. They play on the current output at its volume, and not at all while muted or at volume 0. Rejected: shipping sound files (a decoder and files to install, for tones a few lines of code make); one sound for everything, as in the mockup (opening and going back would sound alike); a sound as a game starts (the game would have to wait for the interface sounds' output, maybe a slow Bluetooth one, to open and close before opening its own; the zoom into the game says it started).
- **No interface sounds over a game, even paused in its menu** (2026-10-18). The game has the sound output, and an ALSA device that doesn't mix can't be opened twice. Rejected: mixing them into the paused game's stream (the paused game's thread would have to keep writing sound, only for the sake of a few ticks).
- **A held direction ticks at most every 60 ms** (2026-10-18), about 15 times a second, so the 30 moves a second of a long hold don't buzz. Rejected: shorter ticks as the repeats speed up (at 30 a second they still run together).
- **LED, light bar and rumble reactions are pluggable effects** (2026-10-18), registered like the other extension points (see [architecture.md](architecture.md)): an ordered list of `Effect`s, each reacting to events (a battery reading, the game saving in-game, a save state made, the game's sound, the game stopping) and drawing over the accents for a moment, later ones over earlier ones. Effects only react to events and end by themselves, so between them nothing runs and an idle screen still draws nothing. Rejected: effects that run on a clock (a breathing light bar), which would break "Nothing moves unless you asked it to".
- **Effects colour the lights by accent, not LED by LED** (2026-10-18): one colour for the LEDs of each accent, and the on-screen light bar shows the first accent's, as the LED bar does. One place works out what the lights show (`Runner::show_lights`: the accents, the system's colour in place of the first when there is one, then the effects over them) and sets both, so the light bar mirrors the LEDs on any device, also one without LEDs (the desktop). Rejected: a colour per LED (no effect needs one, and the screen has no counterpart for most LEDs); effects setting the LEDs and the theme separately (they could disagree).
- **The built-in effects: a low-battery pulse, a flash on saving, and rumble following the sound** (2026-10-18). As the battery becomes low (unplugged at 15 % or less, also at start-up), every light pulses three times between the warning colour and the accents (half a second each); the LEDs switch, and the light bar fades, its colour changing over 350 ms. When the game saves in-game or a save state is made, the lights flash white for 0.3 s. Rumble following the sound rumbles for 0.1 s on a frame whose loudest sample is at least -12 dBFS and 1.5 times the recent average (each frame moves the average an eighth of the way), so sudden sounds rumble and steady loud music settles. Rejected: pulsing for as long as the battery is low, or on every reading (constant motion; the status bar's battery already shows it); a smooth fade on the LEDs (each LED update is 42 sysfs writes on the Brick); a fixed loudness threshold (loud music would rumble without end); the autosave flashing (it is every 10 s).
- **Each built-in effect has a user setting; rumble following the sound is off by default** (2026-10-18). "Low battery warning" and "Flash when the game saves" under Lights (on), "Follow the sound" under Rumble (off). People disagree on lights blinking at night and on rumble they didn't ask a game for, which is what user settings are for; the sound-following rumble is a novelty, so it is offered rather than imposed. Rejected: developer settings (these are matters of taste, not debugging).
- **The game's rumble and the effects' share one strength setting** (2026-10-18): "Rumble", 0 (never) to 5 (as strongly as asked), default 5. Rejected: an on/off switch only (no way to soften a strong motor); separate strengths for games and effects (the effect has its own switch already).
- **While a list scrolls, only the rows coming into view and the two the focus moves between are laid out again** (2026-10-18). The focused row is marked in its slot's data, and a cover without art holds a shared 1×1 image instead of Slint's empty one (see Findings: both made every row near the view change at each move, and Slint shape all their text again). Rejected: matching each row's index against a focused-row property (about 20 texts shaped a move, against 5).
- **The frame statistics don't count texts shaped** (2026-10-19). They count the row slots changed, and the CPU time a frame shows what a frame costs. Rejected: counting Slint's text layout cache misses (Slint offers them only in its internal crate's API, with its `testing` feature, which ties Specular to one Slint version; the counts of texts shaped in Findings were taken that way).
- **The status and hint bars are drawn through textures, redrawn only when they change** (2026-10-18), with Slint's `cache-rendering-hint`: FemtoVG builds every rounded shape anew each frame, and the battery and the buttons' marks were among the costliest while scrolling. Rejected: the focus plate and the picture beside a list too (through a texture their glow and gradient come out slightly different, by up to 16 of 255 in a colour channel); hiding the row slots that are off screen (they already aren't drawn, and walking their items costs under 1 % of the UI thread on the desktop).

## Design

### Theme

`theme.slint` has two globals: `Theme` (colours, type sizes, gutters, bar heights, all in u via `Theme.u`) and `Motion` (durations and easing curves). Row heights come from `specular_core::list`, because the core lays lists out. The accent colours come from the settings, set from Rust, and the light bar's from `App::tint` (the accent, or the system's colour).

The palette (`specular_core::palette`) is eight hues at about teal's saturation and lightness, so any of them suits the dark UI and reads on the LEDs. Each palette colour, and the warning colour, is light enough for dark text to be read on it; a test checks this.

Colours: a dark grey background, three surface steps, off-white text. The smallest text you read is the caption size (27 u); only labels on art and button icons go below it. The thinnest line is 2 u, as 1 px is barely visible at 400 ppi.

### Motion

The Light Bar timings, from the spec:

| Motion | Timing |
|---|---|
| Focus move | 220 ms, slight overshoot, `cubic-bezier(.3, 1.4, .5, 1)` |
| List scroll | 280 ms, decelerate |
| Fade (text, toggles) | 160 ms |
| Screen change | 260 ms slide and fade |
| Launch zoom | 420 ms, `cubic-bezier(.05, .7, .1, 1)` (fast start, long ease-out) |
| Splash (over the picture while the game starts) | 160 ms fade in and out; up at least 1 s, until the first frame |
| Zoom to live game | 160 ms crossfade |
| Quit (game shrinks into its art) | 340 ms |
| In-game menu (slides in, game moves aside) | 240 ms / 280 ms |

The light bar's colour changes over 350 ms (also between the accent and a system's colour). Today the in-game menu sits on a panel down the left over the dimmed game; the blurred backdrop is for V1.

### Prefabs

Every screen is built from a few patterns, and any new screen has to be too:

- **Lists of rows.** Each row is one of: header, note (explanatory text, focused only to be read), text, image (a game with its art), toggle, choice (a colour choice also shows its swatch), number, action, or a key of the on-screen keyboard.
- **A grid of pictures**, each in its kind's shape.
- **A row's own hint**: a button hint a row adds while it is focused, before the screen's (X Reset on a setting that overrides).
- **The index letter**: one large letter (150 u) on a raised square shaped like the focus plate, centred over the list.
- **The splash**: three centred lines (the core, its copyright notice, its web address) on a raised plate shaped like the focus plate, at the bottom of the screen over the picture of a game starting.
- **A sheet**, a picker for long choices such as Language. No screen needs one yet.

Layouts are `List`, `ListWithPicture` (a game list beside the focused game's art), `Grid` and `Keys` (the on-screen keyboard). Toasts show for 2.5 s, a long one long enough to read (65 ms a character); the volume gauge shows for 1.2 s after the last change, and the index letter for 0.7 s after the last jump. Games without a picture get a generated cover, a gradient derived from the title so each game keeps its own. How tall pictures are, as a fraction of their width, is `list::picture_shape(rows)` (from each row's `ImageRef::Art { shape, .. }`): the grid's layout takes it (`list::layout(rows, focus, None, width, height, Some(shape))`), and the window's `picture-shape` sizes the picture beside the list, the grid's pictures and the flying cover.

### Long lists

A list's drawing cost doesn't grow with its length. The core's list layout gives only the rows within a screen of the view. The UI draws them from a fixed set of reusable row slots (`list::max_drawn`: three views' height of the shortest row, plus two; 23 in a list on the Brick, 32 in a grid); row *i* always uses slot *i* mod the slot count, so while a list scrolls a row keeps its slot and only the slots of rows coming into view change, and those of the rows the focus leaves and reaches. A new screen or layout gets fresh slots, so the old one's rows don't animate into the new one. The app builds a screen's rows once and keeps them until its content changes.

While a direction is held, the picture beside the list changes only once the focus has rested for 120 ms. When more than 64 pictures are loaded, all but those on screen and the one beside the list are dropped.

### Buttons

Buttons are abstract (A, B, X, Y, L1, R1, L2, R2, Start, Select, Menu, the d-pad, the volume keys). Each device says which ones it has, and hints show only those.

- **Held directions, and L2 / R2:** the first repeat after 350 ms, then about 14 a second, rising to about 30 a second after 1.5 s of repeating. Repeats are driven by the caller's clock, so they are deterministic in tests.
- **List ends:** a list wraps around on a fresh press, never on a repeat. The same goes for choices.
- **L1 / R1:** jump by about 80 % of a screen.
- **L2 / R2:** in a list of games sorted by title, jump to the first game of the previous or next letter (from the middle of the M's, back to the first L). They repeat when held.
- **Number and choice rows:** left and right change the value.

### Sounds

| Sound | When | Tones |
|---|---|---|
| Move | the focus moves (also by a letter jump), a value steps, a key is used on the on-screen keyboard | 1250 Hz |
| Open | a screen opens | 1250 Hz, then 1875 Hz 40 ms later |
| Back | a screen closes | 1875 Hz, then 1250 Hz 40 ms later |
| Switch on / off | a switch is turned on or off | 1875 Hz / 833 Hz |

Each tone is the mockup's: a sine whose gain rises exponentially from 0.0001 to 0.03 (−30 dBFS) in 2 ms, then falls exponentially back to 0.0001 by 25 ms, where it stops. The other tones are a fifth above or below the tick.

The app says which sound an interaction makes (`app::Sound`, as `Effect::Sound`), and drops those that mustn't play. The UI's `sounds` module makes each sound once, at the output's rate, and plays it on a thread of its own, so playing one is a copy into the output's queue. The output is opened at the first sound (opening can block) and closed after 2 s without one (an open output keeps the sound hardware, or a Bluetooth stream, powered), and before a game opens its own. A sound that can't start within 100 ms of being asked for (a Bluetooth output slow to open) is dropped. After a failure to open or play, sounds are dropped for 2 s, or until the output changes, and each failure is logged once. The log's `sound:` lines say when the interface sounds' output opens and closes, and why.

### Lights and rumble effects

`specular_core::effects`, run on the UI thread, which feeds it events and sets the LEDs, the light bar's tint and the effects' rumble from each `frame`:

```rust
pub enum Event { Battery(Battery), GameSaved, StateSaved, Sound(u16 /* frame's peak, 0–32768 */), GameStopped }

pub struct Frame { pub lights: [Rgb; 2] /* by Accent */, pub rumble: Motors }

pub trait Effect {
    fn name(&self) -> &'static str;                               // for the log
    fn setting(&self) -> &'static str;                            // the Bool setting turning it on
    fn event(&mut self, event: Event, now: Duration) -> bool;     // true: it starts (logged)
    fn draw(&self, now: Duration, frame: &mut Frame) -> Option<Duration>;  // its next change; None: showing nothing
}

impl Effects {
    pub fn new(list: Vec<Box<dyn Effect>>) -> Self;               // all off until their settings apply
    pub fn apply(&mut self, key: &str, value: &Value);
    pub fn event(&mut self, event: Event, now: Duration, started: impl FnMut(&'static str));
    pub fn frame(&self, now: Duration, accents: [Rgb; 2]) -> (Frame, Option<Duration>);
}
pub fn builtin() -> Vec<Box<dyn Effect>>;  // LowBattery, SaveFlash, FollowSound
```

A battery reading comes every 10 s; the game's thread reports an in-game save (`Outputs::on_save`) and keeps the loudest sample of its sound, which the UI takes with each game frame it receives (`Session::take_sound_peak`), so following the sound costs no extra wake-ups. A timer wakes the UI at the effects' next change and stops when none runs. The LEDs are only written when what they show changes.

### Fonts

Geist and Geist Mono are built into the UI. Network and Bluetooth device names can hold emoji, which Geist lacks, so Noto Emoji is a fallback font. Slint looks for fallback fonts only among the system's and those in `$SLINT_FONT_PATH`, not among fonts built into the UI, so Noto Emoji ships as a file (`fonts/` next to the binary) and Specular sets `$SLINT_FONT_PATH` at start-up (`specular_ui::run`, before Slint starts and before any other thread does), followed by the font files in `{sd card}/fonts/`. A `$SLINT_FONT_PATH` already set takes Noto Emoji's place. Each of the card's fonts counts towards Specular's memory, whole.

## Findings

- (2026-10-12, desktop) With 3,000 games, a focus move takes about 15 µs. Source: the ignored benchmark in `crates/specular-core/src/app/tests.rs`.
- (2026-10-12, device) On the Brick, 3,000 games use 22 MB resident (21 MB with none). Scrolling continuously at 20 moves a second uses about 47 % of one core at the `ondemand` governor's low clock, mostly drawing. Source: Brick measurements of the long-list build.
- (2026-10-18, upstream) Slint's FemtoVG renderer redraws the whole window each frame (no dirty regions). An item outside the current clip isn't drawn, but its children are still visited, each tested against the clip in turn; an item with `visible: false` stops the walk, so empty row slots (`shown` false) aren't walked. Of a list's 23 row slots, the 6 or so on screen are drawn, and the other rows near the view (about 12 while scrolling) are walked and not drawn. Source: `i-slint-renderer-femtovg-1.18.1/lib.rs` (`internal_render_with_post_callback`), `i-slint-core-1.18.1/item_rendering.rs` (`render_item_children`, `filter_item`), `i-slint-compiler-1.18.1/passes/visible.rs`.
- (2026-10-18, upstream) Slint keeps each text's shaping (`TextLayoutCache`, per item) until a property the shaping read (text, font, size, weight) is marked changed. Slint marks everything depending on a changed property as changed without comparing values, so a row's `font-weight: index == focused-row ? …` was marked changed in every row at each focus move. FemtoVG keeps each glyph it rasterised in its atlas (by glyph, font, size and a tenth of a pixel of horizontal position) for good; glyphs over 92 px (the letter index's 150 u letter) are drawn as paths every frame. Source: `i-slint-core-1.18.1/textlayout/sharedparley/cache.rs`, `femtovg-0.27.0/src/text.rs` (`render_atlas`), `src/lib.rs` (`draw_glyph_run`).
- (2026-10-18, upstream) Two empty Slint images are never equal, even an image and itself (`ImageInner::eq` has no case for `None`), so a model row or a property holding one looks changed whenever it is set. Source: `i-slint-core-1.18.1/graphics/image.rs`.
- (2026-10-18, upstream) Slint's FemtoVG renderer builds a new path for every rounded rectangle and border each frame, and FemtoVG tessellates it anew (its path cache lives in the path). `cache-rendering-hint` draws an element and its children into a texture, kept until a property they read changes. Source: `i-slint-renderer-femtovg-1.18.1/itemrenderer.rs` (`draw_border_rectangle`), `i-slint-core-1.18.1/item_rendering.rs` (`render_layer`).
- (2026-10-18, desktop) Holding down in a list of 3,000 games for 6 s (about 150 moves, 370 frames): every one of the 23 row slots was set anew at each `Runner::refresh` (one a move while held; two for a single press, which also loads the picture beside the list), and about 20 texts were shaped a move (2,945 over the hold). With the `focused` flag and the shared 1×1 image (Decisions), 3 or 4 slots change and 3 texts are shaped a move (451 over the hold; with about a dozen texts drawn a frame, roughly 9 in 10 come from the cache). Of the UI thread's time while scrolling, drawing the scene (walking it and FemtoVG building paths and glyph runs) went from about 41 % to 24 % once the bars were cached; shaping from 7–8 % to 1–3 %; tessellating rounded shapes from 8 % to 5 %. The rest is mostly the GL driver and swap (about 20 %, Mesa, so not the Brick's), building the glyphs' textured rectangles (10 %), the event loop, and Specular's own work per move (about 3 %). Walking off-screen slots took under 1 %. Glyph rasterisation took about 4 %, for glyphs at positions not drawn before. The desktop's CPU time a frame (about 1 ms) was too noisy, on a loaded machine, to tell the builds apart. Source: scripted runs (`SPECULAR_SCRIPT=…,down:6000,up:6000`), the frame statistics, and a sampler of the UI thread's stacks (every 1 ms of its CPU time; a throwaway tool, not in the repo).
- (2026-10-12, upstream) CJK fonts cost 15 MB or more. Source: sizes of common CJK font files.
- (2026-10-18, upstream) Slint 1.18.1 reads `$SLINT_FONT_PATH` when it builds its font collection: a list split as `$PATH` is (`:` on Linux), each entry a font file or a folder whose files it reads (not those in folders inside it). It reads each file whole into memory and silently skips any it can't parse. Their families become the fallback (sans-serif) families, in order. Source: `i-slint-common-1.18.1/sharedfontique.rs`.

## Open questions

- How much of a core does scrolling take on the Brick now, against the 47 % measured before the fixes, and how much of each frame is drawing? The frame statistics' `ui: CPU a frame` line answers it (see DEVELOPMENT.md).
