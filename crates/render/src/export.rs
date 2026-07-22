use std::{fs, io::Cursor, path::Path};

use image::{DynamicImage, ImageFormat, Rgba, RgbaImage, imageops};
use thiserror::Error;
use typst::layout::Abs;
use typst_tui_compiler::CompiledDocument;

use crate::render_at;

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

#[derive(Debug, Error)]
pub enum Error {
    #[error("could not export PDF: {0}")]
    Pdf(String),
    #[error("could not render PNG")]
    Render(#[from] crate::Error),
    #[error("could not encode PNG")]
    Encode(#[from] image::ImageError),
    #[error("exported PNG dimensions are too large")]
    PngTooLarge,
    #[error("could not write export to {path}")]
    Write {
        path: std::path::PathBuf,
        #[source]
        source: std::io::Error,
    },
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
    let pages = render_at(document, EXPORT_PIXELS_PER_POINT)?;
    let width = pages
        .iter()
        .map(|page| page.image.width())
        .max()
        .unwrap_or(1);
    let gaps = u32::try_from(pages.len().saturating_sub(1))
        .map_err(|_| Error::PngTooLarge)?
        .checked_mul(PAGE_GAP)
        .ok_or(Error::PngTooLarge)?;
    let height = pages.iter().try_fold(gaps, |height, page| {
        height
            .checked_add(page.image.height())
            .ok_or(Error::PngTooLarge)
    })?;
    let mut image = RgbaImage::from_pixel(width, height.max(1), Rgba([255, 255, 255, 255]));
    let mut y = 0_u32;
    for page in pages {
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
