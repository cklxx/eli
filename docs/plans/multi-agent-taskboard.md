<!-- /autoplan restore point: /Users/bytedance/.gstack/projects/cklxx-eli/main-autoplan-restore-20260408-173416.md -->

# Multi-Agent Task Board Architecture for Eli

## Context

Eli is a turn-based, request-response agent framework. Each inbound message triggers one turn through the hook pipeline, producing one response. The existing `AgentTracker` + `agent.*` tools provide fire-and-forget background agents, but there is no persistent task management, no inter-agent coordination, and no pipeline orchestration.

**Goal:** Add a persistent task board (kanban) that organizes agent collaboration. Conversation remains the primary interface. Agents create TODOs, TODOs are automatically consumed by appropriate workers, results flow back.

**User constraints:**
1. Budget = monitoring only, no enforcement
2. Task board system DECOUPLED from the current agent framework

## Revised Design (post-review)

Both CEO and Eng review phases (4 independent reviewers: 2 Claude subagents + 2 Codex) converged on three recommendations:

1. **Unbundle** — ship persistent task ledger first, defer pipeline engine and worker pool
2. **Decouple** — standalone module, not embedded in EliFramework or EliHookSpec
3. **Extend, don't replace** — build on existing `AgentTracker` patterns, not a parallel system

### Architecture: Standalone Task Ledger

```
┌─────────────────────────────────────────────────┐
│ crates/eli/src/taskboard/  (standalone module)   │
│                                                  │
│  TaskLedger (SQLite-backed, single source of truth) │
│    ├── create(task) -> TaskId                     │
│    ├── claim(id, agent_id) -> Result             │
│    ├── update_status(id, status) -> Result        │
│    ├── complete(id, result) -> Result             │
│    ├── fail(id, error) -> Result                  │
│    ├── list(filter) -> Vec<Task>                  │
│    ├── get(id) -> Option<Task>                    │
│    └── subscribe() -> Receiver<TaskEvent>         │
│                                                  │
│  Runs on dedicated thread (rusqlite is sync)     │
│  No imports from framework.rs, hooks.rs          │
└──────────────┬───────────────────────────────────┘
               │ thin adapter
┌──────────────▼───────────────────────────────────┐
│ crates/eli/src/builtin/taskboard_plugin.rs       │
│                                                  │
│  Registers task.* tools into existing REGISTRY    │
│  Subscribes to TaskEvent, renders as channel msgs │
│  Registers `eli task` CLI subcommands             │
│  Budget monitoring via existing BudgetLedger      │
└──────────────────────────────────────────────────┘
```

**Key principle:** `taskboard/` has ZERO imports from `framework.rs` or `hooks.rs`. It is a standalone subsystem that the plugin adapter bridges into eli. Same pattern as nexil being standalone from eli.

### Task Data Model

```rust
// crates/eli/src/taskboard/mod.rs

pub struct Task {
    pub id: TaskId,                     // uuid
    pub kind: String,                   // free-form, not enum (avoid calcification)
    pub status: Status,
    pub parent: Option<TaskId>,
    pub session_origin: String,         // traces to originating conversation
    pub context: Value,                 // task input
    pub result: Option<Value>,          // task output
    pub assigned_to: Option<String>,    // agent id
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub priority: u8,                   // 0=low, 1=normal, 2=high, 3=urgent
    pub metadata: Value,                // extensible fields
}

pub enum Status {
    Todo,
    Claimed { agent_id: String, claimed_at: DateTime<Utc> },
    Running { progress: f32, last_heartbeat: DateTime<Utc> },
    Done,
    Failed {
        error: String,
        agent_id: Option<String>,
        stage: Option<String>,
        tool_trace: Vec<String>,
        retries: u32,
        suggested_fix: Option<String>,
    },
    Blocked { reason: String, waiting_on: Option<TaskId> },
    Cancelled { reason: String },
}

pub enum TaskEvent {
    Created(TaskId),
    StatusChanged { id: TaskId, from: Status, to: Status },
    Completed { id: TaskId, result: Value },
    Failed { id: TaskId, error: String },
}
```

**Changes from original plan:**
- `TaskKind` is `String` not enum (Claude subagent: "will calcify")
- `Failed` includes agent_id, stage, tool_trace, suggested_fix (DX: actionable errors)
- `Claimed` has claimed_at for lease expiry (Codex: thundering herd)
- `Running` has last_heartbeat for stuck task detection
- Added `Cancelled` status
- No `DashMap` — SQLite is single source of truth (Eng consensus)

### Persistence: SQLite on Dedicated Thread

```rust
// crates/eli/src/taskboard/store.rs

pub struct TaskStore {
    sender: mpsc::Sender<StoreCommand>,
}

enum StoreCommand {
    Create(Task, oneshot::Sender<Result<TaskId>>),
    Get(TaskId, oneshot::Sender<Option<Task>>),
    List(TaskFilter, oneshot::Sender<Vec<Task>>),
    UpdateStatus(TaskId, Status, oneshot::Sender<Result<()>>),
    Subscribe(broadcast::Sender<TaskEvent>),
}
```

