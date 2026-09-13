//! Image viewer ("lightbox") state: a gallery of the currently viewed mod's
//! cover plus screenshots, with the selected index.

use eframe::egui;
use std::path::PathBuf;

/// One image in the gallery.
#[derive(Clone)]
pub struct GalleryItem {
    /// Cache key (also used as the egui texture name).
    pub key: String,
    pub path: PathBuf,
    pub label: String,
}

/// The open lightbox. Rendered by `GtaMoApp::ui_lightbox`.
pub struct Lightbox {
    pub items: Vec<GalleryItem>,
    pub index: usize,
    /// Screen rect the image was opened from, for the shared-element ("hero")
    /// transition.
    pub source: Option<egui::Rect>,
    /// When the viewer was opened (drives the hero animation).
    pub started: std::time::Instant,
}

impl Lightbox {
    pub fn new(items: Vec<GalleryItem>, index: usize) -> Self {
        let index = index.min(items.len().saturating_sub(1));
        Self {
            items,
            index,
            source: None,
            started: std::time::Instant::now(),
        }
    }

    /// Same as [`Lightbox::new`], remembering the source rect to animate from.
    pub fn from_rect(items: Vec<GalleryItem>, index: usize, source: egui::Rect) -> Self {
        Self {
            source: Some(source),
            ..Self::new(items, index)
        }
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: usize) -> GalleryItem {
        GalleryItem {
            key: format!("k{n}"),
            path: PathBuf::from(format!("/tmp/{n}.png")),
            label: format!("img {n}"),
        }
    }

    #[test]
    fn new_clamps_index() {
        let lb = Lightbox::new(vec![item(0), item(1)], 5);
        assert_eq!(lb.index, 1);
        assert_eq!(lb.len(), 2);

        let empty = Lightbox::new(vec![], 3);
        assert_eq!(empty.len(), 0);
    }
}
