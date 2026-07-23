use std::collections::{HashMap, VecDeque};

use ratatui::{
    Frame,
    layout::{Rect, Size},
    style::{Modifier, Style},
    widgets::{
        Block, BorderType, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState,
    },
};
use ratatui_image::{
    picker::Picker,
    sliced::{SignedPosition, SlicedImage, SlicedProtocol},
};
use typst_tui_compiler::PagePosition;
#[cfg(test)]
use typst_tui_render::RenderedDocument;
use typst_tui_render::{PageImage, RenderManifest, RenderedPage};
use typst_tui_theme::Theme;

use crate::{action::Action, style::color};

use super::Component;

const ENCODED_PAGE_CACHE_LIMIT: usize = 5;

pub(crate) struct Preview {
    page_sizes: Vec<Size>,
    page_tops: Vec<usize>,
    pages: HashMap<usize, SlicedProtocol>,
    page_lru: VecDeque<usize>,
    scroll: usize,
    viewport: Size,
    inner: Rect,
    zoom: u16,
    theme: Theme,
}

impl Preview {
    pub(crate) fn new(theme: Theme) -> Self {
        Self {
            page_sizes: Vec::new(),
            page_tops: vec![0],
            pages: HashMap::new(),
            page_lru: VecDeque::new(),
            scroll: 0,
            viewport: Size::new(40, 20),
            inner: Rect::default(),
            zoom: 100,
            theme,
        }
    }

