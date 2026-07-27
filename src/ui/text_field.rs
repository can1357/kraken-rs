//! Single editing primitive behind every text input in the app.
//!
//! A [`TextField`] stores the string value the slab kernel commits into
//! application state through `UiAction::SetText`. The kernel owns caret and
//! selection interaction, so this type is a value holder: state code reads it
//! through `Deref<Target = str>` and replaces or appends text wholesale.

use std::ops::Deref;

/// Editable text state committed by the slab kernel.
#[derive(Clone, Debug, Default)]
pub(crate) struct TextField {
    text: String,
}

impl Deref for TextField {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl std::fmt::Display for TextField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl TextField {
    /// The current value.
    pub(crate) fn text(&self) -> &str {
        &self.text
    }

    /// Replaces the value.
    pub(crate) fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
    }

    /// Empties the field.
    pub(crate) fn clear(&mut self) {
        self.text.clear();
    }

    /// Appends text at the end of the value.
    pub(crate) fn insert(&mut self, text: &str) {
        self.text.push_str(text);
    }
}
