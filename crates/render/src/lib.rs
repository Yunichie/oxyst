#![forbid(unsafe_code)]

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
}

pub struct RenderedDocument {
    pages: Vec<PageImage>,
    pixels_per_point: f64,
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

pub struct PageImage {
    image: RgbaImage,
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
        DynamicImage::ImageRgba8(self.image)
    }
}

pub fn render(document: &CompiledDocument, target_width: u32) -> Result<RenderedDocument, Error> {
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
    validate_dimensions(document, pixels_per_point)?;

    let options = RenderOptions {
        pixel_per_pt: pixels_per_point.into(),
        render_bleed: false,
    };
    let pages = document
        .pages()
        .iter()
        .enumerate()
        .map(|(index, page)| {
            let pixmap = typst_render::render(page, &options);
            let (width, height) = (pixmap.width(), pixmap.height());
            let mut rgba = pixmap.data().to_vec();
            unpremultiply(&mut rgba);
            let image = RgbaImage::from_raw(width, height, rgba)
                .ok_or(Error::InvalidPixelData { page: index + 1 })?;
            Ok(PageImage { image })
        })
        .collect::<Result<Vec<_>, Error>>()?;

    Ok(RenderedDocument {
        pages,
        pixels_per_point,
    })
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
    use super::unpremultiply;

    #[test]
    fn converts_premultiplied_alpha() {
        let mut pixel = [64, 32, 16, 128];
        unpremultiply(&mut pixel);
        assert_eq!(pixel, [128, 64, 32, 128]);
    }
}