    #[cfg(test)]
    pub(crate) fn encode_pages(
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

    pub(crate) fn encode_rendered_pages_cancellable(
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

    pub(crate) fn page_sizes(picker: &Picker, manifest: &RenderManifest, width: u16) -> Vec<Size> {
        let width = width.max(1);
        let font = picker.font_size();
        manifest
            .pages()
            .iter()
            .map(|page| {
                let pixel_width = u64::from(page.width().max(1));
                let target_pixel_width = u64::from(width) * u64::from(font.width.max(1));
                let scaled_height = u64::from(page.height()) * target_pixel_width / pixel_width;
                let rows = scaled_height
                    .div_ceil(u64::from(font.height.max(1)))
                    .clamp(1, u64::from(u16::MAX)) as u16;
                Size::new(width, rows)
            })
            .collect()
    }

    pub(crate) fn replace_manifest(&mut self, page_sizes: Vec<Size>) {
        self.page_sizes = page_sizes;
        self.rebuild_page_tops();
        self.pages.clear();
        self.page_lru.clear();
        self.scroll = 0;
        self.clamp_scroll();
    }

    pub(crate) fn install_pages(&mut self, pages: Vec<(usize, SlicedProtocol)>) {
        for (index, page) in pages {
            if index >= self.page_sizes.len() {
                continue;
            }
            self.pages.insert(index, page);
            self.page_lru.retain(|candidate| *candidate != index);
            self.page_lru.push_back(index);
        }
        let required = self.required_pages();
        while self.pages.len() > ENCODED_PAGE_CACHE_LIMIT {
            let Some(position) = self
                .page_lru
                .iter()
                .position(|index| !required.contains(index))
            else {
                break;
            };
            let Some(index) = self.page_lru.remove(position) else {
                break;
            };
            self.pages.remove(&index);
        }
    }

    pub(crate) fn page_requests(&self) -> Vec<usize> {
        self.required_pages()
            .into_iter()
            .filter(|index| !self.pages.contains_key(index))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn replace_pages(&mut self, pages: Vec<SlicedProtocol>) {
        let sizes = pages.iter().map(SlicedProtocol::size).collect();
        self.replace_manifest(sizes);
        self.install_pages(pages.into_iter().enumerate().collect());
    }

    pub(crate) fn clear(&mut self) {
        self.page_sizes.clear();
        self.page_tops.clear();
        self.page_tops.push(0);
        self.pages.clear();
        self.page_lru.clear();
        self.scroll = 0;
    }

    pub(crate) fn target_width(&self) -> u16 {
        let width = u32::from(self.viewport.width.max(1)) * u32::from(self.zoom) / 100;
        u16::try_from(width).unwrap_or(u16::MAX).max(1)
    }

    pub(crate) fn zoom(&mut self, direction: isize) -> bool {
        let delta = if direction < 0 { -25 } else { 25 };
        let next = self.zoom.saturating_add_signed(delta);
        let next = next.clamp(50, 200);
        let changed = next != self.zoom;
        self.zoom = next;
        changed
    }

    pub(crate) fn go_to_page(&mut self, page: usize) -> bool {
        if page == 0 || page > self.page_sizes.len() {
            return false;
        }
        self.scroll = self.page_top(page - 1);
        self.clamp_scroll();
        true
    }

    pub(crate) fn set_viewport(&mut self, area: Rect) {
        self.viewport = Size::new(area.width.saturating_sub(2), area.height.saturating_sub(2));
        self.inner = Rect::new(
            area.x.saturating_add(1),
            area.y.saturating_add(1),
            self.viewport.width,
            self.viewport.height,
        );
        self.clamp_scroll();
    }

    pub(crate) fn hide(&mut self) {
        self.inner = Rect::default();
    }

    pub(crate) fn contains(&self, column: u16, row: u16) -> bool {
        column >= self.inner.x
            && column < self.inner.right()
            && row >= self.inner.y
            && row < self.inner.bottom()
    }

    pub(crate) fn position_at(&self, column: u16, row: u16) -> Option<PagePosition> {
        if !self.contains(column, row) {
            return None;
        }

        let content_y = self.scroll + usize::from(row - self.inner.y);
        let page_index = self.page_index_at(content_y)?;
        let page_top = self.page_top(page_index);
        let size = self.page_sizes[page_index];
        if content_y >= page_top + usize::from(size.height) {
            return None;
        }
        let page_left = self.inner.x + self.inner.width.saturating_sub(size.width) / 2;
        if column < page_left || column >= page_left.saturating_add(size.width) {
            return None;
        }
        Some(PagePosition {
            page: page_index,
            x: (f64::from(column - page_left) + 0.5) / f64::from(size.width.max(1)),
            y: ((content_y - page_top) as f64 + 0.5) / f64::from(size.height.max(1)),
        })
    }

    pub(crate) fn scroll_to(&mut self, position: PagePosition) {
        if !position.y.is_finite() || !(0.0..=1.0).contains(&position.y) {
            return;
        }
        let Some(size) = self.page_sizes.get(position.page).copied() else {
            return;
        };
        let offset = (f64::from(size.height) * position.y).round() as usize;
        let target = self.page_top(position.page) + offset;
        self.scroll = target.saturating_sub(usize::from(self.viewport.height) / 2);
        self.clamp_scroll();
    }

    pub(crate) fn scroll_lines(&mut self, lines: isize) {
        self.scroll = self.scroll.saturating_add_signed(lines);
        self.clamp_scroll();
    }

    pub(crate) fn scroll_pages(&mut self, pages: isize) {
        let Some(current) = self.current_page_index() else {
            return;
        };
        let target = current
            .saturating_add_signed(pages)
            .min(self.page_sizes.len() - 1);
        self.scroll = self.page_top(target);
        self.clamp_scroll();
    }

    pub(crate) fn set_theme(&mut self, theme: Theme) {
        self.theme = theme;
    }

    pub(crate) fn draw_preview(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        focused: bool,
        dimmed: bool,
    ) {
        self.set_viewport(area);
        let border_color = if focused {
            self.theme.accent
        } else {
            self.theme.border
        };
        let title = if self.page_sizes.is_empty() {
            " Preview ".to_owned()
        } else {
            format!(
                " Preview | Page {} / {} | {}% ",
                self.current_page(),
                self.page_sizes.len(),
                self.zoom
            )
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(color(border_color)))
            .title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);

        if inner.width == 0 || inner.height == 0 {
            return;
        }
        if self.page_sizes.is_empty() {
            frame.render_widget(Paragraph::new("No rendered preview").centered(), inner);
            return;
        }

        if let Some(range) = self.visible_page_range() {
            for page_index in range {
                let size = self.page_sizes[page_index];
                let page_top = self.page_top(page_index) as i64;
                let y = page_top - self.scroll as i64;
                let x = inner.width.saturating_sub(size.width) / 2;
                if self.pages.contains_key(&page_index) {
                    self.page_lru.retain(|candidate| *candidate != page_index);
                    self.page_lru.push_back(page_index);
                }
                if let Some(page) = self.pages.get(&page_index) {
                    let position = SignedPosition::from((
                        i16::try_from(x).unwrap_or(i16::MAX),
                        y.clamp(i64::from(i16::MIN), i64::from(i16::MAX)) as i16,
                    ));
                    frame.render_widget(SlicedImage::new(page, position), inner);
                } else {
                    let visible_top = y.max(0) as u16;
                    let visible_bottom =
                        (y + i64::from(size.height)).min(i64::from(inner.height)) as u16;
                    if visible_top < visible_bottom {
                        frame.render_widget(
                            Paragraph::new("Rendering...").centered(),
                            Rect::new(
                                inner.x.saturating_add(x),
                                inner.y.saturating_add(visible_top),
                                size.width.min(inner.width),
                                visible_bottom - visible_top,
                            ),
                        );
                    }
                }

                let divider_y = page_top + i64::from(size.height) - self.scroll as i64;
                if divider_y >= 0 && divider_y < i64::from(inner.height) {
                    frame.render_widget(
                        Paragraph::new("-".repeat(usize::from(inner.width)))
                            .style(Style::default().fg(color(self.theme.border))),
                        Rect::new(inner.x, inner.y + divider_y as u16, inner.width, 1),
                    );
                }
            }
        }
        if dimmed {
            frame.render_widget(
                Block::default().style(Style::default().add_modifier(Modifier::DIM)),
                inner,
            );
        }
        let content_height = self.content_height();
        if content_height > usize::from(inner.height) && inner.width > 1 {
            let mut state = ScrollbarState::new(content_height)
                .position(self.scroll)
                .viewport_content_length(usize::from(inner.height));
            frame.render_stateful_widget(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .thumb_style(Style::default().fg(color(self.theme.muted)))
                    .track_style(Style::default().fg(color(self.theme.border))),
                inner,
                &mut state,
            );
        }
    }

    fn current_page(&self) -> usize {
        self.current_page_index().map_or(1, |index| index + 1)
    }

    fn current_page_index(&self) -> Option<usize> {
        if self.page_sizes.is_empty() {
            return None;
        }
        if self.scroll > 0 && self.scroll == self.max_scroll() {
            return Some(self.page_sizes.len() - 1);
        }
        self.page_index_at(self.scroll)
    }

    fn page_top(&self, page: usize) -> usize {
        self.page_tops.get(page).copied().unwrap_or(0)
    }

    fn page_index_at(&self, content_y: usize) -> Option<usize> {
        if self.page_sizes.is_empty() {
            return None;
        }
        Some(
            self.page_tops
                .partition_point(|top| *top <= content_y)
                .saturating_sub(1)
                .min(self.page_sizes.len() - 1),
        )
    }

    fn visible_page_range(&self) -> Option<std::ops::Range<usize>> {
        let start = self.page_index_at(self.scroll)?;
        let end_y = self
            .scroll
            .saturating_add(usize::from(self.viewport.height.saturating_sub(1)));
        let end = self.page_index_at(end_y)?;
        Some(start..end.saturating_add(1))
    }

    fn required_pages(&self) -> Vec<usize> {
        let Some(visible) = self.visible_page_range() else {
            return Vec::new();
        };
        let mut pages = Vec::new();
        let current = self.current_page_index().unwrap_or(visible.start);
        pages.push(current);
        for page in visible.clone() {
            if !pages.contains(&page) {
                pages.push(page);
            }
        }
        if let Some(previous) = visible.start.checked_sub(1) {
            pages.push(previous);
        }
        if visible.end < self.page_sizes.len() {
            pages.push(visible.end);
        }
        pages
    }

    fn rebuild_page_tops(&mut self) {
        self.page_tops.clear();
        self.page_tops.reserve(self.page_sizes.len() + 1);
        self.page_tops.push(0);
        for page in &self.page_sizes {
            let next = self
                .page_tops
                .last()
                .copied()
                .unwrap_or(0_usize)
                .saturating_add(usize::from(page.height))
                .saturating_add(1);
            self.page_tops.push(next);
        }
    }

    fn max_scroll(&self) -> usize {
        self.content_height()
            .saturating_sub(usize::from(self.viewport.height))
    }

    fn clamp_scroll(&mut self) {
        self.scroll = self.scroll.min(self.max_scroll());
    }

    fn content_height(&self) -> usize {
        self.page_tops.last().copied().unwrap_or(0)
    }
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
    SlicedProtocol::new(picker, page.into_image(), Some(Size::new(width, rows)))
        .map_err(|error| error.to_string())
}

impl Default for Preview {
    fn default() -> Self {
        Self::new(Theme::new(
            typst_tui_theme::ThemeName::Dark,
            typst_tui_theme::ColorDepth::Ansi16,
        ))
    }
}

impl Component for Preview {
    fn update(&mut self, action: Action) {
        match action {
            Action::ScrollPreviewPages(pages) => self.scroll_pages(pages),
            Action::Move(typst_tui_document::Motion::Up) => self.scroll_lines(-1),
            Action::Move(typst_tui_document::Motion::Down) => self.scroll_lines(1),
            _ => {}
        }
    }

    fn draw(&mut self, frame: &mut Frame, area: Rect, focused: bool) {
        self.draw_preview(frame, area, focused, false);
    }
}

#[cfg(test)]
mod tests {
    use std::{convert::Infallible, error::Error, fs, io, path::PathBuf, time::Instant};

    use ratatui::{
        Terminal,
        backend::TestBackend,
        layout::{Rect, Size},
        style::{Color, Modifier},
        text::Line,
        widgets::Paragraph,
    };
    use ratatui_image::picker::{Picker, ProtocolType};
    use typst_tui_compiler::{CompileOutcome, Compiler, PagePosition};
    use typst_tui_theme::{ColorDepth, Theme, ThemeName};

    use super::Preview;

    fn theme() -> Theme {
        Theme::new(ThemeName::Dark, ColorDepth::Ansi16)
    }

    #[test]
    fn draws_empty_preview_state() -> Result<(), Infallible> {
        let backend = TestBackend::new(30, 6);
        let mut terminal = Terminal::new(backend)?;
        let mut preview = Preview::new(theme());

        terminal.draw(|frame| preview.draw_preview(frame, frame.area(), true, false))?;

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
        let mut preview = Preview::new(theme());
        preview.replace_pages(pages);

        let backend = TestBackend::new(32, 8);
        let mut terminal = Terminal::new(backend)?;
        terminal.draw(|frame| preview.draw_preview(frame, frame.area(), true, false))?;

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
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| matches!(cell.symbol(), "▲" | "▼" | "█"))
        );

        let clicked = preview
            .position_at(16, 2)
            .ok_or_else(|| io::Error::other("preview click did not hit the page"))?;
        assert_eq!(clicked.page, 0);
        preview.scroll_to(PagePosition {
            page: 0,
            x: 0.5,
            y: 1.0,
        });
        terminal.draw(|frame| preview.draw_preview(frame, frame.area(), true, true))?;
        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.modifier.contains(Modifier::DIM))
        );

        Ok(())
    }

    #[test]
    fn page_navigation_moves_to_page_boundaries() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let source = fs::read_to_string(root.join("multi-page.typ"))?;
        let mut compiler = Compiler::new(&root, root.join("multi-page.typ"))?;
        let CompileOutcome::Success(document) = compiler.compile(&source) else {
            return Err(io::Error::other("multi-page fixture did not compile").into());
        };
        let rendered = typst_tui_render::render(&document, 280)?;
        let pages =
            Preview::encode_pages(&Picker::halfblocks(), rendered, 28).map_err(io::Error::other)?;
        let mut preview = Preview::new(theme());
        preview.replace_pages(pages);
        preview.set_viewport(ratatui::layout::Rect::new(0, 0, 32, 8));

        assert_eq!(preview.current_page(), 1);
        preview.scroll_pages(1);
        assert_eq!(preview.current_page(), 2);
        preview.scroll_pages(1);
        assert_eq!(preview.current_page(), 2);
        preview.scroll_pages(-1);
        assert_eq!(preview.current_page(), 1);

        Ok(())
    }

    #[test]
    fn requests_only_visible_pages_and_adjacent_overscan() {
        let mut preview = Preview::new(theme());
        preview.replace_manifest(vec![Size::new(20, 10); 10]);
        preview.set_viewport(Rect::new(0, 0, 22, 7));

        assert_eq!(preview.page_requests(), vec![0, 1]);
        assert!(preview.go_to_page(5));
        assert_eq!(preview.page_requests(), vec![4, 3, 5]);
    }

    #[test]
    fn encoded_page_cache_keeps_the_visible_window() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let source = (0..8)
            .map(|page| format!("= Page {page}\n#pagebreak()"))
            .collect::<Vec<_>>()
            .join("\n");
        let CompileOutcome::Success(document) = compiler.compile(&source) else {
            return Err("fixture did not compile".into());
        };
        let rendered = typst_tui_render::render(&document, 200)?;
        let pages =
            Preview::encode_pages(&Picker::halfblocks(), rendered, 20).map_err(io::Error::other)?;
        let mut preview = Preview::new(theme());
        preview.replace_pages(pages);

        assert!(preview.pages.len() <= super::ENCODED_PAGE_CACHE_LIMIT);
        assert!(preview.pages.contains_key(&0));
        Ok(())
    }

    #[test]
    #[ignore = "manual release-mode performance probe"]
    fn first_page_terminal_encoding_latency() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let mut compiler = Compiler::new(&root, root.join("simple.typ"))?;
        let source = (0..100)
            .map(|page| format!("#rect(width: 100%, height: 100%, fill: rgb({page}, 80, 120))"))
            .collect::<Vec<_>>()
            .join("\n#pagebreak()\n");
        let CompileOutcome::Success(document) = compiler.compile(&source) else {
            return Err("fixture did not compile".into());
        };
        let picker = Picker::halfblocks();
        let width = 80;
        let pixels = u32::from(width) * u32::from(picker.font_size().width.max(1));
        let manifest = typst_tui_render::render_manifest(&document, pixels)?;
        let (rendered, _) = typst_tui_render::render_pages_cached_cancellable(
            &document,
            &manifest,
            [0],
            &typst_tui_render::RenderCache::default(),
            || false,
        )?;

        let started = Instant::now();
        let encoded =
            Preview::encode_rendered_pages_cancellable(&picker, rendered, width, || false)?
                .ok_or("encoding was cancelled")?;
        let elapsed = started.elapsed();

        eprintln!("100-page document: first_page_terminal_encoding={elapsed:?}");
        assert_eq!(encoded.len(), 1);
        Ok(())
    }

    #[test]
    fn scrolled_preview_stays_inside_its_area() -> Result<(), Box<dyn Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
        let source = fs::read_to_string(root.join("multi-page.typ"))?;
        let mut compiler = Compiler::new(&root, root.join("multi-page.typ"))?;
        let CompileOutcome::Success(document) = compiler.compile(&source) else {
            return Err(io::Error::other("multi-page fixture did not compile").into());
        };
        let width = 46;
        let height = 12;
        let area = Rect::new(22, 2, 22, 8);
        for protocol_type in [ProtocolType::Iterm2, ProtocolType::Sixel] {
            let mut picker = Picker::halfblocks();
            picker.set_protocol_type(protocol_type);
            let font_height = usize::from(picker.font_size().height);
            let rendered = typst_tui_render::render(&document, 280)?;
            let pages = Preview::encode_pages(&picker, rendered, 18).map_err(io::Error::other)?;
            let mut preview = Preview::new(theme());
            preview.replace_pages(pages);
            preview.scroll_lines(5);

            let background = (0..height)
                .map(|_| Line::raw(".".repeat(width.into())))
                .collect::<Vec<_>>();
            let mut terminal = Terminal::new(TestBackend::new(width, height))?;
            terminal.draw(|frame| {
                frame.render_widget(Paragraph::new(background), frame.area());
                preview.draw_preview(frame, area, true, false);
            })?;

            let buffer = terminal.backend().buffer();
            for y in 0..height {
                for x in 0..width {
                    if !area.contains((x, y).into()) {
                        assert_eq!(
                            buffer[(x, y)].symbol(),
                            ".",
                            "{protocol_type:?} rendered outside the preview"
                        );
                    }
                }
            }
            let sequences = (area.y + 1..area.bottom() - 1)
                .flat_map(|y| (area.x + 1..area.right() - 1).map(move |x| buffer[(x, y)].symbol()))
                .filter(|symbol| symbol.contains('\x1b'))
                .collect::<Vec<_>>();
            assert!(
                !sequences.is_empty(),
                "{protocol_type:?} did not render an image sequence"
            );

            if protocol_type == ProtocolType::Sixel {
                let band_count = sequences[0].bytes().filter(|byte| *byte == b'-').count();
                let visible_rows = usize::from(area.height.saturating_sub(2));
                let max_visible_bands = (visible_rows * font_height).div_ceil(6);
                assert!(
                    band_count <= max_visible_bands,
                    "Sixel rendered {band_count} bands for a {visible_rows}-row viewport"
                );
            }
        }

        Ok(())
    }
}
