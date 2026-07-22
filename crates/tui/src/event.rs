use std::{
    io,
    sync::mpsc::{Receiver, TryRecvError},
    time::Duration,
};

use crossterm::event::{self, KeyEvent};

use crate::compile::CompileResult;

pub(crate) enum Event {
    Key(KeyEvent),
    Paste(String),
    CompileFinished(CompileResult),
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
        event::Event::Mouse(_) => Event::Ignored,
        event::Event::Paste(text) => Event::Paste(text),
        event::Event::Resize(_, _) => Event::Ignored,
        event::Event::FocusGained | event::Event::FocusLost => Event::Ignored,
    })
}
