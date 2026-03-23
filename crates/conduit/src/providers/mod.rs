pub mod anthropic;
pub mod openai;

use crate::adapter::ProviderAdapter;
use crate::clients::parsing::TransportKind;

pub fn adapter_for_transport(transport: TransportKind) -> &'static dyn ProviderAdapter {
    match transport {
        TransportKind::Messages => &anthropic::ANTHROPIC_ADAPTER,
        TransportKind::Completion | TransportKind::Responses => &openai::OPENAI_ADAPTER,
    }
}
