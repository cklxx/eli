//! Scoped tape session that binds an LLM to a named tape.

use serde_json::Value;

use crate::core::errors::ConduitError;
use crate::core::results::ToolAutoResult;
use crate::llm::{ChatRequest, LLM};
use crate::tape::entries::TapeEntry;

/// A convenience wrapper that pairs an [`LLM`] reference with a tape name,
/// so that every call automatically records to (and reads from) the tape.
pub struct TapeSession<'a> {
    llm: &'a mut LLM,
    tape: String,
}

impl<'a> TapeSession<'a> {
    /// Create a new session bound to the given `tape` name.
    pub fn new(llm: &'a mut LLM, tape: impl Into<String>) -> Self {
        Self {
            llm,
            tape: tape.into(),
        }
    }

    /// The tape name this session is bound to.
    pub fn tape_name(&self) -> &str {
        &self.tape
    }

    /// Chat with the model, recording the exchange to the session tape.
    ///
    /// The `tape` field of the request is overridden with this session's tape name.
    pub async fn chat(&mut self, mut req: ChatRequest<'_>) -> Result<String, ConduitError> {
        req.tape = Some(&self.tape);
        self.llm.chat_async(req).await
    }

    /// Run tools, recording the exchange to the session tape.
    ///
    /// The `tape` field of the request is overridden with this session's tape name.
    pub async fn run_tools(
        &mut self,
        mut req: ChatRequest<'_>,
    ) -> Result<ToolAutoResult, ConduitError> {
        req.tape = Some(&self.tape);
        self.llm.run_tools(req).await
    }

    /// Append a raw [`TapeEntry`] to the session tape.
    pub async fn append(&mut self, entry: &TapeEntry) -> Result<(), ConduitError> {
        self.llm.append_tape_entry(&self.tape, entry).await
    }

    /// Record a handoff (anchor + event) to the session tape.
    pub async fn handoff(
        &mut self,
        name: &str,
        state: Option<Value>,
        meta: Value,
    ) -> Result<Vec<TapeEntry>, ConduitError> {
        self.llm.handoff_tape(&self.tape, name, state, meta).await
    }
}
