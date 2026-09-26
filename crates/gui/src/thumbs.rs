//! Shared asynchronous cache of image textures (mod covers, screenshots, upper
//! files…).
//!
//! Image paths come from `mod.toml` or from the filesystem and are not trusted,
//! so decoding is bounded to avoid decompression bombs exhausting memory.
//!
//! Decoding happens on a small worker pool and is downscaled to a bounded
//! thumbnail size, so opening a folder with many images never blocks the UI.

use eframe::egui;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet, VecDeque};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Maximum decoded side (px) and allocation (bytes) per image.
const MAX_IMAGE_DIM: u32 = 8192;
const MAX_IMAGE_ALLOC: u64 = 64 * 1024 * 1024;

/// Maximum number of textures kept alive at once (LRU eviction).
const CAPACITY: usize = 128;

/// Textures uploaded to the GPU per frame, to keep frames smooth while a large
/// folder is being decoded.
const MAX_UPLOADS_PER_FRAME: usize = 12;

/// On-disk thumbnail cache limits (pruned at most once per process).
const DISK_CAP: usize = 5000;
const DISK_MAX_AGE: Duration = Duration::from_secs(60 * 60 * 24 * 60);
static DISK_PRUNED: AtomicBool = AtomicBool::new(false);

/// Thumbnail size class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ThumbSize {
    /// Grid/list/gallery previews.
    Small,
    /// Full-screen image viewer.
    Full,
}

impl ThumbSize {
    fn max_side(self) -> u32 {
        match self {
            ThumbSize::Small => 256,
            ThumbSize::Full => 2048,
        }
    }

    fn tag(self) -> &'static str {
        match self {
            ThumbSize::Small => "s",
            ThumbSize::Full => "f",
        }
    }
}

/// State of a requested thumbnail.
pub enum ThumbState {
    Ready(egui::TextureHandle),
    Loading,
    Failed,
}

/// A decoded thumbnail going back to the UI thread.
struct ThumbResult {
    key: String,
    path: PathBuf,
    size: ThumbSize,
    image: egui::ColorImage,
    /// Original image dimensions (before downscaling).
    dims: (u32, u32),
}

/// A decode request.
#[derive(Clone)]
struct Job {
    key: String,
    path: PathBuf,
    size: ThumbSize,
}

/// Worker -> UI message (decoded thumbnail, or the failed key).
type ThumbMsg = Result<ThumbResult, String>;

/// Shared job queue + shutdown flag for the worker pool.
struct Shared {
    queue: Mutex<VecDeque<Job>>,
    cv: Condvar,
    shutdown: AtomicBool,
}

impl Shared {
    fn push(&self, job: Job) {
        self.queue.lock().unwrap().push_back(job);
        self.cv.notify_one();
    }

    fn pop(&self) -> Option<Job> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if self.shutdown.load(Ordering::SeqCst) {
                return None;
            }
            if let Some(job) = q.pop_front() {
                return Some(job);
            }
            q = self.cv.wait(q).unwrap();
        }
    }
}

/// Decodes an image and downscales it to the given size class, returning the
/// scaled image and the original dimensions.
fn decode_scaled(path: &Path, size: ThumbSize) -> Option<(image::DynamicImage, (u32, u32))> {
    let mut reader = image::ImageReader::open(path).ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIM);
    limits.max_image_height = Some(MAX_IMAGE_DIM);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC);
    reader.limits(limits);
    let img = reader.decode().ok()?;
    let dims = (img.width(), img.height());
    let max_side = size.max_side();
    Some((img.thumbnail(max_side, max_side), dims))
}

fn to_color_image(img: &image::DynamicImage) -> egui::ColorImage {
    let rgba = img.to_rgba8();
    egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    )
}

/// Path (without extension) of the on-disk cache file for a thumbnail key.
fn cache_stem(dir: &Path, key: &str) -> PathBuf {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    dir.join(format!("{:016x}", hasher.finish()))
}

/// Loads a cached thumbnail from disk, if present.
fn load_cached(dir: &Path, key: &str) -> Option<egui::ColorImage> {
    let stem = cache_stem(dir, key);
    for ext in ["png", "jpg"] {
        let path = stem.with_extension(ext);
        if let Ok(img) = image::open(&path) {
            return Some(to_color_image(&img));
        }
    }
    None
}

