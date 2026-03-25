//! Incremental SSE frame decoding.

use serde_json::Value;

/// Incrementally decodes SSE `data:` frames into JSON values.
#[derive(Debug, Default)]
pub struct SseDecoder {
    buffer: String,
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
        self.buffer.push_str(&String::from_utf8_lossy(bytes));
        self.drain()
    }

    pub fn finish(&mut self) -> Vec<Value> {
        if self.finished {
            return Vec::new();
        }
        self.buffer.push('\n');
        self.drain()
    }

    fn drain(&mut self) -> Vec<Value> {
        if self.finished {
            self.buffer.clear();
            return Vec::new();
        }
        let mut events = Vec::new();
        let mut start = 0;
        while let Some(line_end) = self.buffer[start..].find('\n') {
            let line_end = start + line_end;
            let line = self.buffer[start..line_end].trim_end_matches('\r');
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
            self.buffer = self.buffer[start..].to_owned();
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
}
