//! The art service, off the UI thread: a worker that finds missing box art and stores it, shrunk,
//! in `cache/art/` (see `specular_core::art`), and a loader that decodes stored pictures for
//! display. The loader always works on the newest request, so scrolling never queues stale work.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime};

use slint::{Rgba8Pixel, SharedPixelBuffer};
use specular_core::art::{self, ArtError, ArtIndex, ArtSource, Entry, GameDb, Identity, IMAGE_SIZE};
use specular_core::library::{Game, SystemDecl};

/// A decoded picture, 8-bit RGBA.
pub struct Picture { pub width: u32, pub height: u32, pub rgba: Vec<u8> }

/// Decodes a PNG file's bytes.
pub fn decode(bytes: &[u8]) -> Result<Picture, png::DecodingError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Palettes and small bit depths become 8-bit channels.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info()?;
    let mut buf = vec![0; reader.output_buffer_size().unwrap_or(0)];
    let info = reader.next_frame(&mut buf)?;
    buf.truncate(info.buffer_size());
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::Rgb => buf.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        // Grayscale (an indexed image is already expanded to RGB or RGBA).
        _ => buf.iter().flat_map(|&g| [g, g, g, 255]).collect(),
    };
    Ok(Picture { width: info.width, height: info.height, rgba })
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

fn encode(p: &Picture) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    let mut e = png::Encoder::new(&mut out, p.width, p.height);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    e.write_header()?.write_image_data(&p.rgba)?;
    Ok(out)
}

/// Box art from the libretro thumbnail server, for identified games. Plain HTTP: see the box
/// art decisions in ROADMAP.md.
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

    fn find(&self, _: &Game, identity: Option<&Identity>, system: &SystemDecl) -> Result<Option<Vec<u8>>, ArtError> {
        let Some(id) = identity else { return Ok(None) };
        let url = art::thumbnail_url(system, &id.name);
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
    /// While paused (a game is running), the worker reads and downloads nothing.
    Paused(bool),
}

/// After every source said it has no image, they aren't asked again for this long.
const RETRY_MISSING: Duration = Duration::from_secs(30 * 24 * 3600);
/// After the network failed or the card couldn't be written, the worker waits this long before
/// trying again.
const RETRY_LATER: Duration = Duration::from_secs(60);

/// Starts the worker. It calls `stored` (on its own thread) with a game's ROM path whenever it
/// stores a new picture. Games that already have a picture, the user's or a cached one, are
/// skipped, so the user's own are never replaced.
pub fn worker(roms: PathBuf, cache: PathBuf, systems: Vec<SystemDecl>, sources: Vec<Box<dyn ArtSource>>,
              stored: impl Fn(PathBuf) + Send + 'static) -> mpsc::Sender<ArtCmd> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || Worker { roms, cache, systems, sources, tables: HashMap::new() }.run(rx, stored));
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
}

/// What became of one game.
enum Outcome {
    /// A new picture is in `cache/art/`.
    Stored,
    /// Done with it for now: it has a picture already, or no source has one.
    Nothing,
    /// The network is down, or the card couldn't be written: try again after `RETRY_LATER`.
    Later,
}

/// The worker's to-do list. `done` holds the games dealt with since the last `ArtCmd::Library`.
#[derive(Default)]
struct Queue { games: Vec<Game>, on_screen: Vec<PathBuf>, done: HashSet<PathBuf>, paused: bool }

impl Queue {
    fn apply(&mut self, cmd: ArtCmd) {
        match cmd {
            // Each scan starts over; games that have a picture are skipped quickly.
            ArtCmd::Library(g) => { self.games = g; self.done.clear(); }
            ArtCmd::OnScreen(p) => self.on_screen = p,
            ArtCmd::Paused(p) => self.paused = p,
        }
    }

