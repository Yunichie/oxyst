use std::{fs, io::Cursor, path::Path};

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage, imageops};
use oxyst_compiler::CompiledDocument;
use typst::layout::Abs;

use crate::{
    Error, MAX_TOTAL_PIXELS, RenderManifest, manifest_at, render_pages_cancellable,
    validate_total_pixels,
};

const EXPORT_PIXELS_PER_POINT: f64 = 2.0;
const PAGE_GAP: u32 = 16;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportFormat {
    Pdf,
    Png,
    Svg,
}

impl ExportFormat {
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Png => "png",
            Self::Svg => "svg",
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Png => "PNG",
            Self::Svg => "SVG",
        }
    }
}

pub fn export(document: &CompiledDocument, format: ExportFormat, path: &Path) -> Result<(), Error> {
    let bytes = match format {
        ExportFormat::Pdf => {
            typst_pdf::pdf(document.paged_document(), &typst_pdf::PdfOptions::default())
                .map_err(|diagnostics| Error::Pdf(format!("{diagnostics:?}")))?
        }
        ExportFormat::Png => png(document)?,
        ExportFormat::Svg => typst_svg::svg_merged(
            document.paged_document(),
            &typst_svg::SvgOptions::default(),
            Abs::pt(4.0),
        )
        .into_bytes(),
    };

    fs::write(path, bytes).map_err(|source| Error::Write {
        path: path.to_owned(),
        source,
    })
}

fn png(document: &CompiledDocument) -> Result<Vec<u8>, Error> {
    let manifest = manifest_at(document, EXPORT_PIXELS_PER_POINT)?;
    validate_total_pixels(&manifest)?;
    let (width, height) = png_dimensions(&manifest)?;
    let mut image = RgbaImage::from_pixel(width, height.max(1), Rgba([255, 255, 255, 255]));
    let mut y = 0_u32;
    for index in 0..manifest.pages().len() {
        let page = render_pages_cancellable(document, &manifest, [index], || false)?
            .into_iter()
            .next()
            .ok_or(Error::PageOutOfBounds { page: index + 1 })?
            .into_image();
        let x = (width - page.image.width()) / 2;
        imageops::replace(&mut image, page.image.as_ref(), i64::from(x), i64::from(y));
        y = y
            .saturating_add(page.image.height())
            .saturating_add(PAGE_GAP);
    }

    let mut bytes = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(image).write_to(&mut bytes, ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

fn png_dimensions(manifest: &RenderManifest) -> Result<(u32, u32), Error> {
    let width = manifest
        .pages()
        .iter()
        .map(|page| page.width())
        .max()
        .unwrap_or(1);
    let gaps = u32::try_from(manifest.pages().len().saturating_sub(1))
        .map_err(|_| Error::PngTooLarge)?
        .checked_mul(PAGE_GAP)
        .ok_or(Error::PngTooLarge)?;
    let height = manifest.pages().iter().try_fold(gaps, |height, page| {
        height.checked_add(page.height()).ok_or(Error::PngTooLarge)
    })?;
    let pixels = u64::from(width)
        .checked_mul(u64::from(height))
        .ok_or(Error::PngTooLarge)?;
    if pixels > MAX_TOTAL_PIXELS
        || usize::try_from(pixels)
            .ok()
            .and_then(|pixels| pixels.checked_mul(4))
            .is_none()
    {
        return Err(Error::PngTooLarge);
    }
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use crate::{Error, PageFingerprint, PageMetadata, RenderManifest};

    use super::png_dimensions;

    fn manifest(sizes: &[(u32, u32)]) -> RenderManifest {
        RenderManifest {
            pages: sizes
                .iter()
                .map(|&(width, height)| PageMetadata {
                    width,
                    height,
                    fingerprint: PageFingerprint(0),
                })
                .collect(),
            pixels_per_point: 1.0,
        }
    }

    #[test]
    fn png_canvas_includes_gaps() -> Result<(), Error> {
        assert_eq!(png_dimensions(&manifest(&[(10, 20), (8, 30)]))?, (10, 66));
        Ok(())
    }

    #[test]
    fn png_canvas_rejects_mixed_sizes_that_exceed_the_budget() {
        assert!(matches!(
            png_dimensions(&manifest(&[(16_384, 1), (1, 16_384)])),
            Err(Error::PngTooLarge)
        ));
    }
}
