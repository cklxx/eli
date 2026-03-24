# TODOs

## P3: Python/JS bindings for conduit (PyO3/napi-rs)

**What:** Publish conduit with FFI bindings so Python/JS developers can use the tape system and LLM toolkit.

**Why:** Expands addressable market from Rust-only to the entire agent ecosystem. Currently Eli only targets Rust developers building agents — a small intersection.

**Pros:**
- 100x larger audience
- Validates conduit as standalone value
- Enables the "SQLite for agent memory" positioning

**Cons:**
- Significant maintenance burden — two FFI surfaces to maintain
- API must be stable before binding to it
- Testing matrix explodes (Rust + Python + JS)

**Context:** The CEO review identified this as an "ocean" (too big to boil now) but high-value long-term. The Rust API needs to prove the decision layer thesis first. If persistent decisions don't matter to users, bindings don't help.

**Effort:** XL (human) → L with CC+gstack
**Priority:** P3
**Depends on:** Conduit published as standalone crate, decision layer validated with real usage