- Dedicated `std::thread` running SQLite event loop (rusqlite is sync)
- `mpsc::Sender` for commands from async tokio tasks
- `oneshot::Sender` for responses back
- WAL mode for concurrent reads
- Claims are atomic: `UPDATE tasks SET status = 'claimed' WHERE id = ? AND status = 'todo'`

### CLI Surface (human-first)

```
eli task add "refactor auth module"           # create task from CLI
eli task add --kind explore "find all callers" # with kind
eli task list                                  # show all tasks
eli task list --status running                 # filter by status
eli task show <id>                             # task details + history
eli task cancel <id>                           # cancel a task
eli task board                                 # kanban-style view
```

**Rationale (DX review):** Both voices said don't make "hello world" depend on the model deciding to call `task.create`. Human CLI surface first, model tools second.

### Model Tools

```
task.create   — create a task (available to interactive agent)
task.status   — query task status
task.list     — list tasks with filters
task.cancel   — cancel a task
task.update   — update task progress/result
```

These follow the existing `agent.*` naming pattern. The `agent.*` tools remain for simple background work; `task.*` adds persistence and lifecycle tracking.

**Boundary:** `agent.*` = fire-and-forget subprocesses. `task.*` = persistent, queryable, observable work units. `agent.run` can still be used for quick background jobs. `task.create` is for work that needs tracking, resumability, and status reporting.

### Budget Monitoring (not enforcement)

```rust
// In taskboard_plugin.rs adapter

struct TaskBudgetMonitor {
    ledger: Arc<BudgetLedger>,  // existing
    metrics: TaskMetrics,
}

struct TaskMetrics {
    tasks_created: AtomicU64,
    tasks_completed: AtomicU64,
    tasks_failed: AtomicU64,
    total_tokens_spent: AtomicU64,
    active_workers: AtomicU64,
}
```

Monitor subscribes to TaskEvents, records metrics. Available via `eli task stats`. No enforcement, no blocking.

### Rate Limiting on task.create

To prevent prompt injection spawning unbounded tasks:
- Max 20 tasks per session per minute
- Max 100 total active tasks (configurable via `ELI_MAX_TASKS`)
- Max task depth: 5 levels of parent→child

## Implementation: Phase 1 Only

**Scope:** Persistent task ledger + CLI + model tools + single serial worker. NO pipeline engine, NO worker pool, NO agent-to-agent messaging.

### New files:
- `crates/eli/src/taskboard/mod.rs` — Task, Status, TaskEvent types
- `crates/eli/src/taskboard/store.rs` — SQLite persistence on dedicated thread
- `crates/eli/src/taskboard/worker.rs` — Single serial worker (consumes one task at a time)
- `crates/eli/src/builtin/taskboard_plugin.rs` — Plugin adapter (tools, CLI, monitoring)

### Modified files:
- `crates/eli/src/builtin/mod.rs` — register taskboard plugin
- `crates/eli/src/builtin/cli/mod.rs` — register `eli task` subcommands
- `crates/eli/Cargo.toml` — add rusqlite dependency

### NOT modified (decoupling):
- `framework.rs` — NO changes
- `hooks.rs` — NO changes (no new hook points)
- `control_plane.rs` — NO changes

### Worker (Phase 1: serial, single)

```rust
// crates/eli/src/taskboard/worker.rs

pub struct TaskWorker {
    store: TaskStore,
    capabilities: Vec<String>,
}

impl TaskWorker {
    pub async fn run(&self, cancel: CancellationToken) {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(Duration::from_secs(2)) => {
                    if let Some(task) = self.store.claim_next(&self.capabilities).await {
                        self.execute(task).await;
                    }
                }
            }
        }
    }
}
```

Phase 1 worker: poll every 2s, claim one task, execute via `process_inbound` with synthetic envelope, report result. Worktree isolation for code-modifying tasks (reuse existing `worktree.rs`).

## Deferred to Phase 2+ (evidence-gated)

Only build these AFTER Phase 1 proves valuable through real usage:

| Item | Trigger to build | Effort |
|---|---|---|
| Pipeline engine (stages, gates) | User manually chains tasks 3+ times | L |
| Worker pool (N concurrent) | Serial worker becomes bottleneck | M |
| Agent-to-agent messaging | Workers need to coordinate without board | M |
| YAML pipeline definitions | Pipeline engine ships and users want customization | M |
| Long-lived monitor agents | Specific monitoring use case emerges | M |
| Sub-task failure policies (AllOrNothing, BestEffort) | Compound tasks are actually used | S |

## Key Decisions (resolved)

