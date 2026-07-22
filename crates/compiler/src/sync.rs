use std::{num::NonZeroUsize, sync::Arc};

use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Duration},
    introspection::PagedPosition,
    layout::{Abs, Point},
    syntax::{FileId, Source},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_ide::{IdeWorld, Jump, jump_from_click, jump_from_cursor};
use typst_layout::PagedDocument;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PagePosition {
    pub page: usize,
    pub x: f64,
    pub y: f64,
}

#[derive(Clone)]
pub struct DocumentSync {
    document: Arc<PagedDocument>,
    world: Arc<SyncWorld>,
}

impl DocumentSync {
    pub(crate) fn new(document: PagedDocument, source: Source) -> Self {
        Self {
            document: Arc::new(document),
            world: Arc::new(SyncWorld::new(source)),
        }
    }

    #[must_use]
    pub fn position_from_cursor(&self, cursor: usize) -> Option<PagePosition> {
        jump_from_cursor(self.document.as_ref(), &self.world.source, cursor)
            .into_iter()
            .next()
            .and_then(|position| self.normalize(position))
    }

    #[must_use]
    pub fn source_from_click(&self, position: PagePosition) -> Option<usize> {
        if !position.x.is_finite()
            || !position.y.is_finite()
            || !(0.0..=1.0).contains(&position.x)
            || !(0.0..=1.0).contains(&position.y)
        {
            return None;
        }

        let page = self.document.pages().get(position.page)?;
        let size = page.frame.size();
        let page_number = NonZeroUsize::new(position.page.checked_add(1)?)?;
        let position = PagedPosition {
            page: page_number,
            point: Point::new(
                Abs::pt(size.x.to_pt() * position.x),
                Abs::pt(size.y.to_pt() * position.y),
            ),
        };
        match jump_from_click(self.world.as_ref(), self.document.as_ref(), &position) {
            Some(Jump::File(id, offset)) if id == self.world.main => Some(offset),
            _ => None,
        }
    }

    fn normalize(&self, position: PagedPosition) -> Option<PagePosition> {
        let page = position.page.get().checked_sub(1)?;
        let size = self.document.pages().get(page)?.frame.size();
        let (width, height) = (size.x.to_pt(), size.y.to_pt());
        if width <= 0.0 || height <= 0.0 {
            return None;
        }

        Some(PagePosition {
            page,
            x: (position.point.x.to_pt() / width).clamp(0.0, 1.0),
            y: (position.point.y.to_pt() / height).clamp(0.0, 1.0),
        })
    }
}

struct SyncWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    main: FileId,
    source: Source,
}

impl SyncWorld {
    fn new(source: Source) -> Self {
        Self {
            library: LazyHash::new(Library::default()),
            book: LazyHash::new(FontBook::new()),
            main: source.id(),
            source,
        }
    }
}

impl World for SyncWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }

    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }

    fn main(&self) -> FileId {
        self.main
    }

    fn source(&self, id: FileId) -> FileResult<Source> {
        (id == self.main)
            .then(|| self.source.clone())
            .ok_or(FileError::AccessDenied)
    }

    fn file(&self, id: FileId) -> FileResult<Bytes> {
        (id == self.main)
            .then(|| Bytes::from_string(self.source.clone()))
            .ok_or(FileError::AccessDenied)
    }

    fn font(&self, _index: usize) -> Option<Font> {
        None
    }

    fn today(&self, _offset: Option<Duration>) -> Option<Datetime> {
        None
    }
}

impl IdeWorld for SyncWorld {
    fn upcast(&self) -> &dyn World {
        self
    }
}