/// Stores a thumbnail on disk (PNG when it has alpha, JPEG otherwise).
fn store_cached(dir: &Path, key: &str, img: &image::DynamicImage) {
    let _ = std::fs::create_dir_all(dir);
    let stem = cache_stem(dir, key);
    if img.color().has_alpha() {
        let _ = img.to_rgba8().save(stem.with_extension("png"));
    } else {
        let _ = img.to_rgb8().save(stem.with_extension("jpg"));
    }
}

/// Best-effort prune of stale thumbnails (at most once per process).
fn maybe_prune(dir: &Path) {
    if DISK_PRUNED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    let mut files: Vec<(PathBuf, SystemTime)> = Vec::new();
    for e in entries.flatten() {
        let Ok(meta) = e.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(UNIX_EPOCH);
        if now
            .duration_since(mtime)
            .map(|d| d > DISK_MAX_AGE)
            .unwrap_or(false)
        {
            let _ = std::fs::remove_file(e.path());
            continue;
        }
        files.push((e.path(), mtime));
    }
    if files.len() > DISK_CAP {
        files.sort_by_key(|(_, t)| *t);
        for (path, _) in files.iter().take(files.len() - DISK_CAP) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Produces a thumbnail for a job, using (and filling) the on-disk cache for
/// small thumbnails.
fn produce(job: &Job, cache_dir: Option<&Path>) -> Option<(egui::ColorImage, (u32, u32))> {
    if job.size == ThumbSize::Small {
        if let Some(dir) = cache_dir {
            maybe_prune(dir);
            if let Some(color) = load_cached(dir, &job.key) {
                let dims = image::image_dimensions(&job.path)
                    .unwrap_or((color.size[0] as u32, color.size[1] as u32));
                return Some((color, dims));
            }
            let (img, dims) = decode_scaled(&job.path, job.size)?;
            store_cached(dir, &job.key, &img);
            return Some((to_color_image(&img), dims));
        }
    }
    let (img, dims) = decode_scaled(&job.path, job.size)?;
    Some((to_color_image(&img), dims))
}

/// LRU texture cache fed by a background worker pool.
pub struct ThumbCache {
    textures: HashMap<String, egui::TextureHandle>,
    order: VecDeque<String>,
    pending: HashSet<String>,
    failed: HashSet<String>,
    /// Original dimensions keyed by `(path, size)` once decoded.
    dims: HashMap<(PathBuf, ThumbSize), (u32, u32)>,
    shared: Option<Arc<Shared>>,
    workers: Vec<JoinHandle<()>>,
    tx: Option<Sender<ThumbMsg>>,
    rx: Receiver<ThumbMsg>,
}

impl Default for ThumbCache {
    fn default() -> Self {
        Self::new()
    }
}

impl ThumbCache {
    pub fn new() -> Self {
        let (_tx, rx) = channel();
        Self {
            textures: HashMap::new(),
            order: VecDeque::new(),
            pending: HashSet::new(),
            failed: HashSet::new(),
            dims: HashMap::new(),
            shared: None,
            workers: Vec::new(),
            tx: None,
            rx,
        }
    }

    /// Lazily starts the worker pool on first use.
    fn ensure_pool(&mut self) {
        if self.shared.is_some() {
            return;
        }
        let (tx, rx) = channel();
        self.rx = rx;
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            cv: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let workers = std::thread::available_parallelism()
            .map(|n| (n.get() / 2).clamp(1, 4))
            .unwrap_or(2);
        let cache_dir = gta_mo_core::config::cache_dir_path().map(|d| d.join("thumbs"));
        for _ in 0..workers {
            let shared = shared.clone();
            let tx = tx.clone();
            let cache_dir = cache_dir.clone();
            self.workers.push(thread::spawn(move || {
                while let Some(job) = shared.pop() {
                    let result = match produce(&job, cache_dir.as_deref()) {
                        Some((image, dims)) => Ok(ThumbResult {
                            key: job.key,
                            path: job.path,
                            size: job.size,
                            image,
                            dims,
                        }),
                        None => Err(job.key),
                    };
                    if tx.send(result).is_err() {
                        return;
                    }
                }
            }));
        }
        self.tx = Some(tx);
        self.shared = Some(shared);
    }

    fn cache_key(key_base: &str, path: &Path, size: ThumbSize) -> Option<String> {
        let meta = std::fs::metadata(path).ok()?;
        let len = meta.len();
        let mtime = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Some(format!("{key_base}#{}#{mtime}#{len}", size.tag()))
    }

    fn insert_texture(&mut self, key: String, tex: egui::TextureHandle) {
        if self.textures.len() >= CAPACITY && !self.textures.contains_key(&key) {
            if let Some(old) = self.order.pop_front() {
                self.textures.remove(&old);
            }
        }
        self.textures.insert(key.clone(), tex);
        self.order.retain(|k| k != &key);
        self.order.push_back(key);
    }

    /// Returns the texture for `path`, enqueueing an async decode when needed.
    /// `key_base` is a caller-provided namespace (e.g. `folder/cover.png`).
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        key_base: &str,
        path: &Path,
        size: ThumbSize,
    ) -> ThumbState {
        let Some(key) = Self::cache_key(key_base, path, size) else {
            return ThumbState::Failed;
        };
        if let Some(t) = self.textures.get(&key) {
            return ThumbState::Ready(t.clone());
        }
        if self.failed.contains(&key) {
            return ThumbState::Failed;
        }
        if self.pending.contains(&key) {
            return ThumbState::Loading;
        }
        self.ensure_pool();
        self.pending.insert(key.clone());
        if let Some(shared) = &self.shared {
            shared.push(Job {
                key,
                path: path.to_path_buf(),
                size,
            });
            ctx.request_repaint();
        }
        ThumbState::Loading
    }

    /// Uploads finished textures (bounded per frame). Call once per frame.
    pub fn poll(&mut self, ctx: &egui::Context) {
        let mut uploaded = 0usize;
        while uploaded < MAX_UPLOADS_PER_FRAME {
            match self.rx.try_recv() {
                Ok(Ok(res)) => {
                    self.pending.remove(&res.key);
                    self.dims.insert((res.path, res.size), res.dims);
                    let tex = ctx.load_texture(&res.key, res.image, egui::TextureOptions::LINEAR);
                    self.insert_texture(res.key, tex);
                    uploaded += 1;
                }
                Ok(Err(key)) => {
                    self.pending.remove(&key);
                    self.failed.insert(key);
                }
                Err(TryRecvError::Empty) | Err(TryRecvError::Disconnected) => break,
            }
        }
        if !self.pending.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Original dimensions of an image, once decoded.
    pub fn dimensions(&self, path: &Path, size: ThumbSize) -> Option<(u32, u32)> {
        self.dims.get(&(path.to_path_buf(), size)).copied()
    }
}

impl Drop for ThumbCache {
    fn drop(&mut self) {
        if let Some(shared) = &self.shared {
            shared.shutdown.store(true, Ordering::SeqCst);
            shared.cv.notify_all();
        }
        // Drop the sender so workers blocked on `send` can finish.
        self.tx.take();
        for w in self.workers.drain(..) {
            let _ = w.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_changes_with_metadata() {
        let dir = std::env::temp_dir().join(format!("gta-mo-thumb-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.bin");
        std::fs::write(&path, b"hello").unwrap();
        let k1 = ThumbCache::cache_key("x", &path, ThumbSize::Small).unwrap();
        // Different size class -> different key.
        assert_ne!(
            k1,
            ThumbCache::cache_key("x", &path, ThumbSize::Full).unwrap()
        );
        // Modifying the file changes the key (mtime/len).
        std::fs::write(&path, b"hello world").unwrap();
        let k2 = ThumbCache::cache_key("x", &path, ThumbSize::Small).unwrap();
        assert_ne!(k1, k2);
        // Missing file -> None.
        assert!(ThumbCache::cache_key("x", &dir.join("nope"), ThumbSize::Small).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_is_failed() {
        let ctx = egui::Context::default();
        let mut cache = ThumbCache::new();
        let path = std::path::PathBuf::from("/tmp/gta-mo-does-not-exist-xyz");
        assert!(matches!(
            cache.get(&ctx, "k", &path, ThumbSize::Small),
            ThumbState::Failed
        ));
    }

    #[test]
    fn disk_cache_roundtrip() {
        let dir = std::env::temp_dir().join(format!("gta-mo-diskcache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("src.png");
        image::RgbaImage::from_pixel(8, 6, image::Rgba([10, 20, 30, 255]))
            .save(&src)
            .unwrap();
        let cache_dir = dir.join("cache");
        let job = Job {
            key: "k1".into(),
            path: src.clone(),
            size: ThumbSize::Small,
        };
        let first = produce(&job, Some(&cache_dir)).unwrap();
        assert_eq!(first.1, (8, 6));
        assert!(cache_stem(&cache_dir, "k1").with_extension("png").is_file());

        // A second request is served from disk even if the source disappears.
        std::fs::remove_file(&src).unwrap();
        assert!(load_cached(&cache_dir, "k1").is_some());
        assert!(produce(&job, Some(&cache_dir)).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
