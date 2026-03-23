//! Scoped tape session that binds an LLM to a named tape.

use serde_json::Value;

use crate::core::errors::ConduitError;
use crate::core::results::ToolAutoResult;
use crate::llm::LLM;
use crate::tape::entries::TapeEntry;
use crate::tools::context::ToolContext;
use crate::tools::schema::ToolSet;

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
    pub async fn chat(
        &mut self,
        prompt: Option<&str>,
        system_prompt: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
        messages: Option<Vec<Value>>,
        max_tokens: Option<u32>,
    ) -> Result<String, ConduitError> {
        self.llm
            .chat_async(
                prompt,
                system_prompt,
                model,
                provider,
                messages,
                max_tokens,
                Some(&self.tape),
            )
            .await
    }

    /// Run tools, recording the exchange to the session tape.
    #[allow(clippy::too_many_arguments)]
    pub async fn run_tools(
        &mut self,
        prompt: Option<&str>,
        system_prompt: Option<&str>,
        model: Option<&str>,
        provider: Option<&str>,
        messages: Option<Vec<Value>>,
        max_tokens: Option<u32>,
        tools: &ToolSet,
        context: Option<&ToolContext>,
    ) -> Result<ToolAutoResult, ConduitError> {
        self.llm
            .run_tools(
                prompt,
                system_prompt,
                model,
                provider,
                messages,
                max_tokens,
                tools,
                context,
                Some(&self.tape),
            )
            .await
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
