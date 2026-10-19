//! The art service, off the UI thread: a worker that finds missing pictures and stores them,
//! shrunk, in `cache/art/` (see `specular_core::art`), along with the frame each game was left
//! on, and a loader that decodes stored pictures for display. The loader always works on the
//! newest request, so scrolling never queues stale work.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use slint::{Rgba8Pixel, SharedPixelBuffer};
use specular_core::art::{self, ArtError, ArtIndex, ArtKind, ArtSource, Entry, GameDb, Identity, Method, IMAGE_SIZE};
use specular_core::library::{Game, SystemDecl};

/// A decoded picture, 8-bit RGBA.
pub struct Picture { pub width: u32, pub height: u32, pub rgba: Vec<u8> }

/// Decodes a PNG or JPEG file's bytes (told apart by their contents). A picture that would take
/// more than 64 MB to decode (a large photo) is refused rather than risk the device's memory.
pub fn decode(bytes: &[u8]) -> image::ImageResult<Picture> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(64 << 20);
    reader.limits(limits);
    let rgba = reader.decode()?.into_rgba8();
    Ok(Picture { width: rgba.width(), height: rgba.height(), rgba: rgba.into_raw() })
}

/// Shrinks `p` to fit in `max` × `max`, keeping its shape, by averaging the pixels each new pixel
/// covers. Pictures already small enough are returned as they are.
pub fn shrink(p: Picture, max: u32) -> Picture {
    if p.width <= max && p.height <= max { return p; }
    let scale = max as f64 / p.width.max(p.height) as f64;
    let (w, h) = (((p.width as f64 * scale).round() as u32).max(1), ((p.height as f64 * scale).round() as u32).max(1));
    let mut out = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        let (y0, y1) = (y * p.height / h, ((y + 1) * p.height / h).max(y * p.height / h + 1));
        for x in 0..w {
            let (x0, x1) = (x * p.width / w, ((x + 1) * p.width / w).max(x * p.width / w + 1));
            let mut sum = [0u32; 4];
            for sy in y0..y1 {
                let row = &p.rgba[((sy * p.width + x0) * 4) as usize..((sy * p.width + x1) * 4) as usize];
                for px in row.chunks_exact(4) { for c in 0..4 { sum[c] += px[c] as u32; } }
            }
            let n = (y1 - y0) * (x1 - x0);
            out.extend(sum.map(|s| ((s + n / 2) / n) as u8));
        }
    }
    Picture { width: w, height: h, rgba: out }
}

fn encode(p: &Picture) -> image::ImageResult<Vec<u8>> {
    use image::ImageEncoder;
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out).write_image(&p.rgba, p.width, p.height, image::ExtendedColorType::Rgba8)?;
    Ok(out)
}

/// Box art, title screens and screenshots from the libretro thumbnail server, for identified
/// games. Plain HTTP: see the box art decisions in docs/library-and-box-art.md.
pub struct LibretroThumbnails { agent: ureq::Agent }

impl LibretroThumbnails {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("Specular/", env!("CARGO_PKG_VERSION"), " (retro game launcher)"))
            .build();
        Self { agent: config.into() }
    }
}

impl ArtSource for LibretroThumbnails {
    fn name(&self) -> &'static str { "libretro thumbnails" }

    fn find(&self, _: &Game, identity: Option<&Identity>, system: &SystemDecl, kind: ArtKind) -> Result<Option<Vec<u8>>, ArtError> {
        let Some(url) = identity.and_then(|id| art::thumbnail_url(system, kind, &id.name)) else { return Ok(None) };
        match self.agent.get(&url).call() {
            // A download cut off part way is tried again later.
            Ok(mut r) => r.body_mut().with_config().limit(16 << 20).read_to_vec().map(Some).map_err(|e| ArtError::Offline(e.to_string())),
            Err(ureq::Error::StatusCode(404)) => Ok(None),
            // The server is having trouble, or the network is down: try again later.
            Err(e @ (ureq::Error::StatusCode(500..) | ureq::Error::Io(_) | ureq::Error::Timeout(_)
                | ureq::Error::HostNotFound | ureq::Error::ConnectionFailed)) => Err(ArtError::Offline(e.to_string())),
            Err(e) => Err(ArtError::Failed(format!("{url}: {e}"))),
        }
    }
}

