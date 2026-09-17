use std::{
    io,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use crossterm::event::{self, KeyEvent, MouseEvent};

use crate::message::{ExplorerScanResult, ExportResult};

pub(crate) enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize,
    ProjectFilesChanged,
    FileWatchFailed(String),
    ExplorerScanFinished(ExplorerScanResult),
    ExportFinished(ExportResult),
    Tick,
    Ignored,
}

pub(crate) fn read(internal: &Receiver<Event>) -> io::Result<Event> {
    match internal.try_recv() {
        Ok(event) => return Ok(event),
        Err(TryRecvError::Empty | TryRecvError::Disconnected) => {}
    }

    if !event::poll(Duration::from_millis(50))? {
        return Ok(Event::Tick);
    }

    Ok(match event::read()? {
        event::Event::Key(key) => Event::Key(key),
        event::Event::Mouse(mouse) => Event::Mouse(mouse),
        event::Event::Paste(text) => Event::Paste(text),
        event::Event::Resize(_, _) => Event::Resize,
        event::Event::FocusGained | event::Event::FocusLost => Event::Ignored,
    })
}
