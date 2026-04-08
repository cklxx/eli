//! Incremental SSE frame decoding.

use serde_json::Value;

/// Incrementally decodes SSE `data:` frames into JSON values.
///
/// Uses a raw byte buffer internally to avoid corrupting multibyte UTF-8
/// characters that may be split across chunk boundaries.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: Vec<u8>,
    finished: bool,
}

impl SseDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, bytes: &[u8]) -> Vec<Value> {
        if self.finished {
            return Vec::new();
        }
        self.buffer.extend_from_slice(bytes);
        self.drain()
    }

    pub fn finish(&mut self) -> Vec<Value> {
        if self.finished {
            return Vec::new();
        }
        self.buffer.push(b'\n');
        self.drain()
    }

    fn drain(&mut self) -> Vec<Value> {
        if self.finished {
            self.buffer.clear();
            return Vec::new();
        }
        let mut events = Vec::new();
        let mut start = 0;
        while let Some(rel) = self.buffer[start..].iter().position(|&b| b == b'\n') {
            let line_end = start + rel;
            let mut end = line_end;
            if end > start && self.buffer[end - 1] == b'\r' {
                end -= 1;
            }
            // Decode only complete lines — partial multibyte sequences stay
            // buffered for the next chunk.
            let line = String::from_utf8_lossy(&self.buffer[start..end]);
            if let Some(data) = line.strip_prefix("data: ") {
                if data == "[DONE]" {
                    self.finished = true;
                    break;
                }
                if let Ok(event) = serde_json::from_str::<Value>(data) {
                    events.push(event);
                }
            }
            start = line_end + 1;
        }
        if self.finished {
            self.buffer.clear();
        } else {
            self.buffer.drain(..start);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_push_decodes_split_frames() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(b"data: {\"a\":").is_empty());
        let events = decoder.push(b"1}\n");
        assert_eq!(events, vec![json!({"a": 1})]);
    }

    #[test]
    fn test_finish_stops_after_done() {
        let mut decoder = SseDecoder::new();
        assert!(decoder.push(b"data: {\"a\": 1}\n").len() == 1);
        assert!(decoder.push(b"data: [DONE]\ndata: {\"b\": 2}\n").is_empty());
        assert!(decoder.push(b"data: {\"c\": 3}\n").is_empty());
        assert!(decoder.finish().is_empty());
    }

    #[test]
    fn test_multibyte_utf8_split_across_chunks() {
        // The euro sign (€) is U+20AC, encoded as 3 bytes: [0xE2, 0x82, 0xAC].
        // Split the SSE line so the first chunk ends mid-character.
        let mut decoder = SseDecoder::new();

        // "data: {\"c\":\"\xe2" — first byte of € but line is incomplete
        let mut chunk1 = b"data: {\"c\":\"\xE2".to_vec();
        assert!(
            decoder.push(&chunk1).is_empty(),
            "incomplete line should not decode"
        );

        // Remaining 2 bytes of €, closing JSON, and newline
        chunk1 = b"\x82\xAC\"}\n".to_vec();
        let events = decoder.push(&chunk1);
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]["c"].as_str().unwrap(),
            "\u{20AC}",
            "euro sign must survive chunk split"
        );
    }

    #[test]
    fn test_crlf_line_endings() {
        let mut decoder = SseDecoder::new();
        let events = decoder.push(b"data: {\"ok\":true}\r\n");
        assert_eq!(events, vec![json!({"ok": true})]);
    }
}
