//! Image viewer ("lightbox") state: a gallery of the currently viewed mod's
//! cover plus screenshots, with the selected index.

use std::path::PathBuf;

/// One image in the gallery.
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
}

impl Lightbox {
    pub fn new(items: Vec<GalleryItem>, index: usize) -> Self {
        let index = index.min(items.len().saturating_sub(1));
        Self { items, index }
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
