pub(crate) struct Clipboard {
    system: Option<arboard::Clipboard>,
    register: String,
}

impl Clipboard {
    pub(crate) fn new() -> Self {
        Self {
            system: arboard::Clipboard::new().ok(),
            register: String::new(),
        }
    }

    pub(crate) fn copy(&mut self, text: &str) -> bool {
        self.register.clear();
        self.register.push_str(text);
        self.system
            .as_mut()
            .is_some_and(|clipboard| clipboard.set_text(text).is_ok())
    }

    pub(crate) fn paste(&mut self) -> Option<(String, bool)> {
        if let Some(text) = self
            .system
            .as_mut()
            .and_then(|clipboard| clipboard.get_text().ok())
        {
            self.register.clone_from(&text);
            return Some((text, true));
        }
        (!self.register.is_empty()).then(|| (self.register.clone(), false))
    }
}

impl Default for Clipboard {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Clipboard;

    #[test]
    fn internal_register_is_a_clipboard_fallback() {
        let mut clipboard = Clipboard {
            system: None,
            register: String::new(),
        };

        assert!(!clipboard.copy("selected"));
        assert_eq!(clipboard.paste(), Some(("selected".to_owned(), false)));
    }
}
