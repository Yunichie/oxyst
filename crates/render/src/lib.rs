#![forbid(unsafe_code)]

mod export;

pub use export::{ExportFormat, export};

use std::{
    collections::hash_map::DefaultHasher,
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
    sync::Arc,
};

use image::{DynamicImage, RgbaImage};
use thiserror::Error;
use typst_render::RenderOptions;
use typst_tui_compiler::CompiledDocument;

const MAX_TARGET_WIDTH: u32 = 2_048;
const MAX_DIMENSION: f64 = 16_384.0;
const MAX_TOTAL_PIXELS: u64 = 64 * 1_024 * 1_024;
const MIN_PIXELS_PER_POINT: f64 = 0.25;
const MAX_PIXELS_PER_POINT: f64 = 4.0;
const MAX_CACHED_PAGES: usize = 5;
const MAX_CACHE_BYTES: usize = 64 * 1024 * 1024;

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
    #[error("preview page {page} does not exist")]
    PageOutOfBounds { page: usize },
    #[error("preview manifest does not match the compiled document")]
    ManifestMismatch,
    #[error("renderer returned invalid pixel data for page {page}")]
    InvalidPixelData { page: usize },
    #[error("preview rendering was cancelled")]
    Cancelled,
    #[error("could not export PDF: {0}")]
    Pdf(String),
    #[error("could not encode PNG")]
    EncodePng(#[from] image::ImageError),
    #[error("exported PNG dimensions are too large")]
    PngTooLarge,
    #[error("could not write export to {path}")]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
}

pub struct RenderedDocument {
    pages: Vec<PageImage>,
    pixels_per_point: f64,
}

