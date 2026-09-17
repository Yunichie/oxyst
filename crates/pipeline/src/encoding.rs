use oxyst_render::{PageImage, RenderedDocument, RenderedPage};
use ratatui::layout::Size;
use ratatui_image::{
    picker::{Picker, ProtocolType},
    protocol::halfblocks::Halfblocks,
    sliced::SlicedProtocol,
};

pub fn encode_document(
    picker: &Picker,
    rendered: RenderedDocument,
    width: u16,
) -> Result<Vec<SlicedProtocol>, String> {
    rendered
        .into_pages()
        .into_iter()
        .map(|page| encode_page(picker, page, width))
        .collect()
}

pub fn encode_rendered_pages_cancellable(
    picker: &Picker,
    rendered: Vec<RenderedPage>,
    width: u16,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Vec<(usize, SlicedProtocol)>>, String> {
    let mut pages = Vec::with_capacity(rendered.len());
    for page in rendered {
        if cancelled() {
            return Ok(None);
        }
        let index = page.index();
        pages.push((index, encode_page(picker, page.into_image(), width)?));
    }
    Ok(Some(pages))
}

fn encode_page(picker: &Picker, page: PageImage, width: u16) -> Result<SlicedProtocol, String> {
    let width = width.max(1);
    let font = picker.font_size();
    let pixel_width = u64::from(page.width().max(1));
    let target_pixel_width = u64::from(width) * u64::from(font.width.max(1));
    let scaled_height = u64::from(page.height()) * target_pixel_width / pixel_width;
    let rows = scaled_height
        .div_ceil(u64::from(font.height.max(1)))
        .clamp(1, u64::from(u16::MAX)) as u16;
    let size = Size::new(width, rows);
    let image = page.into_image();
    match picker.protocol_type() {
        ProtocolType::Halfblocks => Halfblocks::new(image, size)
            .map(SlicedProtocol::Halfblocks)
            .map_err(|error| error.to_string()),
        ProtocolType::Sixel | ProtocolType::Kitty | ProtocolType::Iterm2 => {
            SlicedProtocol::new(picker, image, Some(size)).map_err(|error| error.to_string())
        }
    }
}
