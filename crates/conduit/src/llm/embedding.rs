//! Embedding input types.

/// Input for embedding operations.
pub enum EmbedInput<'a> {
    Single(&'a str),
    Multiple(&'a [String]),
}

impl<'a> From<&'a str> for EmbedInput<'a> {
    fn from(s: &'a str) -> Self {
        EmbedInput::Single(s)
    }
}

impl<'a> From<&'a [String]> for EmbedInput<'a> {
    fn from(v: &'a [String]) -> Self {
        EmbedInput::Multiple(v)
    }
}
