//! Shared LRU cache of image textures (mod covers, screenshots, upper files…).
//!
//! Image paths come from `mod.toml` or from the filesystem and are not trusted,
//! so decoding is bounded to avoid decompression bombs exhausting memory.

use eframe::egui;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;

/// Maximum decoded side (px) and allocation (bytes) per image.
const MAX_IMAGE_DIM: u32 = 8192;
const MAX_IMAGE_ALLOC: u64 = 64 * 1024 * 1024;

/// Maximum number of textures kept alive at once (LRU eviction).
const CAPACITY: usize = 128;

/// LRU texture cache keyed by an arbitrary (namespaced) string.
#[derive(Default)]
pub struct ThumbCache {
    textures: HashMap<String, egui::TextureHandle>,
    order: VecDeque<String>,
    missing: HashSet<String>,
}

impl ThumbCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the texture for `path`, decoding and caching it on first use.
    /// Keys that failed to decode are remembered so a broken image is not
    /// retried every frame.
    pub fn get(
        &mut self,
        ctx: &egui::Context,
        key: &str,
        path: &Path,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(key) {
            return Some(t.clone());
        }
        if self.missing.contains(key) {
            return None;
        }
        let decoded = image::ImageReader::open(path).ok().and_then(|mut reader| {
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(MAX_IMAGE_DIM);
            limits.max_image_height = Some(MAX_IMAGE_DIM);
            limits.max_alloc = Some(MAX_IMAGE_ALLOC);
            reader.limits(limits);
            reader.decode().ok()
        });
        let Some(img) = decoded else {
            self.missing.insert(key.to_string());
            return None;
        };
        let img = img.to_rgba8();
        let (w, h) = (img.width(), img.height());
        let color =
            egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
        let tex = ctx.load_texture(key, color, egui::TextureOptions::LINEAR);

        if self.textures.len() >= CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.textures.remove(&old);
            }
        }
        self.textures.insert(key.to_string(), tex.clone());
        self.order.retain(|k| k != key);
        self.order.push_back(key.to_string());
        Some(tex)
    }
}
