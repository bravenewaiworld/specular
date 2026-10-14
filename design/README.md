# Specular design

Specular should feel like part of the device it runs on: quiet, exact and quick. This spec sets the rules that both visual directions share. `mockup.html` shows the two directions we compared on the same screens (press T to switch). **Light Bar was chosen** (2026-10-11); Instrument was too cluttered and hard to read at real size:
- **Light Bar:** soft motion, a teal accent, and a glowing bar along the top edge that echoes the device's LED bar.
- **Instrument:** stepped motion, an orange accent, monospace labels, ruled lines and a dot-matrix clock.

Specular's UI will be written with Slint (a Rust UI toolkit). Once its theme file exists (`theme.slint`), that file becomes the source of the actual values, and this file keeps only the principles and the reasons behind them.

## Principles

- **The game is the point.** You should be able to start a game in a few presses, and nothing should get between you and it. Box art zooms into the running game, and the game shrinks back into its box art when you quit.
- **Two accent colours, from a palette.** All UI colour comes from two accents, each picked from a fixed palette of hues that suit the dark UI. The accent colours the light bar, toggles and unfocused gauges; the second accent colours the focus marks, and can be set to the same as the first. On devices with LEDs, the LEDs show them too (on the Brick, the top bar the accent and the buttons the second). The only other colours are box art and the optional per-system tint of the light bar.
- **Nothing moves unless you asked it to.** Animation follows input (focus, scrolling, opening or closing something) and then stops. Nothing runs in the background, so an idle screen draws nothing and the GPU can sleep. The one exception is the clock, which redraws once a minute.
- **Show the state, keep the internals on request.** Battery, storage and Wi-Fi are shown in plain words. Frame time, GPU, memory and config paths appear only after you turn on "Developer details".
- **Every screen is built from a few patterns.** Lists of rows, where each row is a header, text, toggle, choice, number or action. A grid of box art. A sheet, which is a picker for long choices such as Language. Any new screen has to be built from these.

## Device-relative sizes

All sizes are in **u**, a base unit worked out from the screen's pixel density: `u = ppi / 400`. That makes 1 u one Brick panel pixel (1024×768 at about 400 ppi). On another device, text keeps the same physical size, and a bigger screen shows more rows instead of bigger ones.

| Token | u | Use |
|---|---|---|
| title | 64 | Screen titles (Instrument: 58, in the dot-matrix face) |
| body | 42 | Row labels |
| secondary | 33 | Values, details |
| caption | 27 | Headers, hints, status. The smallest size for text you read; only labels on art and button icons go below it. |
| row | 92 | Row height (headers: 76) |
| gutter | 48 | Left and right margins |
| line | 2 | Thinnest line. At 400 ppi, 1 px is barely visible. |
| bottom radius | 12 | See corners, below |
| status bar / hint bar | 72 / 76 | Top and bottom bars |

**Corners.** Focus plates, cards, art and toasts copy the case of the Brick Hammer (the all-aluminium edition of the TrimUI Brick, our reference device): the top corners are sharp and only the bottom corners are rounded. Raised surfaces have a 2 u highlight along their top edge, like light catching a machined chamfer.

**Colour.**
- The background is a dark grey, never pure black. On an IPS panel, black saves no power and makes backlight bleed easier to see.
- Text is off-white.
- There are three surface steps.
- Focus never relies on colour alone: a focused row also gets a plate, heavier type and brighter text.

## Motion

Only use effects Slint can reproduce:
- timed animations with a standard ease or a cubic-bezier curve; for a spring-like feel, use an overshoot curve such as `cubic-bezier(.3, 1.4, .5, 1)`;
- no backdrop blur, blend modes or filters;
- the in-game menu's frosted backdrop is a blurred copy of the frame, made once when the menu opens;
- glows are drop shadows.

| | Light Bar | Instrument |
|---|---|---|
| Focus move | 220 ms, slight overshoot | 90 ms, 3 steps |
| List scroll | 280 ms, decelerate | 90 ms, 3 steps |
| Fade (text, toggles) | 160 ms | 90 ms |
| Screen change | 260 ms slide and fade | 120 ms fade in 4 steps, no slide |
| Launch zoom | 420 ms, `cubic-bezier(.05, .7, .1, 1)` (fast start, long ease-out) | 200 ms, 5 steps |
| Zoom to live game | 160 ms crossfade | 160 ms crossfade |
| Quit (game shrinks into its art) | 340 ms | 160 ms, 4 steps |
| In-game menu (slides in, game moves aside) | 240 ms / 280 ms | 100 ms / 120 ms, 4 steps |

## Input

The buttons are abstract (A, B, X, Y, L1, R1, L2, R2, Start, Select, Menu, the d-pad and the volume keys). Each device says which ones it has, and hints show only those.

- **Held directions:** the first repeat comes after 350 ms, then about 14 per second, rising to about 30 per second after 1.5 s of repeating.
- **List ends:** a list wraps around on a fresh press, never on a repeat.
- **L1 / R1:** jump by about 80% of a screen.
- **Number and choice rows:** left and right change the value.

## Open questions

- Whether held scrolling needs a letter index for long lists.
- Sounds: the mockup plays a quiet tick on each move; real sounds are still to be designed.
- Whether the light bar should react in-game, for example to low battery or a save.
