use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Paragraph},
};
use ratatui_image::{
    picker::Picker,
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use typst_tui_render::RenderedDocument;

pub(crate) struct Preview {
    pages: Vec<SlicedProtocol>,
    scroll: usize,
    viewport: Size,
}

impl Preview {
    pub(crate) fn new() -> Self {
        Self {
            pages: Vec::new(),
            scroll: 0,
            viewport: Size::new(40, 20),
        }
    }

    pub(crate) fn encode_pages(
        picker: &Picker,
        rendered: RenderedDocument,
        width: u16,
    ) -> Result<Vec<SlicedProtocol>, String> {
        let width = width.max(1);
        let font = picker.font_size();
        rendered
            .into_pages()
            .into_iter()
            .map(|page| {
                let pixel_width = u64::from(page.width().max(1));
                let target_pixel_width = u64::from(width) * u64::from(font.width.max(1));
                let scaled_height = u64::from(page.height()) * target_pixel_width / pixel_width;
                let rows = scaled_height
                    .div_ceil(u64::from(font.height.max(1)))
                    .clamp(1, u64::from(u16::MAX)) as u16;
                SlicedProtocol::new(picker, page.into_image(), Some(Size::new(width, rows)))
                    .map_err(|error| error.to_string())
            })
            .collect()
    }

    pub(crate) fn replace_pages(&mut self, pages: Vec<SlicedProtocol>) {
        self.pages = pages;
        self.scroll = 0;
        self.clamp_scroll();
    }

    pub(crate) fn target_width(&self) -> u16 {
        self.viewport.width.max(1)
    }

    pub(crate) fn set_viewport(&mut self, area: Rect) {
        self.viewport = Size::new(area.width.saturating_sub(2), area.height.saturating_sub(2));
        self.clamp_scroll();
    }

    pub(crate) fn scroll_lines(&mut self, lines: isize) {
        self.scroll = self.scroll.saturating_add_signed(lines);
        self.clamp_scroll();
    }

    pub(crate) fn scroll_pages(&mut self, pages: isize) {
        let page_height = isize::try_from(self.viewport.height.max(1)).unwrap_or(isize::MAX);
        self.scroll_lines(pages.saturating_mul(page_height));
    }

    pub(crate) fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        self.set_viewport(area);
        let border_color = if focused {
            Color::Cyan
        } else {
            Color::DarkGray
        };
        let title = if self.pages.is_empty() {
            " Preview ".to_owned()
        } else {
            format!(
                " Preview | Page {} / {} ",
                self.current_page(),
                self.pages.len()
            )
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(border_color))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }
        if self.pages.is_empty() {
            frame.render_widget(Paragraph::new("No rendered preview").centered(), inner);
            return;
        }

        let mut page_top = 0_i64;
        for page in &self.pages {
            let size = page.size();
            let y = page_top - self.scroll as i64;
            if y < i64::from(inner.height) && y + i64::from(size.height) > 0 {
                let x = inner.width.saturating_sub(size.width) / 2;
                let position = SignedPosition::from((
                    i16::try_from(x).unwrap_or(i16::MAX),
                    y.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
                ));
                frame.render_widget(SlicedImage::new(page, position), inner);
            }

            page_top += i64::from(size.height);
            let divider_y = page_top - self.scroll as i64;
            if divider_y >= 0 && divider_y < i64::from(inner.height) {
                frame.render_widget(
                    Paragraph::new("-".repeat(usize::from(inner.width)))
                        .style(Style::default().fg(Color::DarkGray)),
                    Rect::new(inner.x, inner.y + divider_y as u16, inner.width, 1),
                );
            }
            page_top += 1;
        }
    }

    fn current_page(&self) -> usize {
        let mut top = 0;
        for (index, page) in self.pages.iter().enumerate() {
            let bottom = top + usize::from(page.size().height) + 1;
            if self.scroll < bottom {
                return index + 1;
            }
            top = bottom;
        }
        self.pages.len().max(1)
    }

    fn clamp_scroll(&mut self) {
        let content_height = self.pages.iter().fold(0_usize, |height, page| {
            height + usize::from(page.size().height) + 1
        });
        let max_scroll = content_height.saturating_sub(usize::from(self.viewport.height));
        self.scroll = self.scroll.min(max_scroll);
    }
}

impl Default for Preview {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, error::Error, fs, io, path::PathBuf};

    use ratatui::{Terminal, backend::TestBackend, style::Color};
    use ratatui_image::picker::Picker;
    use typst_tui_compiler::{CompileOutcome, Compiler};

    use super::Preview;

    #[test]
    fn draws_empty_preview_state() -> Result<(), Infallible> {
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        let mut preview = Preview::new();

        terminal.draw(|frame| preview.draw(frame, frame.area(), true))?;

        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>();
        assert!(rendered.contains("Preview"));
        assert!(rendered.contains("No rendered preview"));

        Ok(())
    }

    #[test]
    fn draws_rendered_typst_page() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let source = fs::read_to_string(root.join("simple.typ"))?;
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let CompileOutcome::Success(document) = compiler.compile(&source) else {
            return Err(io::Error::other("fixture did not compile").into());
        };
        let rendered = typst_tui_render::render(&document, 280)?;
        let pages =
            Preview::encode_pages(&Picker::halfblocks(), rendered, 28).map_err(io::Error::other)?;
        let mut preview = Preview::new();
        preview.replace_pages(pages);

        let backend = TestBackend::new(32, 14);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|frame| preview.draw(frame, frame.area(), true))?;

        let buffer = terminal.backend().buffer();
        let rendered = (0..buffer.area.height)
            .flat_map(|y| (0..buffer.area.width).map(move |x| buffer[(x, y)].symbol()))
            .collect::<String>();
        let has_raster_color = (1..buffer.area.height - 1).any(|y| {
            (1..buffer.area.width - 1).any(|x| {
                matches!(buffer[(x, y)].fg, Color::Rgb(_, _, _))
                    || matches!(buffer[(x, y)].bg, Color::Rgb(_, _, _))
            })
        });
        assert!(rendered.contains("Page 1 / 1"));
        assert!(has_raster_color);

        Ok(())
    }
}