/// What the worker is told.
pub enum ArtCmd {
    /// The games on the card (after each scan): the worker goes through all of them, in this
    /// order.
    Library(Vec<Game>),
    /// These games are on screen: they go first.
    OnScreen(Vec<PathBuf>),
    /// The kind of picture each system's games are shown with (`art::kind`; box art for systems
    /// left out). Only those kinds, then box art for games without, are downloaded. A change
    /// makes the worker go through the library again.
    Kinds(HashMap<&'static str, ArtKind>),
    /// While paused (a game is running), the worker reads and downloads nothing.
    Paused(bool),
    /// Whether the network is there (WiFi connected). Without it, the worker stops at the first
    /// game that needs a download, reports that it needs the network (through `worker`'s
    /// `need_network` callback), and waits for it.
    Online(bool),
    /// The game at this ROM path was quit showing this frame: stored as its picture of where it
    /// was left off before any other work, also while paused.
    LeftOff(PathBuf, Picture),
}

/// After every source said it has no picture of a kind for a game, they aren't asked for it again
/// for this long.
const RETRY_MISSING: Duration = Duration::from_secs(30 * 24 * 3600);
/// After the network failed or the card couldn't be written, the worker waits this long before
/// trying again.
const RETRY_LATER: Duration = Duration::from_secs(60);

/// Starts the worker, `online` or not (see `ArtCmd::Online`). It calls `stored` (on its own thread)
/// with a game's ROM path whenever it downloads a new picture. A game is skipped once it has a
/// picture of its kind; box art is fetched only when no source has that kind's, and never for a
/// game with the user's own image (which counts as box art). It calls `need_network(true)` when it
/// has a download waiting for the network, and `need_network(false)` once it has nothing left to do
/// (or is paused).
pub fn worker(roms: PathBuf, cache: PathBuf, systems: Vec<SystemDecl>, sources: Vec<Box<dyn ArtSource>>, online: bool,
              stored: impl Fn(PathBuf) + Send + 'static, need_network: impl Fn(bool) + Send + 'static) -> mpsc::Sender<ArtCmd> {
    let (tx, rx) = mpsc::channel();
    let queue = Queue { online, ..Queue::default() };
    let worker = Worker { roms, cache, systems, sources, tables: HashMap::new(), identified: Default::default(), downloaded: Default::default(), none: Default::default() };
    std::thread::spawn(move || worker.run(rx, queue, stored, need_network));
    tx
}

struct Worker {
    roms: PathBuf,
    cache: PathBuf,
    systems: Vec<SystemDecl>,
    sources: Vec<Box<dyn ArtSource>>,
    /// Each system's game table and index, loaded when first needed and dropped when the worker
    /// has nothing left to do, to free their memory.
    tables: HashMap<&'static str, (GameDb, ArtIndex)>,
    /// How many games were identified by each of `Method::ALL`, then how many weren't, since the
    /// worker last ran out of work (for one line in the log).
    identified: [usize; Method::ALL.len() + 1],
    /// How many pictures of each kind (by `ArtKind as usize`) were downloaded since the worker
    /// last ran out of work.
    downloaded: [usize; ArtKind::ALL.len()],
    /// How many games no source had a picture of each kind for, likewise.
    none: [usize; ArtKind::ALL.len()],
}

/// What became of one game.
enum Outcome {
    /// A new picture is in `cache/art/`.
    Stored,
    /// Done with it for now: it has a picture already, or no source has one.
    Nothing,
    /// The network is down, or the card couldn't be written: try again after `RETRY_LATER`.
    Later,
    /// It needs a download, and the worker was told the network isn't there.
    Offline,
}

/// The worker's to-do list. `done` holds the games dealt with since the last `ArtCmd::Library`
/// or change of `kinds`. `waiting_for_network` is set when a game needed a download while
/// offline, and cleared once nothing is left to process (or while paused).
#[derive(Default)]
struct Queue {
    games: Vec<Game>, on_screen: Vec<PathBuf>, kinds: HashMap<&'static str, ArtKind>, left_off: Vec<(PathBuf, Picture)>,
    done: HashSet<PathBuf>, paused: bool, online: bool, waiting_for_network: bool,
}

impl Queue {
    fn apply(&mut self, cmd: ArtCmd) {
        match cmd {
            // Each scan starts over; games that have a picture are skipped quickly.
            ArtCmd::Library(g) => { self.games = g; self.done.clear(); }
            ArtCmd::OnScreen(p) => self.on_screen = p,
            ArtCmd::Kinds(k) => { self.kinds = k; self.done.clear(); }
            ArtCmd::Paused(p) => self.paused = p,
            ArtCmd::Online(o) => self.online = o,
            ArtCmd::LeftOff(rom, frame) => self.left_off.push((rom, frame)),
        }
    }