#[derive(Clone, Debug)]
pub struct RenderManifest {
    pages: Vec<PageMetadata>,
    pixels_per_point: f64,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PageFingerprint(u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PageMetadata {
    width: u32,
    height: u32,
    fingerprint: PageFingerprint,
}

#[derive(Clone, Default)]
pub struct RenderCache {
    pages: HashMap<PageCacheKey, Arc<RgbaImage>>,
    order: VecDeque<PageCacheKey>,
    bytes: usize,
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct PageCacheKey {
    page: PageFingerprint,
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

impl RenderManifest {
    #[must_use]
    pub fn pages(&self) -> &[PageMetadata] {
        &self.pages
    }

    #[must_use]
    pub fn pixels_per_point(&self) -> f64 {
        self.pixels_per_point
    }
}

impl PageMetadata {
    #[must_use]
    pub fn width(self) -> u32 {
        self.width
    }

    #[must_use]
    pub fn height(self) -> u32 {
        self.height
    }

    #[must_use]
    pub fn fingerprint(self) -> PageFingerprint {
        self.fingerprint
    }
}

impl PageFingerprint {
    #[must_use]
    pub fn value(self) -> u64 {
        self.0
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

pub struct RenderedPage {
    index: usize,
    image: PageImage,
}

impl RenderedPage {
    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub fn into_image(self) -> PageImage {
        self.image
    }
}

impl RenderCache {
    fn get(&mut self, key: PageCacheKey) -> Option<Arc<RgbaImage>> {
        let image = Arc::clone(self.pages.get(&key)?);
        self.order.retain(|candidate| *candidate != key);
        self.order.push_back(key);
        Some(image)
    }

    fn insert(&mut self, key: PageCacheKey, image: Arc<RgbaImage>) {
        if let Some(previous) = self.pages.insert(key, Arc::clone(&image)) {
            self.bytes = self.bytes.saturating_sub(image_bytes(&previous));
        }
        self.bytes = self.bytes.saturating_add(image_bytes(&image));
        self.order.retain(|candidate| *candidate != key);
        self.order.push_back(key);

        while self.pages.len() > 1
            && (self.pages.len() > MAX_CACHED_PAGES || self.bytes > MAX_CACHE_BYTES)
        {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.pages.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(image_bytes(&removed));
            }
        }
    }
}

fn image_bytes(image: &RgbaImage) -> usize {
    usize::try_from(image.width())
        .unwrap_or(usize::MAX)
        .saturating_mul(usize::try_from(image.height()).unwrap_or(usize::MAX))
        .saturating_mul(4)
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
    let manifest = render_manifest(document, target_width)?;
    validate_total_pixels(&manifest)?;
    let indices = 0..manifest.pages.len();
    let (pages, cache) =
        render_pages_cached_cancellable(document, &manifest, indices, cache, cancelled)?;

    Ok((
        RenderedDocument {
            pages: pages.into_iter().map(RenderedPage::into_image).collect(),
            pixels_per_point: manifest.pixels_per_point,
        },
        cache,
    ))
}

pub fn render_manifest(
    document: &CompiledDocument,
    target_width: u32,
) -> Result<RenderManifest, Error> {
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
    manifest_at(document, pixels_per_point)
}

pub fn render_pages_cached_cancellable(
    document: &CompiledDocument,
    manifest: &RenderManifest,
    indices: impl IntoIterator<Item = usize>,
    cache: &RenderCache,
    cancelled: impl Fn() -> bool,
) -> Result<(Vec<RenderedPage>, RenderCache), Error> {
    if manifest.pages.len() != document.pages().len() {
        return Err(Error::ManifestMismatch);
    }
    let options = RenderOptions {
        pixel_per_pt: manifest.pixels_per_point.into(),
        render_bleed: false,
    };
    let mut pages = Vec::new();
    let mut next_cache = cache.clone();

    for index in indices {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let page = document
            .pages()
            .get(index)
            .ok_or(Error::PageOutOfBounds { page: index + 1 })?;
        let metadata = manifest
            .pages
            .get(index)
            .ok_or(Error::PageOutOfBounds { page: index + 1 })?;
        validate_page_fingerprint(page, *metadata)?;
        let key = PageCacheKey {
            page: metadata.fingerprint,
            pixels_per_point: manifest.pixels_per_point.to_bits(),
        };
        let image = if let Some(image) = next_cache.get(key) {
            image
        } else {
            let pixmap = typst_render::render(page, &options);
            let image = Arc::new(rendered_image(
                pixmap.width(),
                pixmap.height(),
                pixmap.data(),
                index,
            )?);
            next_cache.insert(key, Arc::clone(&image));
            image
        };
        pages.push(RenderedPage {
            index,
            image: PageImage { image },
        });
    }

    Ok((pages, next_cache))
}

pub fn render_pages_cancellable(
    document: &CompiledDocument,
    manifest: &RenderManifest,
    indices: impl IntoIterator<Item = usize>,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<RenderedPage>, Error> {
    if manifest.pages.len() != document.pages().len() {
        return Err(Error::ManifestMismatch);
    }
    let options = RenderOptions {
        pixel_per_pt: manifest.pixels_per_point.into(),
        render_bleed: false,
    };
    let mut pages = Vec::new();

    for index in indices {
        if cancelled() {
            return Err(Error::Cancelled);
        }
        let page = document
            .pages()
            .get(index)
            .ok_or(Error::PageOutOfBounds { page: index + 1 })?;
        let metadata = manifest
            .pages
            .get(index)
            .ok_or(Error::PageOutOfBounds { page: index + 1 })?;
        validate_page_fingerprint(page, *metadata)?;
        let pixmap = typst_render::render(page, &options);
        pages.push(RenderedPage {
            index,
            image: PageImage {
                image: Arc::new(rendered_image(
                    pixmap.width(),
                    pixmap.height(),
                    pixmap.data(),
                    index,
                )?),
            },
        });
    }

    Ok(pages)
}

fn render_at(document: &CompiledDocument, pixels_per_point: f64) -> Result<Vec<PageImage>, Error> {
    let manifest = manifest_at(document, pixels_per_point)?;
    validate_total_pixels(&manifest)?;
    render_pages_cached_cancellable(
        document,
        &manifest,
        0..manifest.pages.len(),
        &RenderCache::default(),
        || false,
    )
    .map(|(pages, _)| pages.into_iter().map(RenderedPage::into_image).collect())
}

fn manifest_at(
    document: &CompiledDocument,
    pixels_per_point: f64,
) -> Result<RenderManifest, Error> {
    if document.pages().is_empty() {
        return Err(Error::EmptyDocument);
    }
    let mut pages = Vec::with_capacity(document.pages().len());
    for (index, page) in document.pages().iter().enumerate() {
        let size = page.frame.size();
        let width = (size.x.to_pt() * pixels_per_point).round().max(1.0);
        let height = (size.y.to_pt() * pixels_per_point).round().max(1.0);
        if !width.is_finite()
            || !height.is_finite()
            || width > MAX_DIMENSION
            || height > MAX_DIMENSION
            || width * height > MAX_TOTAL_PIXELS as f64
        {
            return Err(Error::PageTooLarge { page: index + 1 });
        }
        pages.push(PageMetadata {
            width: width as u32,
            height: height as u32,
            fingerprint: PageFingerprint(page_hash(page)),
        });
    }
    Ok(RenderManifest {
        pages,
        pixels_per_point,
    })
}

fn page_hash(page: &impl Hash) -> u64 {
    let mut hasher = DefaultHasher::new();
    page.hash(&mut hasher);
    hasher.finish()
}

fn validate_page_fingerprint(page: &impl Hash, metadata: PageMetadata) -> Result<(), Error> {
    (PageFingerprint(page_hash(page)) == metadata.fingerprint)
        .then_some(())
        .ok_or(Error::ManifestMismatch)
}

fn rendered_image(width: u32, height: u32, data: &[u8], index: usize) -> Result<RgbaImage, Error> {
    let mut rgba = data.to_vec();
    unpremultiply(&mut rgba);
    RgbaImage::from_raw(width, height, rgba).ok_or(Error::InvalidPixelData { page: index + 1 })
}

fn validate_total_pixels(manifest: &RenderManifest) -> Result<(), Error> {
    let mut total_pixels = 0_u64;
    for page in &manifest.pages {
        total_pixels = total_pixels
            .saturating_add(u64::from(page.width).saturating_mul(u64::from(page.height)));
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
    use std::{
        error::Error,
        fs,
        path::PathBuf,
        sync::Arc,
        time::{Instant, SystemTime, UNIX_EPOCH},
    };

    use image::RgbaImage;
    use typst_tui_compiler::{CompileOutcome, Compiler};

    use super::{
        MAX_CACHED_PAGES, RenderCache, render_cached_cancellable, render_manifest,
        render_pages_cached_cancellable, render_pages_cancellable, unpremultiply,
    };

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
        let first_manifest = render_manifest(&first_document, 400)?;
        let second_manifest = render_manifest(&second_document, 400)?;
        let (second, _) = render_cached_cancellable(&second_document, 400, &cache, || false)?;

        assert_eq!(first.pages.len(), 2);
        assert_eq!(
            first_manifest.pages[0].fingerprint,
            second_manifest.pages[0].fingerprint
        );
        assert_ne!(
            first_manifest.pages[1].fingerprint,
            second_manifest.pages[1].fingerprint
        );
        assert!(Arc::ptr_eq(&first.pages[0].image, &second.pages[0].image));
        assert!(!Arc::ptr_eq(&first.pages[1].image, &second.pages[1].image));
        Ok(())
    }

    #[test]
    fn uncached_pages_have_unique_pixel_ownership() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let CompileOutcome::Success(document) = compiler.compile("= Owned page") else {
            return Err("fixture did not compile".into());
        };
        let manifest = render_manifest(&document, 800)?;

        let pages = render_pages_cancellable(&document, &manifest, [0], || false)?;

        assert_eq!(pages.len(), 1);
        assert_eq!(Arc::strong_count(&pages[0].image.image), 1);
        Ok(())
    }

    #[test]
    fn renders_only_requested_pages_and_bounds_the_cache() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let source = (0..8)
            .map(|page| format!("= Page {page}\n#pagebreak()"))
            .collect::<Vec<_>>()
            .join("\n");
        let CompileOutcome::Success(document) = compiler.compile(&source) else {
            return Err("fixture did not compile".into());
        };
        let manifest = render_manifest(&document, 320)?;

        let (rendered, cache) = render_pages_cached_cancellable(
            &document,
            &manifest,
            [1, 3, 5, 7, 0, 2, 4],
            &RenderCache::default(),
            || false,
        )?;

        assert_eq!(
            rendered
                .iter()
                .map(super::RenderedPage::index)
                .collect::<Vec<_>>(),
            [1, 3, 5, 7, 0, 2, 4]
        );
        assert!(cache.pages.len() <= MAX_CACHED_PAGES);
        Ok(())
    }

    #[test]
    #[ignore = "manual release-mode performance probe"]
    fn image_heavy_document_reports_first_page_latency() -> Result<(), Box<dyn Error>> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let root = std::env::temp_dir().join(format!(
            "typst-tui-preview-perf-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root)?;
        let result = (|| -> Result<(), Box<dyn Error>> {
            let image = RgbaImage::from_fn(1_024, 1_024, |x, y| {
                image::Rgba([(x % 256) as u8, (y % 256) as u8, 128, 255])
            });
            image.save(root.join("sample.png"))?;
            let source = (0..100)
                .map(|_| "#image(\"sample.png\", width: 100%)")
                .collect::<Vec<_>>()
                .join("\n#pagebreak()\n");
            let main = root.join("main.typ");
            fs::write(&main, &source)?;
            let mut compiler = Compiler::new(&root, &main)?;

            let compile_started = Instant::now();
            let CompileOutcome::Success(document) = compiler.compile(&source) else {
                return Err("image-heavy fixture did not compile".into());
            };
            let compile_elapsed = compile_started.elapsed();
            let manifest_started = Instant::now();
            let manifest = render_manifest(&document, 800)?;
            let manifest_elapsed = manifest_started.elapsed();
            let first_page_started = Instant::now();
            let (pages, cache) = render_pages_cached_cancellable(
                &document,
                &manifest,
                [0],
                &RenderCache::default(),
                || false,
            )?;
            let first_page_elapsed = first_page_started.elapsed();

            eprintln!(
                "100 image pages: compile={compile_elapsed:?}, manifest={manifest_elapsed:?}, first_page={first_page_elapsed:?}"
            );
            assert_eq!(manifest.pages().len(), 100);
            assert_eq!(pages.len(), 1);
            assert_eq!(cache.pages.len(), 1);
            Ok(())
        })();
        let _ = fs::remove_dir_all(&root);
        result
    }
}
