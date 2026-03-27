use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::channels::message::{ChannelMessage, MessageKind};

pub const SIDECAR_CONTRACT_VERSION: &str = "eli.sidecar.v1";
const SIDECAR_SCHEMA_ID: &str = "https://eliagent.github.io/contracts/eli-sidecar-v1.schema.json";
const SIDECAR_SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SidecarMediaPayload {
    pub media_type: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data_base64: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SidecarContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_channel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sender_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chat_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_target: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outbound_media: Vec<SidecarMediaPayload>,
    #[serde(
        default,
        rename = "_eli_cleanup_only",
        skip_serializing_if = "is_false"
    )]
    pub eli_cleanup_only: bool,
    #[serde(flatten)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SidecarChannelMessage {
    #[serde(default = "contract_version")]
    pub contract_version: String,
    pub session_id: String,
    pub channel: String,
    pub content: String,
    pub chat_id: String,
    #[serde(default)]
    pub is_active: bool,
    #[serde(default)]
    pub kind: MessageKind,
    #[serde(default)]
    pub context: SidecarContext,
    pub output_channel: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<SidecarMediaPayload>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct SidecarToolRequest {
    #[serde(default = "contract_version")]
    pub contract_version: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SidecarNoticeRequest {
    #[serde(default = "contract_version")]
    pub contract_version: String,
    pub session_id: String,
    pub text: String,
}

impl SidecarContext {
    pub fn from_map(map: Map<String, Value>) -> Self {
        let mut extra = map;
        Self {
            source_channel: take_string(&mut extra, "source_channel"),
            account_id: take_string(&mut extra, "account_id"),
            sender_id: take_string(&mut extra, "sender_id"),
            sender_name: take_string(&mut extra, "sender_name"),
            chat_type: take_string(&mut extra, "chat_type"),
            group_label: take_string(&mut extra, "group_label"),
            reply_to_id: take_string(&mut extra, "reply_to_id"),
            channel_target: take_string(&mut extra, "channel_target"),
            outbound_media: take_media(&mut extra, "outbound_media"),
            eli_cleanup_only: take_bool(&mut extra, "_eli_cleanup_only"),
            extra,
        }
    }

    pub fn into_map(self) -> Map<String, Value> {
        let mut extra = self.extra;
        insert_string(&mut extra, "source_channel", self.source_channel);
        insert_string(&mut extra, "account_id", self.account_id);
        insert_string(&mut extra, "sender_id", self.sender_id);
        insert_string(&mut extra, "sender_name", self.sender_name);
        insert_string(&mut extra, "chat_type", self.chat_type);
        insert_string(&mut extra, "group_label", self.group_label);
        insert_string(&mut extra, "reply_to_id", self.reply_to_id);
        insert_string(&mut extra, "channel_target", self.channel_target);
        insert_media(&mut extra, "outbound_media", self.outbound_media);
        insert_bool(&mut extra, "_eli_cleanup_only", self.eli_cleanup_only);
        extra
    }
}

impl SidecarChannelMessage {
    pub fn from_channel_message(message: &ChannelMessage, media: Vec<SidecarMediaPayload>) -> Self {
        Self {
            contract_version: contract_version(),
            session_id: message.session_id.clone(),
            channel: message.channel.clone(),
            content: message.content.clone(),
            chat_id: message.chat_id.clone(),
            is_active: message.is_active,
            kind: message.kind,
            context: SidecarContext::from_map(message.context.clone()),
            output_channel: message.output_channel.clone(),
            media,
        }
    }

    pub fn into_channel_message(self) -> ChannelMessage {
        ChannelMessage {
            session_id: self.session_id,
            channel: self.channel,
            content: self.content,
            chat_id: self.chat_id,
            is_active: self.is_active,
            kind: self.kind,
            context: self.context.into_map(),
            media: Vec::new(),
            output_channel: self.output_channel,
        }
    }

    pub fn has_supported_contract_version(&self) -> bool {
        self.contract_version == SIDECAR_CONTRACT_VERSION
    }
}

pub fn contract_version() -> String {
    SIDECAR_CONTRACT_VERSION.to_owned()
}

pub fn schema_bundle() -> Value {
    serde_json::json!({
        "$schema": SIDECAR_SCHEMA_DRAFT,
        "$id": SIDECAR_SCHEMA_ID,
        "title": "Eli Sidecar Contract",
        "contract_version": SIDECAR_CONTRACT_VERSION,
        "definitions": {
            "channel_message": schema_value::<SidecarChannelMessage>(),
            "tool_request": schema_value::<SidecarToolRequest>(),
            "notice_request": schema_value::<SidecarNoticeRequest>(),
        },
    })
}

fn schema_value<T: JsonSchema>() -> Value {
    serde_json::to_value(schema_for!(T)).unwrap_or(Value::Null)
}

fn take_string(extra: &mut Map<String, Value>, key: &str) -> Option<String> {
    let value = extra.remove(key)?;
    match value {
        Value::String(string) => Some(string),
        other => {
            extra.insert(key.to_owned(), other);
            None
        }
    }
}

fn take_bool(extra: &mut Map<String, Value>, key: &str) -> bool {
    let value = match extra.remove(key) {
        Some(value) => value,
        None => return false,
    };
    match value {
        Value::Bool(flag) => flag,
        other => {
            extra.insert(key.to_owned(), other);
            false
        }
    }
}

fn take_media(extra: &mut Map<String, Value>, key: &str) -> Vec<SidecarMediaPayload> {
    let value = match extra.remove(key) {
        Some(value) => value,
        None => return Vec::new(),
    };
    match serde_json::from_value(value.clone()) {
        Ok(media) => media,
        Err(_) => {
            extra.insert(key.to_owned(), value);
            Vec::new()
        }
    }
}

fn insert_string(extra: &mut Map<String, Value>, key: &str, value: Option<String>) {
    if let Some(value) = value {
        extra.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_media(extra: &mut Map<String, Value>, key: &str, value: Vec<SidecarMediaPayload>) {
    if value.is_empty() {
        return;
    }
    extra.insert(key.to_owned(), serde_json::json!(value));
}

fn insert_bool(extra: &mut Map<String, Value>, key: &str, value: bool) {
    if value {
        extra.insert(key.to_owned(), Value::Bool(true));
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;

    use serde_json::{Map, json};

    use super::*;
    use crate::channels::message::ChannelMessage;

    #[test]
    fn test_sidecar_context_round_trip_preserves_known_fields_and_extra() {
        let mut raw = Map::new();
        raw.insert("source_channel".into(), json!("feishu"));
        raw.insert("account_id".into(), json!("default"));
        raw.insert("_eli_cleanup_only".into(), json!(true));
        raw.insert("channel".into(), json!("$webhook"));

        let round_trip = SidecarContext::from_map(raw).into_map();

        assert_eq!(round_trip.get("source_channel"), Some(&json!("feishu")));
        assert_eq!(round_trip.get("account_id"), Some(&json!("default")));
        assert_eq!(round_trip.get("_eli_cleanup_only"), Some(&json!(true)));
        assert_eq!(round_trip.get("channel"), Some(&json!("$webhook")));
    }

    #[test]
    fn test_sidecar_channel_message_defaults_contract_version() {
        let message = ChannelMessage::new("mock:default:u1", "webhook", "hello")
            .with_chat_id("u1")
            .finalize();

        let payload = SidecarChannelMessage::from_channel_message(&message, Vec::new());

        assert_eq!(payload.contract_version, SIDECAR_CONTRACT_VERSION);
    }

    #[test]
    fn test_schema_bundle_has_versioned_id() {
        let schema = schema_bundle();
        assert_eq!(schema["$id"], SIDECAR_SCHEMA_ID);
        assert_eq!(schema["contract_version"], SIDECAR_CONTRACT_VERSION);
    }

    #[test]
    fn test_schema_bundle_matches_committed_schema() {
        assert_eq!(schema_bundle(), read_json("eli-sidecar.schema.json"));
    }

    #[test]
    fn test_channel_message_fixture_round_trips() {
        let fixture = read_json("channel-message.json");
        let payload: SidecarChannelMessage = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(serde_json::to_value(payload).unwrap(), fixture);
    }

    #[test]
    fn test_tool_request_fixture_round_trips() {
        let fixture = read_json("tool-request.json");
        let payload: SidecarToolRequest = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(serde_json::to_value(payload).unwrap(), fixture);
    }

    #[test]
    fn test_notice_request_fixture_round_trips() {
        let fixture = read_json("notice-request.json");
        let payload: SidecarNoticeRequest = serde_json::from_value(fixture.clone()).unwrap();
        assert_eq!(serde_json::to_value(payload).unwrap(), fixture);
    }

    fn read_json(name: &str) -> Value {
        let path = contract_dir().join(name);
        let text = fs::read_to_string(path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn contract_dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../sidecar/contracts/v1")
    }
}