    /// The kind of picture `game` is shown with.
    fn kind(&self, game: &Game) -> ArtKind { self.kinds.get(game.system).copied().unwrap_or(ArtKind::BoxArt) }

    /// The next game to work on: those on screen first, then the library in order.
    fn next(&self) -> Option<Game> {
        if self.paused { return None; }
        self.on_screen.iter().filter_map(|p| self.games.iter().find(|g| &g.path == p))
            .chain(&self.games).find(|g| !self.done.contains(&g.path)).cloned()
    }
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<ArtCmd>, mut queue: Queue, stored: impl Fn(PathBuf), need_network: impl Fn(bool)) {
        let mut retry_at = None::<Instant>;
        let mut needs_network = false;
        loop {
            if let Some((rom, frame)) = queue.left_off.pop() { self.store_left_off(&rom, frame); continue; }
            if queue.next().is_none() { queue.waiting_for_network = false; }
            if queue.waiting_for_network != needs_network {
                needs_network = queue.waiting_for_network;
                eprintln!("art: {}", if needs_network { "waiting for the network" } else { "done with the network" });
                need_network(needs_network);
            }
            let waiting = retry_at.is_some_and(|t| Instant::now() < t) || (queue.waiting_for_network && !queue.online);
            let Some(game) = queue.next().filter(|_| !waiting) else {
                // Nothing to do until told something, or until it's time to try again (never
                // while paused). The tables are kept while a retry is due.
                self.report();
                self.save();
                if retry_at.is_none() { self.tables.clear(); }
                let cmd = match retry_at.filter(|_| !queue.paused) {
                    Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
                    None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                };
                match cmd {
                    // The network is back: no need to wait out a retry.
                    Ok(ArtCmd::Online(true)) => { queue.online = true; retry_at = None; }
                    Ok(cmd) => queue.apply(cmd),
                    Err(mpsc::RecvTimeoutError::Timeout) => retry_at = None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
                continue;
            };
            match self.process(&game, queue.kind(&game), queue.online) {
                Outcome::Stored => { queue.done.insert(game.path.clone()); stored(game.path); }
                Outcome::Nothing => { queue.done.insert(game.path); }
                Outcome::Later => retry_at = Some(Instant::now() + RETRY_LATER),
                Outcome::Offline => queue.waiting_for_network = true,
            }
            for cmd in rx.try_iter() { queue.apply(cmd); }
        }
    }

    /// Logs how the games identified since the last report were found, e.g. "identified 12
    /// games: 9 by crc, 1 by fuzzy name, 2 unknown", and the pictures downloaded, e.g.
    /// "downloaded 4 pictures: 3 title, 1 boxart; none to be had for 2 games: 2 title".
    fn report(&mut self) {
        let names = Method::ALL.iter().map(|m| format!("by {}", m.name())).chain(["unknown".to_string()]);
        if let Some(counts) = tally(&self.identified, "games", names) { eprintln!("art: identified {counts}"); }
        let kinds = || ArtKind::ALL.iter().map(|k| k.id().to_string());
        match (tally(&self.downloaded, "pictures", kinds()), tally(&self.none, "games", kinds())) {
            (None, None) => {}
            (d, n) => eprintln!("art: downloaded {}; none to be had for {}", d.as_deref().unwrap_or("nothing"), n.as_deref().unwrap_or("any game")),
        }
        (self.identified, self.downloaded, self.none) = Default::default();
    }

    /// Stores `frame`, shrunk, as the picture of where the game at `rom` was left off.
    fn store_left_off(&self, rom: &Path, frame: Picture) {
        let (start, path, size) = (Instant::now(), art::cached_image(&self.cache, ArtKind::LeftOff, rom), (frame.width, frame.height));
        let small = shrink(frame, IMAGE_SIZE);
        match encode(&small).map_err(std::io::Error::other).and_then(|png| write(&png, &path).map(|()| png.len())) {
            Ok(bytes) => eprintln!("art: left off: {} ({}×{} from {}×{}, {} KB) in {} ms", path.display(), small.width, small.height,
                size.0, size.1, bytes.div_ceil(1024), start.elapsed().as_millis()),
            Err(e) => eprintln!("art: storing where {} was left off: {e}", rom.display()),
        }
    }

    fn save(&mut self) {
        for (_, index) in self.tables.values_mut() {
            if let Err(e) = index.save() { eprintln!("art: saving the index: {e}"); }
        }
    }

    /// Finds and stores a picture for `game`, shown as `kind`, unless it has one: of `kind`, else
    /// of box art if no source has `kind`'s (`ArtKind::fallbacks`). The ROM is identified first if
    /// its index entry is missing or out of date. Without the network (not `online`), it stops
    /// short of asking the sources.
    fn process(&mut self, game: &Game, kind: ArtKind, online: bool) -> Outcome {
        // The kinds to download, best first: those before the first the game has a picture of.
        // (Checked before identifying, so a game with the user's own image, whose system shows
        // box art, is never identified.)
        let has = |k| art::files(&self.roms, &self.cache, &game.path, k).iter().any(|p| p.exists());
        let wanted: Vec<ArtKind> = kind.fallbacks().iter().copied().take_while(|&k| !has(k)).filter(|k| k.thumbnails().is_some()).collect();
        if wanted.is_empty() { return Outcome::Nothing; }
        let Some(system) = self.systems.iter().find(|s| s.id == game.system).cloned() else { return Outcome::Nothing };
        let rom = self.roms.join(&game.path);
        let Ok(meta) = std::fs::metadata(&rom) else { return Outcome::Nothing };
        let modified = meta.modified().map_or(0, unix_secs);
        let (cache, identified) = (&self.cache, &mut self.identified);
        let (db, index) = self.tables.entry(system.id).or_insert_with(|| {
            let db = GameDb::parse(system.game_db);
            let index = ArtIndex::load(art::index_path(cache, system.id), &db.version);
            (db, index)
        });
        let mut entry = index.get(&game.path, meta.len(), modified).unwrap_or_else(|| {
            let identity = art::identify(&rom, &system, db).unwrap_or_else(|e| { eprintln!("art: {}: {e}", rom.display()); None });
            identified[identity.as_ref().and_then(|id| Method::ALL.iter().position(|&m| m == id.by)).unwrap_or(Method::ALL.len())] += 1;
            let e = Entry { size: meta.len(), modified, identity, missing: Default::default() };
            index.set(&game.path, &e);
            e
        });
        let now = unix_secs(SystemTime::now());
        for kind in wanted {
            let k = kind as usize;
            if entry.missing[k].is_some_and(|t| now.saturating_sub(t) < RETRY_MISSING.as_secs()) { continue; }
            if !online { return Outcome::Offline; }
            match fetch(&self.sources, cache, game, entry.identity.as_ref(), &system, kind) {
                Outcome::Nothing => {
                    self.none[k] += 1;
                    entry.missing[k] = Some(now);
                    index.set(&game.path, &entry);
                }
                o => { if matches!(o, Outcome::Stored) { self.downloaded[k] += 1; } return o; }
            }
        }
        Outcome::Nothing
    }
}

/// `counts` of `what`, by their `names`, e.g. "12 games: 9 by crc, 3 unknown" (leaving out
/// zeros); None if all are zero.
fn tally(counts: &[usize], what: &str, names: impl Iterator<Item = String>) -> Option<String> {
    let total: usize = counts.iter().sum();
    let each: Vec<String> = counts.iter().zip(names).filter(|(n, _)| **n > 0).map(|(n, name)| format!("{n} {name}")).collect();
    (total > 0).then(|| format!("{total} {what}: {}", each.join(", ")))
}

/// Asks `sources` in order for `game`'s picture of `kind`, and stores the first usable one in
/// `cache`: `Stored`, `Nothing` (none has one) or `Later`.
fn fetch(sources: &[Box<dyn ArtSource>], cache: &Path, game: &Game, identity: Option<&Identity>, system: &SystemDecl, kind: ArtKind) -> Outcome {
    for source in sources {
        match source.find(game, identity, system, kind) {
            Ok(Some(bytes)) => {
                // An image that doesn't decode counts as none; a failed write is retried.
                let png = match decode(&bytes).map(|p| encode(&shrink(p, IMAGE_SIZE))) {
                    Ok(Ok(png)) => png,
                    e => { eprintln!("art: {}: {}: {} not a usable image: {:?}", source.name(), game.path.display(), kind.id(), e.err()); continue; }
                };
                return match write(&png, &art::cached_image(cache, kind, &game.path)) {
                    Ok(()) => Outcome::Stored,
                    Err(e) => { eprintln!("art: storing {}'s {}: {e}", game.path.display(), kind.id()); Outcome::Later }
                };
            }
            Ok(None) => {}
            Err(ArtError::Offline(e)) => { eprintln!("art: {}: offline: {e}", source.name()); return Outcome::Later; }
            Err(ArtError::Failed(e)) => eprintln!("art: {}: {e}", source.name()),
        }
    }
    Outcome::Nothing
}

fn unix_secs(t: SystemTime) -> u64 { t.duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs()) }

