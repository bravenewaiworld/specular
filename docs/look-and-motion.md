# Look and motion

Status: In progress · Updated: 2026-10-18

How Specular looks, moves and feels under the thumbs: the Light Bar look, the theme and its device-relative sizes, the motion rules, the prefab components every screen is drawn with, and the fonts. Specular should feel like part of the device it runs on: quiet, exact and quick. The interactive reference is `design/mockup.html`; `crates/specular-ui/ui/theme.slint` holds the actual values.

## Decisions

- **The Light Bar look** (2026-10-11). Monochrome, with a thin glowing bar along the top edge that mirrors the device's LED bar. Geist for text, Geist Mono for numbers. Focus plates, cards, art and toasts copy the Brick Hammer's case (the all-aluminium edition of the TrimUI Brick): sharp top corners, rounded bottom corners, and a 2 u highlight along a raised surface's top edge, like light on a machined chamfer. Motion is short, with a slight overshoot. Rejected: the Instrument direction (stepped motion, an orange accent, monospace labels, ruled lines, a dot-matrix clock), which was cluttered, used too many typefaces, and was hard to read at real size. `design/mockup.html` shows the Light Bar look only.
- **Two accent colours, each picked from a fixed palette** (2026-10-14). All UI colour comes from them. The accent colours the light bar, toggles and unfocused gauges; the second accent colours the focus marks, and can be set to follow the first. On devices with LEDs, the LEDs show them too. The only other colours are box art, the low-battery warning, and (planned) a per-system tint of the light bar.
- **The game is the point** (2026-10-12). A game starts in a few presses, and nothing gets between you and it: box art zooms into the running game, and the game shrinks back into its box art when you quit.
- **Nothing moves unless you asked it to** (2026-10-12). Animation follows input (focus, scrolling, opening or closing something) and then stops. An idle screen draws nothing, so the GPU can sleep. The one exception is the clock, once a minute.
- **Sizes are in u, where u = ppi / 400** (2026-10-12). 1 u is one pixel of the Brick's panel (1024×768 at about 400 ppi). On another device text keeps its physical size, and a bigger screen shows more rows rather than bigger ones.
- **A dark grey background, never pure black** (2026-10-12). On an IPS panel black saves no power, and it makes backlight bleed easier to see.
- **Focus never relies on colour alone** (2026-10-12). A focused row also gets a plate, heavier type and brighter text.
- **The power button: a tap sleeps, a 1 s hold opens a power menu** (2026-10-14). The tap acts on release. The menu offers Power off, Restart and Cancel, also during a game (over its menu), and the game is quit, its saves written, before powering off or restarting.
- **The device sleeps when left alone, never during a running game** (2026-10-13). After 30 s by default ("Sleep after").
- **Only effects Slint can reproduce** (2026-10-12): timed animations with a standard ease or a cubic-bezier curve (an overshoot curve for a spring-like feel); no backdrop blur, blend modes or filters; glows are drop shadows. The in-game menu's frosted backdrop is to be a blurred copy of the frame, made once when the menu opens.
- **No fonts for non-Latin scripts are bundled** (2026-10-18). CJK fonts cost 15 MB or more, so Specular uses any font files in `{sd card}/fonts/` as fallbacks, and users who need one add it there.
- **The card's fonts are named one by one, after the emoji font, at start-up** (2026-10-18). Specular puts each TrueType or OpenType file directly in `{sd card}/fonts/` on `$SLINT_FONT_PATH`, after Noto Emoji, and logs which it uses and which it skips, and why. Fonts added later are used from the next start. Rejected: putting the folder itself on `$SLINT_FONT_PATH` (Slint would read whole every file in it, font or not, and say nothing of those it can't use); looking in folders inside it (Slint doesn't either); the card's fonts before Noto Emoji (a CJK font's own symbols would replace its emoji).
- **L2 and R2 jump between first letters in game lists and grids, and show the letter** (2026-10-18). The letter shows large, on a raised square over the middle of the list, for 0.7 s after the last jump, like the volume gauge after the last change. Held, they repeat like directions; at either end a fresh press wraps and a held one stops, as in lists. Rejected: a strip of letters down the side (always on screen, for something used now and then), L1 and R1 (they already jump by a screen).
- **Sounds are designed later** (2026-10-18), starting from the mockup's quiet tick on each move.
- **In-game LED, light bar and rumble reactions are pluggable effects** (2026-10-18), registered like the other extension points (see [architecture.md](architecture.md)).

## Design

### Theme

`theme.slint` has two globals: `Theme` (colours, type sizes, gutters, bar heights, all in u via `Theme.u`) and `Motion` (durations and easing curves). Row heights come from `specular_core::list`, because the core lays lists out. The accent colours come from the settings, set from Rust.

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
| Zoom to live game | 160 ms crossfade |
| Quit (game shrinks into its art) | 340 ms |
| In-game menu (slides in, game moves aside) | 240 ms / 280 ms |

The light bar's colour changes over 350 ms. Today the in-game menu sits on a panel down the left over the dimmed game; the blurred backdrop isn't drawn yet.

### Prefabs

Every screen is built from a few patterns, and any new screen has to be too:

- **Lists of rows.** Each row is one of: header, note (explanatory text, never focused), text, image (a game with its art), toggle, choice (a colour choice also shows its swatch), number, action, or a key of the on-screen keyboard.
- **A grid of box art.**
- **The index letter**: one large letter (150 u) on a raised square shaped like the focus plate, centred over the list.
- **A sheet**, a picker for long choices such as Language. No screen needs one yet.

Layouts are `List`, `ListWithPicture` (a game list beside the focused game's art), `Grid` and `Keys` (the on-screen keyboard). Toasts show for 2.5 s; notes never take the focus; the volume gauge shows for 1.2 s after the last change, and the index letter for 0.7 s after the last jump. Games without box art get a generated cover, a gradient derived from the title so each game keeps its own.

### Long lists

A list's drawing cost doesn't grow with its length. The core's list layout gives only the rows within a screen of the view. The UI draws them from a fixed set of reusable row slots (about 30 on the Brick); row *i* always uses slot *i* mod the slot count, so while a list scrolls a row keeps its slot and only the slots of rows coming into view change. A new screen or layout gets fresh slots, so the old one's rows don't animate into the new one. The app builds a screen's rows once and keeps them until its content changes.

While a direction is held, the picture beside the list changes only once the focus has rested for 120 ms. When more than 64 pictures are loaded, all but those on screen and the one beside the list are dropped.

### Buttons

Buttons are abstract (A, B, X, Y, L1, R1, L2, R2, Start, Select, Menu, the d-pad, the volume keys). Each device says which ones it has, and hints show only those.

- **Held directions, and L2 / R2:** the first repeat after 350 ms, then about 14 a second, rising to about 30 a second after 1.5 s of repeating. Repeats are driven by the caller's clock, so they are deterministic in tests.
- **List ends:** a list wraps around on a fresh press, never on a repeat. The same goes for choices.
- **L1 / R1:** jump by about 80 % of a screen.
- **L2 / R2:** in a list of games sorted by title, jump to the first game of the previous or next letter (from the middle of the M's, back to the first L). They repeat when held.
- **Number and choice rows:** left and right change the value.

### Fonts

Geist and Geist Mono are built into the UI. Network and Bluetooth device names can hold emoji, which Geist lacks, so Noto Emoji is a fallback font. Slint looks for fallback fonts only among the system's and those in `$SLINT_FONT_PATH`, not among fonts built into the UI, so Noto Emoji ships as a file (`fonts/` next to the binary) and Specular sets `$SLINT_FONT_PATH` at start-up (`specular_ui::run`, before Slint starts and before any other thread does), followed by the font files in `{sd card}/fonts/`. A `$SLINT_FONT_PATH` already set takes Noto Emoji's place. Each of the card's fonts counts towards Specular's memory, whole.

## Findings

- (2026-10-12, desktop) With 3,000 games, a focus move takes about 15 µs. Source: the ignored benchmark in `crates/specular-core/src/app/tests.rs`.
- (2026-10-12, device) On the Brick, 3,000 games use 22 MB resident (21 MB with none). Scrolling continuously at 20 moves a second uses about 47 % of one core at the `ondemand` governor's low clock, mostly drawing. Source: Brick measurements of the long-list build.
- (2026-10-12, upstream) CJK fonts cost 15 MB or more. Source: sizes of common CJK font files.
- (2026-10-18, upstream) Slint 1.18.1 reads `$SLINT_FONT_PATH` when it builds its font collection: a list split as `$PATH` is (`:` on Linux), each entry a font file or a folder whose files it reads (not those in folders inside it). It reads each file whole into memory and silently skips any it can't parse. Their families become the fallback (sans-serif) families, in order. Source: `i-slint-common-1.18.1/sharedfontique.rs`.
