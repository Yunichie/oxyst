#![forbid(unsafe_code)]

mod export;

pub use export::{ExportFormat, export};

use std::{
    collections::HashMap,
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    sync::Arc,
};

use image::{DynamicImage, RgbaImage};
use thiserror::Error;
use typst_render::RenderOptions;
use typst_tui_compiler::CompiledDocument;

const MAX_TARGET_WIDTH: u32 = 2_048;
const MAX_DIMENSION: f64 = 16_384.0;
const MAX_TOTAL_PIXELS: f64 = 64.0 * 1_024.0 * 1_024.0;
const MIN_PIXELS_PER_POINT: f64 = 0.25;
const MAX_PIXELS_PER_POINT: f64 = 4.0;

#[derive(Debug, Error)]
pub enum Error {
    #[error("preview width must be between 1 and {MAX_TARGET_WIDTH} pixels")]
    InvalidTargetWidth,
    #[error("compiled document has no pages")]
    EmptyDocument,
    #[error("page {page} exceeds the preview dimension limit")]
    PageTooLarge { page: usize },
    #[error("document exceeds the preview pixel budget")]
    DocumentTooLarge,
    #[error("renderer returned invalid pixel data for page {page}")]
    InvalidPixelData { page: usize },
    #[error("preview rendering was cancelled")]
    Cancelled,
}

pub struct RenderedDocument {
    pages: Vec<PageImage>,
    pixels_per_point: f64,
}

#[derive(Clone, Default)]
pub struct RenderCache {
    pages: HashMap<PageCacheKey, Arc<RgbaImage>>,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct PageCacheKey {
    page: u64,
    pixels_per_point: u64,
}

impl RenderedDocument {
    #[must_use]
    pub fn pages(&self) -> &[PageImage] {
        &self.pages
    }

    #[must_use]
    pub fn into_pages(self) -> Vec<PageImage> {
        self.pages
    }

    #[must_use]
    pub fn pixels_per_point(&self) -> f64 {
        self.pixels_per_point
    }
}

#[derive(Clone)]
pub struct PageImage {
    image: Arc<RgbaImage>,
}

impl PageImage {
    #[must_use]
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    #[must_use]
    pub fn height(&self) -> u32 {
        self.image.height()
    }

    #[must_use]
    pub fn rgba(&self) -> &[u8] {
        self.image.as_raw()
    }