    /// The next game to work on: those on screen first, then the library in order.
    fn next(&self) -> Option<Game> {
        if self.paused { return None; }
        self.on_screen.iter().filter_map(|p| self.games.iter().find(|g| &g.path == p))
            .chain(&self.games).find(|g| !self.done.contains(&g.path)).cloned()
    }
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<ArtCmd>, stored: impl Fn(PathBuf)) {
        let mut queue = Queue::default();
        let mut retry_at = None::<Instant>;
        loop {
            let waiting = retry_at.is_some_and(|t| Instant::now() < t);
            let Some(game) = queue.next().filter(|_| !waiting) else {
                // Nothing to do until told something, or until it's time to try again (never
                // while paused). The tables are kept while a retry is due.
                self.save();
                if retry_at.is_none() { self.tables.clear(); }
                let cmd = match retry_at.filter(|_| !queue.paused) {
                    Some(t) => rx.recv_timeout(t.saturating_duration_since(Instant::now())),
                    None => rx.recv().map_err(|_| mpsc::RecvTimeoutError::Disconnected),
                };
                match cmd {
                    Ok(cmd) => queue.apply(cmd),
                    Err(mpsc::RecvTimeoutError::Timeout) => retry_at = None,
                    Err(mpsc::RecvTimeoutError::Disconnected) => return,
                }
                continue;
            };
            match self.process(&game) {
                Outcome::Stored => { queue.done.insert(game.path.clone()); stored(game.path); }
                Outcome::Nothing => { queue.done.insert(game.path); }
                Outcome::Later => retry_at = Some(Instant::now() + RETRY_LATER),
            }
            for cmd in rx.try_iter() { queue.apply(cmd); }
        }
    }

    fn save(&mut self) {
        for (_, index) in self.tables.values_mut() {
            if let Err(e) = index.save() { eprintln!("art: saving the index: {e}"); }
        }
    }

    /// Finds and stores art for `game` unless it has a picture, identifying the ROM first if its
    /// index entry is missing or out of date.
    fn process(&mut self, game: &Game) -> Outcome {
        if art::user_image(&self.roms, &game.path).exists() || art::cached_image(&self.cache, &game.path).exists() { return Outcome::Nothing; }
        let Some(system) = self.systems.iter().find(|s| s.id == game.system).cloned() else { return Outcome::Nothing };
        let rom = self.roms.join(&game.path);
        let Ok(meta) = std::fs::metadata(&rom) else { return Outcome::Nothing };
        let modified = meta.modified().map_or(0, unix_secs);
        let cache = &self.cache;
        let (db, index) = self.tables.entry(system.id).or_insert_with(|| {
            let db = GameDb::parse(system.game_db);
            let index = ArtIndex::load(art::index_path(cache, system.id), &db.version);
            (db, index)
        });
        let mut entry = index.get(&game.path, meta.len(), modified).unwrap_or_else(|| {
            let identity = art::identify(&rom, &system, db).unwrap_or_else(|e| { eprintln!("art: {}: {e}", rom.display()); None });
            let e = Entry { size: meta.len(), modified, identity, missing_since: None };
            index.set(&game.path, &e);
            e
        });
        let now = unix_secs(SystemTime::now());
        if entry.missing_since.is_some_and(|t| now.saturating_sub(t) < RETRY_MISSING.as_secs()) { return Outcome::Nothing; }
        for source in &self.sources {
            match source.find(game, entry.identity.as_ref(), &system) {
                Ok(Some(bytes)) => {
                    // An image that doesn't decode counts as none; a failed write is retried.
                    let png = match decode(&bytes).map(|p| encode(&shrink(p, IMAGE_SIZE))) {
                        Ok(Ok(png)) => png,
                        e => { eprintln!("art: {}: {}: not a usable image: {:?}", source.name(), game.path.display(), e.err()); continue; }
                    };
                    return match write(&png, &art::cached_image(cache, &game.path)) {
                        Ok(()) => Outcome::Stored,
                        Err(e) => { eprintln!("art: storing {}: {e}", game.path.display()); Outcome::Later }
                    };
                }
                Ok(None) => {}
                Err(ArtError::Offline(e)) => { eprintln!("art: {}: offline: {e}", source.name()); return Outcome::Later; }
                Err(ArtError::Failed(e)) => eprintln!("art: {}: {e}", source.name()),
            }
        }
        entry.missing_since = Some(now);
        index.set(&game.path, &entry);
        Outcome::Nothing
    }
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

/// Starts the loader. Send it the ROM paths whose pictures are wanted, most wanted first; each
/// new list replaces the last. For each it calls `loaded` (on its own thread) with the picture,
/// the user's own else the cached one, shrunk to `IMAGE_SIZE` (the user's may be any size); or
/// None if the game has none yet.
pub fn loader(roms: PathBuf, cache: PathBuf, loaded: impl Fn(PathBuf, Option<SharedPixelBuffer<Rgba8Pixel>>) + Send + 'static)
    -> mpsc::Sender<Vec<PathBuf>> {
    let (tx, rx) = mpsc::channel::<Vec<PathBuf>>();
    std::thread::spawn(move || {
        let mut wanted = Vec::new();
        loop {
            if wanted.is_empty() { match rx.recv() { Ok(w) => wanted = w, Err(_) => return } }
            if let Some(w) = rx.try_iter().last() { wanted = w; }
            if wanted.is_empty() { continue; }
            let rom = wanted.remove(0);
            let cached = art::cached_image(&cache, &rom);
            let picture = [art::user_image(&roms, &rom), cached.clone()].into_iter().find_map(|path| {
                let bytes = std::fs::read(&path).ok()?;
                match decode(&bytes) {
                    Ok(p) => Some(shrink(p, IMAGE_SIZE)),
                    Err(e) => {
                        eprintln!("art: {}: {e}", path.display());
                        // A broken cached picture is fetched again next time.
                        if path == cached { let _ = std::fs::remove_file(&path); }
                        None
                    }
                }
            });
            loaded(rom, picture.map(|p| SharedPixelBuffer::clone_from_slice(&p.rgba, p.width, p.height)));
        }
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
