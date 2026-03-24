use serde_json::Value;

use crate::clients::parsing::TransportKind;
use crate::core::request_builder::TransportCallRequest;

pub trait ProviderAdapter: Send + Sync {
    fn build_request_url(&self, api_base: &str, transport: TransportKind) -> String;
    fn build_request_body(&self, request: &TransportCallRequest, transport: TransportKind)
    -> Value;
}
