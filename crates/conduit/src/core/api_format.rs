use crate::clients::parsing::TransportKind;

/// Preferred API format for a provider request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ApiFormat {
    #[default]
    Auto,
    Completion,
    Responses,
    Messages,
}

impl ApiFormat {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Completion => "completion",
            Self::Responses => "responses",
            Self::Messages => "messages",
        }
    }
}

impl From<TransportKind> for ApiFormat {
    fn from(value: TransportKind) -> Self {
        match value {
            TransportKind::Completion => Self::Completion,
            TransportKind::Responses => Self::Responses,
            TransportKind::Messages => Self::Messages,
        }
    }
}
