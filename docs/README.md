# Design docs

Why Specular works the way it does: decisions, designs and research, one doc per topic. What is done and what is left is in [ROADMAP.md](../ROADMAP.md); how to work on it is in [DEVELOPMENT.md](../DEVELOPMENT.md).

| Doc | Covers |
|---|---|
| [architecture.md](architecture.md) | Stack, crates, threads, the device boundary, extension points, memory and licence |
| [look-and-motion.md](look-and-motion.md) | The Light Bar look, theme, motion, prefabs, fonts |
| [runtime.md](runtime.md) | The libretro runtime: frame pacing, sound, saves and states, cores |
| [library-and-box-art.md](library-and-box-art.md) | Scanning, systems, game identification, box art |
| [settings.md](settings.md) | Setting declarations, the config files on the SD card, migrations, the SD card layout |
| [wireless.md](wireless.md) | WiFi and Bluetooth screens and services, traces and scenarios |
| [installer.md](installer.md) | Installing the system to internal storage, from Settings |
| [brick.md](brick.md) | The TrimUI Brick port: display platform, hardware services, measurements |

## Format

Every doc follows this template. Leave out a section that would be empty, and keep the order.

````markdown
# <Topic>

Status: Planned | In progress | Implemented · Updated: <YYYY-MM-DD>

<One paragraph: what this covers and why it matters.>

## Decisions

- **<What we do, as a statement>** (<YYYY-MM-DD>). <Why.> Rejected: <alternative> (<why not>).

## Design

<How it works, or will work: structure, data flow, file layouts. New or changed interfaces,
types and formats go in as code snippets (a code outline).>

## Findings

- (<YYYY-MM-DD>, <kind>) <Fact.> Source: <file in this repo or a public link, if any>.

## Open questions

- <Question, and what would answer it.>
````

- **One doc per topic, kept current.** Update it in place as things change; it describes how things are, not how they got there. When a decision is reversed, rewrite it and put the old choice under "Rejected" with the reason.
- **Decisions** carry the date they were made and what was rejected, so they aren't relitigated.
- **Findings** are facts we relied on, each tagged with where it came from:
  - `device`: measured on a device.
  - `desktop`: measured on a PC.
  - `upstream`: from other projects, libraries or their docs.
  - `inferred`: reasoned but not verified.
- **No duplication.** Status of work belongs in the roadmap; how-to belongs in DEVELOPMENT.md; what the code makes obvious belongs nowhere.
- A new topic gets a new doc and a line in the table above.