    #[must_use]
    pub fn into_image(self) -> DynamicImage {
        let image = Arc::try_unwrap(self.image).unwrap_or_else(|image| (*image).clone());
        DynamicImage::ImageRgba8(image)
    }
}

pub fn render(document: &CompiledDocument, target_width: u32) -> Result<RenderedDocument, Error> {
    render_cancellable(document, target_width, || false)
}

pub fn render_cancellable(
    document: &CompiledDocument,
    target_width: u32,
    cancelled: impl Fn() -> bool,
) -> Result<RenderedDocument, Error> {
    render_cached_cancellable(document, target_width, &RenderCache::default(), cancelled)
        .map(|(rendered, _)| rendered)
}

pub fn render_cached_cancellable(
    document: &CompiledDocument,
    target_width: u32,
    cache: &RenderCache,
    cancelled: impl Fn() -> bool,
) -> Result<(RenderedDocument, RenderCache), Error> {
    if target_width == 0 || target_width > MAX_TARGET_WIDTH {
        return Err(Error::InvalidTargetWidth);
    }
    if document.pages().is_empty() {
        return Err(Error::EmptyDocument);
    }

    let widest_page = document
        .pages()
        .iter()
        .map(|page| page.frame.size().x.to_pt())
        .fold(0.0_f64, f64::max);
    let pixels_per_point =
        (f64::from(target_width) / widest_page).clamp(MIN_PIXELS_PER_POINT, MAX_PIXELS_PER_POINT);
    let (pages, cache) =
        render_at_cached_cancellable(document, pixels_per_point, cache, &cancelled)?;

    Ok((
        RenderedDocument {
            pages,
            pixels_per_point,
        },
        cache,
    ))
}

fn render_at(document: &CompiledDocument, pixels_per_point: f64) -> Result<Vec<PageImage>, Error> {
    render_at_cached_cancellable(document, pixels_per_point, &RenderCache::default(), &|| {
        false
    })
    .map(|(pages, _)| pages)
}

fn render_at_cached_cancellable(
    document: &CompiledDocument,
    pixels_per_point: f64,
    cache: &RenderCache,
    cancelled: &impl Fn() -> bool,
) -> Result<(Vec<PageImage>, RenderCache), Error> {
    if document.pages().is_empty() {
        return Err(Error::EmptyDocument);
    }
    validate_dimensions(document, pixels_per_point)?;
    let options = RenderOptions {
        pixel_per_pt: pixels_per_point.into(),
        render_bleed: false,
    };
    let mut pages = Vec::with_capacity(document.pages().len());
    let mut next_cache = RenderCache::default();
    for (index, page) in document.pages().iter().enumerate() {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let key = PageCacheKey {
            page: page_hash(page),
            pixels_per_point: pixels_per_point.to_bits(),
        };
        if let Some(image) = cache.pages.get(&key) {
            next_cache.pages.insert(key, Arc::clone(image));
            pages.push(PageImage {
                image: Arc::clone(image),
            });
            continue;
        }
        let pixmap = typst_render::render(page, &options);
        let (width, height) = (pixmap.width(), pixmap.height());
        let mut rgba = pixmap.data().to_vec();
        unpremultiply(&mut rgba);
        let image = RgbaImage::from_raw(width, height, rgba)
            .ok_or(Error::InvalidPixelData { page: index + 1 })?;
        let image = Arc::new(image);
        next_cache.pages.insert(key, Arc::clone(&image));
        pages.push(PageImage { image });
    }
    Ok((pages, next_cache))
}

fn page_hash(page: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    page.hash(&mut hasher);
    hasher.finish()
}

fn validate_dimensions(document: &CompiledDocument, pixels_per_point: f64) -> Result<(), Error> {
    let mut total_pixels = 0.0;
    for (index, page) in document.pages().iter().enumerate() {
        let size = page.frame.size();
        let width = (size.x.to_pt() * pixels_per_point).round().max(1.0);
        let height = (size.y.to_pt() * pixels_per_point).round().max(1.0);
        if !width.is_finite()
            || !height.is_finite()
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
        {
            return Err(Error::PageTooLarge { page: index + 1 });
        }
        total_pixels += width * height;
        if total_pixels > MAX_TOTAL_PIXELS {
            return Err(Error::DocumentTooLarge);
        }
    }
    Ok(())
}

fn unpremultiply(rgba: &mut [u8]) {
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha == 0 || alpha == 255 {
            continue;
        }

        for channel in &mut pixel[..3] {
            *channel = ((u32::from(*channel) * 255 + alpha / 2) / alpha).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{error::Error, path::PathBuf, sync::Arc};

    use typst_tui_compiler::{CompileOutcome, Compiler};

    use super::{RenderCache, render_cached_cancellable, unpremultiply};

    #[test]
    fn converts_premultiplied_alpha() {
        let mut pixel = [64, 32, 16, 128];
        unpremultiply(&mut pixel);
        assert_eq!(pixel, [128, 64, 32, 128]);
    }

    #[test]
    fn reuses_unchanged_page_images_at_the_same_scale() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let CompileOutcome::Success(first_document) =
            compiler.compile("= Stable page\n#pagebreak()\n= Before")
        else {
            return Err("fixture did not compile".into());
        };
        let (first, cache) =
            render_cached_cancellable(&first_document, 400, &RenderCache::default(), || false)?;

        let CompileOutcome::Success(second_document) =
            compiler.compile("= Stable page\n#pagebreak()\n= After")
        else {
            return Err("updated fixture did not compile".into());
        };
        let (second, _) = render_cached_cancellable(&second_document, 400, &cache, || false)?;

        assert_eq!(first.pages.len(), 2);
        assert!(Arc::ptr_eq(&first.pages[0].image, &second.pages[0].image));
        assert!(!Arc::ptr_eq(&first.pages[1].image, &second.pages[1].image));
        Ok(())
    }
}
