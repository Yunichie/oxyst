use std::io;

use crossterm::event::{self, KeyEvent, MouseEvent};

#[derive(Debug)]
pub(crate) enum Event {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Paste(String),
    Resize(u16, u16),
    Ignored,
}

pub(crate) fn read() -> io::Result<Event> {
    Ok(match event::read()? {
        event::Event::Key(key) => Event::Key(key),
        event::Event::Mouse(mouse) => Event::Mouse(mouse),
        event::Event::Paste(text) => Event::Paste(text),
        event::Event::Resize(width, height) => Event::Resize(width, height),
        event::Event::FocusGained | event::Event::FocusLost => Event::Ignored,
    })
}