1. **Persistence**: SQLite, dedicated thread, WAL mode
2. **Worker isolation**: Worktree per code-modifying task (reuse existing)
3. **Pipeline definition**: DEFERRED — build when needed
4. **Human gates**: DEFERRED — all tasks auto-complete in Phase 1
5. **Worker lifecycle**: Spawn-on-demand, single serial worker
6. **TaskKind**: Free-form String, not enum
7. **Budget**: Monitoring only, no enforcement
8. **Decoupling**: Standalone module + thin plugin adapter

## NOT in Scope

- Pipeline engine (deferred, speculative)
- Worker pool concurrency (deferred, premature)
- Agent-to-agent messaging (deferred, not needed for Phase 1)
- New hook points in EliHookSpec (decoupling constraint)
- TaskBoard as EliFramework member (decoupling constraint)
- Existing DX fixes (install path, docs, error messages — separate effort)
- Existing P0/P1 bugs (lock poisoning, tool loop — tracked in architecture-future-roadmap.md)

## Verification

```bash
cargo test --workspace                        # existing tests still pass
cargo test -p eli -- taskboard                # new taskboard tests
cargo clippy --workspace -- -D warnings       # lint clean
```

### Test plan:
1. Task CRUD: create, get, list, filter, cancel
2. Status transitions: todo → claimed → running → done/failed
3. SQLite persistence: create tasks, restart process, tasks survive
4. Claim atomicity: two workers claiming same task, only one succeeds
5. Rate limiting: >20 tasks/min rejected
6. Depth limiting: >5 levels of nesting rejected
7. Worker lifecycle: start, claim, execute, complete, poll again
8. Crash recovery: worker dies mid-task, task returns to todo after heartbeat timeout
9. CLI commands: `eli task add/list/show/cancel/board`
10. Model tools: `task.create/status/list/cancel/update` via LLM

## Decision Audit Trail

| # | Phase | Decision | Classification | Principle | Rationale | Rejected |
|---|-------|----------|---------------|-----------|-----------|----------|
| 1 | CEO | Mode: SELECTIVE EXPANSION | Mechanical | P3 | Feature enhancement | EXPANSION |
| 2 | CEO | Accept premises 1-3 | — | — | User confirmed | — |
| 3 | CEO | Approach B (lightweight) over A (full) | Taste | P5+P3 | Start minimal, add based on usage | Full pipeline from day 1 |
| 4 | CEO | Surface as User Challenge | — | — | Both models say descope significantly | Auto-approve |
| 5 | CEO | Accept "decoupled" constraint | Mechanical | P5 | User requested | Embed in framework |
| 6 | CEO | Accept "monitoring only" budget | Mechanical | P6 | User requested | Budget enforcement |
| 7 | CEO | Defer pipeline engine | Taste | P3+P5 | Both voices say speculative | Ship pipeline Phase 1 |
| 8 | CEO | Require concrete use cases | Taste | P1 | Both flag no product thesis | Build generic infra |
| 9 | ENG | Standalone singleton, not framework member | Mechanical | P5 | User + both voices | Embed in EliFramework |
| 10 | ENG | SQLite single source of truth | Mechanical | P5 | Two truth sources = bugs | DashMap + SQLite |
| 11 | ENG | Defer pipeline engine entirely | Taste | P3 | Speculative | Pipeline Phase 2 |
| 12 | ENG | rusqlite on dedicated thread | Mechanical | P5 | Blocking tokio = disaster | rusqlite on tokio |
| 13 | ENG | Rate limit task.create | Mechanical | P1 | Unbounded spawn risk | No rate limit |
| 14 | ENG | Crash recovery + idempotent tests | Mechanical | P1 | Happy-path-only unacceptable | Shallow tests |
| 15 | DX | CLI commands before model tools | Mechanical | P5 | Human surface first | Model tools only |
| 16 | DX | Clarify agent.* vs task.* boundary | Mechanical | P1 | Naming collision | Leave ambiguous |
| 17 | DX | Actionable error states | Mechanical | P1 | Bare strings unacceptable | Failed { error: String } |
| 18 | DX | YAML pipeline definitions (deferred) | Taste | P5 | Match SKILL.md pattern | Rust-only |
| 19 | DX | Existing DX fixes separate scope | Taste | P3 | Valid but scope creep | Fix DX in this plan |

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 1 | issues_open | 1 user challenge (scope), 5/6 consensus |
| Codex Review | `/codex review` | Independent 2nd opinion | 3 | issues_found | CEO + Eng + DX voices |
| Eng Review | `/plan-eng-review` | Architecture & tests | 1 | issues_open | 14 findings, 6/6 consensus |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | skipped | No UI scope |
| DX Review | `/plan-devex-review` | Developer experience | 1 | issues_open | Initial 3/10 → 5/10, 6/6 consensus |

**VERDICT:** APPROVED — Phase 1 persistent task ledger. User accepted descoped plan after User Challenge.
