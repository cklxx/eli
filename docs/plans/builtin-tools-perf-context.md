# Builtin Tools: Performance & Context Optimization

**Status**: Draft → Review
**Date**: 2026-03-26
**Goal**: Reduce token overhead, lower latency, and improve context-awareness of the 21 builtin tools.

---

## Already Done (skip)

| Item | Status | Evidence |
|------|--------|---------|
| Tool description compression (multi-round) | ✅ Done | `6897399`, `154015f`, `f07740e` — three review passes on descriptions |
| Tool notices config migration | ✅ Done | `9562910` — moved from env var to config.toml |
| Tool notices default off | ✅ Done | `ad493f8` — default off, only webhook+sidecar |
| System prompt truncation framework | ✅ Done | `prompt_builder.rs` — per-section priority + hard cap |
| Sidecar tools as skills (progressive disclosure) | ✅ Done | `1521c44` — sidecar tools exposed as skills, not always-on tools |
| Tool result truncation (spill to disk) | ✅ Done | `crates/nexil/src/tape/spill.rs` — `SpillConfig` with threshold 500 chars, head 15 + tail 5 lines, full output spilled to `{tape}.d/` files |

---

## Remaining Work

### P0: Schema Token Reduction (~2500 tokens saving)

#### 1. Remove `description` (notice) parameter from schemas
11 tools carry `"description": {"type": "string", "description": "Brief user-facing status text..."}`. This is:
- 30 tokens × 11 = **~330 tokens wasted per request**
- Confuses models (clashes with tool's own description field)
- Only used for webhook+sidecar (already default off)

**Change**: Remove from JSON schema. Keep `maybe_send_user_facing_notice` reading `args.get("description")` — models that happen to pass it still work, but the schema doesn't advertise it.

**Risk**: None. Feature is default-off and models rarely generate it unprompted.

**Files**: `crates/eli/src/builtin/tools.rs` (11 schema blocks)

#### 2. Shorten parameter descriptions
Many descriptions repeat what the parameter name already says:

| Current | Proposed |
|---------|----------|
| `"shell_id": {"type": "string", "description": "The background shell ID returned by bash."}` | `"shell_id": {"type": "string"}` |
| `"path": {"type": "string", "description": "File path (absolute or relative to workspace)."}` | `"path": {"type": "string", "description": "Absolute or workspace-relative."}` |
| `"cmd": {"type": "string", "description": "Shell command to execute."}` | `"cmd": {"type": "string"}` |
| `"content": {"type": "string", "description": "Full file content to write."}` | `"content": {"type": "string"}` |
| `"text": {"type": "string", "description": "The decision to record."}` | `"text": {"type": "string"}` |
| `"query": {"type": "string", "description": "Keyword to search for in tape entries."}` | `"query": {"type": "string"}` |

Keep descriptions only where the meaning isn't obvious from context (e.g., `offset` = "0-based line number", `background` = "run async, poll with bash.output").

**Estimated saving**: ~500–800 tokens per request.

**Files**: `crates/eli/src/builtin/tools.rs` (all tool schema blocks)

### P1: ~~Result Truncation~~ ✅ DONE

Already implemented via `SpillConfig` in `crates/nexil/src/tape/spill.rs`:
- Threshold: 500 chars → spill full output to `{tape_name}.d/{call_id}.txt`
- Truncated view: head 15 lines + tail 5 lines + file reference
- Applied in `LLM::maybe_spill_result()` during tape recording

### P2: Lazy Tool Groups (biggest token saving)

#### 4. Group tools, inject only active groups per turn
Instead of sending all 21 schemas, split into groups:

| Group | Tools | Inject when |
|-------|-------|-------------|
| **core** | bash, fs.read, fs.write, fs.edit | Always |
| **tape** | tape.info, tape.search, tape.reset, tape.handoff, tape.anchors | Tape keywords in message or tape state non-trivial |
| **decision** | decision.set, decision.list, decision.remove | Active decisions exist or "decision/decide" in message |
| **net** | web.fetch | URL pattern in message or "fetch/http/api" keyword |
| **lifecycle** | help, quit, skill, message.send, subagent | First turn + on demand |
| **shell** | bash.output, bash.kill | Background shell is running |

**Implementation**:
1. Add `group: &'static str` to `Tool` struct (nexil)
2. Add `ToolGroupResolver` in eli that decides active groups per turn
3. Modify `run_tools_once` to filter by active groups
4. Inject hint in system prompt: "Additional tools: tape.*, decision.*, web.fetch — request if needed"

**Estimated saving**: Most turns send only core (~1200 tokens vs ~5000).

**Risk**: Medium — model may need a hidden tool. Mitigated by hint line + easy activation by mentioning the tool.

**Files**: `crates/nexil/src/tools/schema.rs`, `crates/eli/src/tools.rs`, `crates/eli/src/builtin/agent/agent_request.rs`

### P3: Parallel Tool Execution

#### 5. Execute independent tool calls concurrently
`ToolExecutor::execute_async` runs calls in a sequential `for` loop. When the model requests multiple tool calls in one turn (e.g., two `fs.read`), they could run in parallel.

**Change**:
```rust
// Partition: group by tool name, run groups in parallel, within-group sequential
let groups = partition_by_tool_name(&tool_calls);
let group_futures = groups.into_iter().map(|calls| async {
    let mut results = Vec::new();
    for call in calls {
        results.push(self.handle_tool_call(&call, &tool_map, context).await);
    }
    results
});
let all_results = futures::future::join_all(group_futures).await;
```

Same-tool calls stay sequential (avoids fs.write→fs.read race). Different tools run concurrently.

**Files**: `crates/nexil/src/tools/executor.rs`

**Risk**: Low — side effects between different tools are rare.

### P4: Schema Caching (minor)

#### 6. Cache model-ready tool list
`model_tools()` clones + renames on every call. Since the registry doesn't change after init, cache the result.

**Files**: `crates/eli/src/tools.rs`

**Impact**: ~1ms per turn. Free optimization.

---

## Execution Order

1. **P0.1** Remove `description` param from 11 schemas — 15 min, zero risk
2. **P0.2** Shorten parameter descriptions — 30 min, low risk
3. ~~**P1.3** Auto-truncate results~~ — ✅ already done (spill system)
4. **P2.3** Tool groups — 2-3 hr, needs integration testing
5. **P3.4** Parallel execution — 1 hr
6. **P4.5** Schema caching — 15 min

---

## Expected Outcomes

| Metric | Current | After P0 | After P0+P2 |
|--------|---------|----------|-------------|
| Schema tokens/request | ~5000 | ~3800 | ~1200 |
| Result truncation | ✅ done | ✅ done | ✅ done |
| 2 parallel tool calls latency | 2x | 2x | 1x |

---

## Open Questions

1. **Tool group activation heuristic**: Keyword regex vs explicit phase markers vs model self-request?
2. **Truncation limit**: 16KB (byte-based, fast) vs token-counted (accurate, slow)?
3. **Parallel execution ordering**: Partition by tool name sufficient, or need dependency analysis?