/// Writes `png` to `path`. A write cut off part way never leaves a partial file at `path`: it is
/// written under another name and renamed. Unlike settings it isn't flushed to the card, as a
/// picture lost to a power cut is only fetched again (the loader deletes one that won't decode).
fn write(png: &[u8], path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")))?;
    let tmp = path.with_extension("png.tmp");
    std::fs::write(&tmp, png)?;
    std::fs::rename(&tmp, path)
}

/// Starts the loader. Send it the games whose pictures are wanted, by ROM path and the kind they
/// are shown as, most wanted first; each new list replaces the last. For each it calls `loaded`
/// (on its own thread) with the picture (the first of `art::pictures` there is), shrunk to
/// `IMAGE_SIZE` (the user's own may be any size); or None if the game has none yet.
pub fn loader(roms: PathBuf, cache: PathBuf, loaded: impl Fn(PathBuf, ArtKind, Option<SharedPixelBuffer<Rgba8Pixel>>) + Send + 'static)
    -> mpsc::Sender<Vec<(PathBuf, ArtKind)>> {
    let (tx, rx) = mpsc::channel::<Vec<(PathBuf, ArtKind)>>();
    std::thread::spawn(move || {
        let mut wanted = Vec::new();
        loop {
            if wanted.is_empty() { match rx.recv() { Ok(w) => wanted = w, Err(_) => return } }
            if let Some(w) = rx.try_iter().last() { wanted = w; }
            if wanted.is_empty() { continue; }
            let (rom, kind) = wanted.remove(0);
            let picture = art::pictures(&roms, &cache, &rom, kind).into_iter().find_map(|path| {
                let bytes = std::fs::read(&path).ok()?;
                match decode(&bytes) {
                    Ok(p) => Some(shrink(p, IMAGE_SIZE)),
                    Err(e) => {
                        eprintln!("art: {}: {e}", path.display());
                        // A broken cached picture is fetched again next time.
                        if path.starts_with(&cache) { let _ = std::fs::remove_file(&path); }
                        None
                    }
                }
            });
            loaded(rom, kind, picture.map(|p| SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height)));
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// A source with pictures of `has` for every game, recording what it is asked for.
    struct Fake { has: Vec<ArtKind>, asked: Arc<Mutex<Vec<ArtKind>>> }

    impl ArtSource for Fake {
        fn name(&self) -> &'static str { "fake" }
        fn find(&self, _: &Game, _: Option<&Identity>, _: &SystemDecl, kind: ArtKind) -> Result<Option<Vec<u8>>, ArtError> {
            self.asked.lock().unwrap().push(kind);
            Ok(self.has.contains(&kind).then(|| encode(&Picture { width: 1, height: 1, rgba: vec![9; 4] }).unwrap()))
        }
    }

    #[test]
    fn downloads_the_kind_shown_else_box_art_and_keeps_where_a_game_was_left_off() {
        let dir = tempfile::tempdir().unwrap();
        let (roms, cache) = (dir.path().join("roms"), dir.path().join("cache"));
        let gba = specular_core::library::builtin_systems().into_iter().find(|s| s.id == "gba").unwrap();
        let game = |name: &str| {
            let path = PathBuf::from(format!("gba/{name}.gba"));
            std::fs::create_dir_all(roms.join("gba")).unwrap();
            std::fs::write(roms.join(&path), b"not a real ROM").unwrap();
            Game { system: "gba", path, title: name.into(), sort_key: name.into() }
        };
        let asked = Arc::new(Mutex::new(Vec::new()));
        let source = Fake { has: vec![ArtKind::TitleScreen, ArtKind::BoxArt], asked: asked.clone() };
        let mut w = Worker { roms: roms.clone(), cache: cache.clone(), systems: vec![gba], sources: vec![Box::new(source)],
            tables: HashMap::new(), identified: Default::default(), downloaded: Default::default(), none: Default::default() };
        let mut process = |g: &Game, kind, online| { asked.lock().unwrap().clear(); let o = w.process(g, kind, online); (o, asked.lock().unwrap().clone()) };
        let cached = |kind, g: &Game| art::cached_image(&cache, kind, &g.path).exists();

        let a = game("A");
        assert!(matches!(process(&a, ArtKind::TitleScreen, false), (Outcome::Offline, v) if v.is_empty()));
        assert!(matches!(process(&a, ArtKind::TitleScreen, true), (Outcome::Stored, v) if v == [ArtKind::TitleScreen]));
        assert!(cached(ArtKind::TitleScreen, &a) && !cached(ArtKind::BoxArt, &a));
        assert!(matches!(process(&a, ArtKind::TitleScreen, true), (Outcome::Nothing, v) if v.is_empty()));
        // No screenshot: box art instead, and the screenshot isn't asked for again soon.
        let b = game("B");
        assert!(matches!(process(&b, ArtKind::Screenshot, true), (Outcome::Stored, v) if v == [ArtKind::Screenshot, ArtKind::BoxArt]));
        assert!(matches!(process(&b, ArtKind::Screenshot, true), (Outcome::Nothing, v) if v.is_empty()));
        // Where you left off is never downloaded; the user's own image is box art.
        let c = game("C");
        std::fs::write(roms.join("gba/C.png"), b"").unwrap();
        assert!(matches!(process(&c, ArtKind::LeftOff, true), (Outcome::Nothing, v) if v.is_empty()));
        assert!(matches!(process(&c, ArtKind::TitleScreen, true), (Outcome::Stored, v) if v == [ArtKind::TitleScreen]));
        drop(process);
        assert_eq!((w.downloaded[ArtKind::TitleScreen as usize], w.none[ArtKind::TitleScreen as usize]), (2, 0));
        assert_eq!((w.downloaded[ArtKind::Screenshot as usize], w.none[ArtKind::Screenshot as usize]), (0, 1));

        w.store_left_off(&c.path, Picture { width: 800, height: 400, rgba: vec![200; 800 * 400 * 4] });
        let stored = decode(&std::fs::read(art::cached_image(&cache, ArtKind::LeftOff, &c.path)).unwrap()).unwrap();
        assert_eq!((stored.width, stored.height), (IMAGE_SIZE, IMAGE_SIZE / 2));
    }

    #[test]
    fn shrinks_by_averaging_and_round_trips_png() {
        // 4×2: left half black, right half white → 2×1 at max 2: one black, one white pixel.
        let px = |v: u8| [v, v, v, 255];
        let rgba: Vec<u8> = [0, 0, 255, 255, 0, 0, 255, 255].iter().flat_map(|&v| px(v)).collect();
        let small = shrink(Picture { width: 4, height: 2, rgba }, 2);
        assert_eq!((small.width, small.height), (2, 1));
        assert_eq!(small.rgba, [px(0), px(255)].concat());
        let back = decode(&encode(&small).unwrap()).unwrap();
        assert_eq!((back.width, back.height, back.rgba), (2, 1, small.rgba));
    }

    #[test]
    fn decodes_jpeg() {
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg).encode(&[200; 8 * 4 * 3], 8, 4, image::ExtendedColorType::Rgb8).unwrap();
        let p = decode(&jpeg).unwrap();
        assert_eq!((p.width, p.height, p.rgba.len()), (8, 4, 8 * 4 * 4));
        assert!(p.rgba.chunks(4).all(|px| px[0].abs_diff(200) < 4 && px[3] == 255));
        assert!(decode(b"neither").is_err());
    }
}
