<!-- /autoplan restore point: /Users/bytedance/.gstack/projects/cklxx-eli/main-autoplan-restore-20260324-202543.md -->
# eli 控制平面战略：从 OpenClaw 教训中构建 agent 执行保障

> 最后更新：2026-03-24
> /autoplan review: COMPLETE (CEO + Eng, dual voices)
> 战略方向：Control Plane（经 CEO review 从 "Runtime Replacement" 转向）

---

## 一、为什么不做 OpenClaw 兼容

原始计划提议把 eli 定位为"OpenClaw 的 Rust runtime"——全面协议兼容，零迁移成本。经过 CEO review（Codex + Claude 独立审查，6/6 维度不同意），该方向被否决。原因：

1. **SKILL.md "几乎相同" 是假的**：eli 解析 2 个字段（name, description），OpenClaw 有 10+ 字段 + gating + capabilities。格式外形相似，执行语义完全不同
2. **解决了错误的问题**：用户真正的痛是 token 浪费（$400 测试成本）和 agent 失控（over-autonomy），不是启动速度（6s → 100ms）
3. **永久从属**：做 runtime 替代意味着 OpenClaw 协议 v3 → v4 变成 eli 的紧急事件。路线图被上游控制
4. **安全论点站不住**："Rust + hook 拦截"是政策，不是沙箱。没有能力边界、签名、隔离，"安全 runtime"是空话
5. **IronClaw 已在做**：10.7K stars，Transformer 论文共同作者，问题都是可修复的。正面竞争"Rust OpenClaw"是劣势局

## 二、新方向：Agent 控制平面

### 核心叙事

> eli 不克隆 OpenClaw。它解决 OpenClaw 解决不了的问题：
> **agent 花了多少钱？做了什么决策？为什么调了这个工具？怎么阻止它失控？**
>
> 预算控制。执行可见。安全拦截。每一步可审计。
> 如果你有 OpenClaw 的 skills，可以导入。但那不是重点。

### 为什么这个方向更好

| 维度 | Runtime Replacement (旧) | Control Plane (新) |
|------|--------------------------|-------------------|
| 路线图 | 被 OpenClaw 控制 | 独立 |
| 用户痛点 | 启动速度（弱痛） | Token 浪费 + 失控（强痛） |
| 差异化 | 性能（可被追上） | 执行保障（架构级） |
| 风险 | 协议追赶、兼容性 bug | 需要证明控制平面有市场 |
| 可逆性 | 低（深度耦合） | 高（都是 hook，可移除） |

---

## 三、竞争定位

eli 不对标 OpenClaw 或 IronClaw。eli 的竞品是"没有控制的 agent 执行"。

| 问题 | OpenClaw | IronClaw | eli |
|------|----------|----------|-----|
| Agent 花了多少 token？ | 不知道 | 不知道 | **实时追踪 + 预算上限** |
| Agent 为什么调了这个工具？ | 看日志 | 看日志 | **hook 执行链可视化** |
| 怎么阻止 agent 调危险工具？ | 配置文件 | WASM 沙箱 | **deny-by-default + 审批 hook** |
| Agent 跑了 20 轮还没完？ | 等着 | 等着 | **预算耗尽 → 优雅停止** |

---

## 四、技术架构：需要什么改造

### 现状问题（Eng Review 发现的 P0 级架构阻塞）

1. **Hook 语义不支持"包裹"**：`run_model` 是 first-result-wins，控制平面 hook 无法在不改语义的情况下包裹内置执行
2. **Token 使用数据不过 hook 边界**：`run_model` 只返回文本，usage 数据在 conduit 的 `run_tools` 内被覆盖，多轮工具调用只保留最后一轮
3. **`wrap_tool` 不能移除工具**：返回 `None` 表示"不改"而不是"移除"，且 `call_wrap_tools` 在主路径中是死代码
4. **无中途取消机制**：conduit 的 `run_tools` 循环没有 `CancellationToken`，预算耗尽无法中止

### 目标架构

```
Channel ──▶ Framework ──▶ ControlPlane ──▶ Builtin Agent ──▶ Conduit LLM
                              │                                    │
                         ┌────┴────┐                          ┌────┴────┐
                         │ RESERVED│ ← 不参与 last-wins       │ Usage   │
                         │ HOOKS   │   在所有 plugin 之前/后   │ Events  │
                         │ • budget│   运行，不可被覆盖        │ per API │
                         │ • safety│                           │ call    │
                         │ • audit │                           └─────────┘
                         └─────────┘
```

关键变更：引入"reserved hook"层级，在 `last-registered-wins` 之外运行。控制平面 hook 包裹整个 plugin 链，而不是参与其中。

---

## 五、实施路径

### Phase 0：基础设施改造（2-3 周）

**目标**：让控制平面功能在架构上成为可能。

| 改动 | 位置 | 描述 |
|------|------|------|
| `UsageEvent` | conduit | 每次 API 调用 emit usage 事件（tokens_in, tokens_out, model, cost_estimate） |
| `CancellationToken` | conduit `run_tools` | 支持中途取消工具循环 |
| Reserved hook 层级 | eli framework | 新增 `ReservedHookSpec` trait，在所有 plugin 之前/后运行，不可覆盖 |
| 修复 `call_wrap_tools` | eli framework | 内置 agent 必须通过 HookRuntime 调用 wrap_tools |
| `ToolAction` enum | eli hooks | `wrap_tool` 返回 `Keep/Remove/Replace(Tool)` 替代 `Option<Tool>` |
| Atomic budget ledger | eli framework | 预算状态独立于 `State` HashMap，用 `AtomicU64` 防并发竞争 |

### Phase 1：控制平面功能（3-4 周）

| 功能 | 描述 |
|------|------|
| **Token Budget Hook** | post-turn 执行，累计 usage，软停止（完成当前 turn 后阻止下一个）。per-session / per-day 粒度。预算耗尽 → 返回 "Budget exceeded" 消息 |
| **Safety Interceptor** | deny-by-default 工具过滤。用 `ToolAction::Remove` 移除不在白名单的工具 schema（模型看不到 = 不会调）。slash commands 也必须过审批链 |
| **Execution Observer** | 结构化事件流（不只是 tracing），`on_hook_event` observer hook，覆盖 hook 层 + conduit 层（provider 请求/重试/工具轮次） |
| **Hook Dashboard** | 实时 TUI 显示每个 turn 的 hook 执行链、token 消耗、延迟、决策审计。基于 Execution Observer 事件流 |

### Phase 2：便利功能（2-3 周）

| 功能 | 描述 |
|------|------|
| **openclaw-import CLI** | 严格子集：只转换 name + description + body。不支持 gating/requires/install。未知字段 → 警告 + 跳过。dry-run 模式。幂等 |
| **Pre-turn token estimation** | 基于 Phase 1 的实际使用数据反馈，添加发送前估算。先支持 OpenAI tokenizer（tiktoken-rs），其他 provider 用近似值 |

---

## 六、设计决策（已解决）

### Q1: Budget 主体 → session_id，config 留 global scope

session_id = `{channel}:{chat_id}`，是现有唯一标识。跨 channel 共享预算需要 user identity 系统（另一个项目）。

```rust
pub enum BudgetScope {
    Session,  // per session_id, default
    Global,   // shared across all sessions, AtomicU64 单例
}
```

### Q2: 结算制，失败 attempt 也计入

usage 数据只有 post-turn 才有（`ToolAutoResult::usage`）。预留制需要 multi-provider tokenizer，Phase 0 做不了。conduit 改造：每次 attempt（包括失败的）都 emit `UsageEvent`。

```rust
pub struct UsageEvent {
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub attempt: u32,       // 0-indexed
    pub success: bool,      // false = failed attempt, still counted
    pub timestamp: String,
}
```

Budget 检查流程：
```
Turn N:   run_model → 返回 → 累计 usage → 超限 → save_state(budget_exceeded=true)
Turn N+1: ControlPlane pre-check → 读到 exceeded → 短路 → "Budget exceeded" 消息
```

### Q3: Reserved hooks 不可覆盖，framework 级硬编码

不改 `EliHookSpec` trait。`ControlPlane` 在 `process_inbound` 里硬编码在 plugin 链之前/之后。Plugin 看不到它，改不了它。

```rust
pub struct ControlPlane {
    budget: AtomicBudgetLedger,
    safety: SafetyPolicy,
    observer: EventBus,
}

// process_inbound 内部：
// 1. control_plane.pre_check()    ← reserved, non-bypassable
// 2. run_plugin_chain()           ← normal last-registered-wins
// 3. control_plane.post_record()  ← reserved, non-bypassable
```

### Q4: Tape events + 内存 ring buffer

不用 tracing（无 redaction，给开发者 debug 的）。Tape 已有 `"event"` kind 和查询能力。Dashboard 用内存 ring buffer（`VecDeque<HookEvent>`，最近 50 条）做实时流。

```rust
pub struct HookEvent {
    pub hook: &'static str,     // "run_model", "wrap_tool", etc.
    pub plugin: String,
    pub duration_ms: u64,
    pub result: HookResult,     // Ok/Err/Skipped
    pub usage: Option<UsageEvent>,
    pub payload_preview: String, // redacted: strip API keys, truncate >200 chars
}
```

Redaction：`sk-*`/`key-*` → `"[REDACTED]"`，长文本截断。Tape 存 redacted 版本。

### 日重置时区 → UTC

全局统一，避免时区歧义。配置不暴露此选项。

---

## 七、NOT in scope

- OpenClaw 全协议兼容（被 CEO review 否决）
- WebSocket Gateway 协议实现
- WhatsApp / Discord / Slack channel 扩展（commodity trap）
- OpenClaw Plugin SDK 兼容
- WASM 工具沙箱（IronClaw 的路线，可后续评估）
- Cost comparison mode（需要本地安装 OpenClaw）
- conduit Python/JS bindings（已在 TODOS.md P3）

---

## 八、一句话

**eli 不是另一个 OpenClaw。它是你套在任何 agent 上的安全带：预算控制、执行可见、工具审批。OpenClaw 的用户花 $400 发现 agent 失控——eli 让那件事不可能发生。**

---

## Decision Audit Trail

| # | Phase | Decision | Principle | Rationale | Rejected |
|---|-------|----------|-----------|-----------|----------|
| 1 | CEO | Mode: SELECTIVE EXPANSION | P1 | Strategic pivot needs rigorous baseline | EXPANSION, HOLD |
| 2 | CEO | Approach B (Control Plane) over A (Runtime) | P3 | Both voices + user confirmed — solves actual pain | A (dependency chain) |
| 3 | CEO | Add hook execution dashboard to scope | P1+P2 | This IS the control plane differentiator | Defer |
| 4 | CEO | Defer cost comparison mode | P3 | Requires OpenClaw installed — external dependency | Include |
| 5 | CEO | Soft stop default for budget exceeded | P5 | Explicit, predictable behavior | Hard stop (data loss risk) |
| 6 | Eng | Reserved hook tier (not last-wins) | P5 | Control plane must be non-bypassable | Participate in plugin chain |
| 7 | Eng | Post-turn budget enforcement first | P3 | Pre-turn needs multi-provider tokenizer — defer | Pre-turn estimation |
| 8 | Eng | openclaw-import strict subset only | P4 | Prevent scope creep back into compatibility | Full field support |
| 9 | Eng | Atomic budget ledger | P5 | State HashMap is mutable by any plugin | Shared State |
| 10 | Eng | ToolAction enum for wrap_tool | P1 | Current None=unchanged can't express removal | Keep Option<Tool> |

---

## Cross-Phase Themes

**Theme 1: "Hook-first" is a claim, not yet a fact** — flagged in CEO + Eng.
Both phases independently found that eli's hook system, while well-designed for extensibility, lacks the semantics needed for enforcement. First-result-wins is great for "who handles this?" but wrong for "should this be allowed?" The reserved hook tier is the architectural pivot that makes "hook-first" true for security and budgeting, not just for feature composition.

**Theme 2: Solving proxy problems instead of real ones** — flagged in CEO + Eng.
CEO: plan leads with performance (proxy) instead of cost control (real pain). Eng: plan leads with protocol compatibility (proxy) instead of execution guarantees (real value). Both converge: start from the user's $400 wasted-token story, not from the benchmark.

**Theme 3: Conduit boundary is load-bearing** — flagged in Eng (both voices).
Token budget, mid-turn cancellation, and usage visibility all require conduit changes. The dual-crate split is an advantage for modularity but means control plane features can't be "just hooks" — they need conduit cooperation. Phase 0 infrastructure work is non-negotiable.

---

## GSTACK REVIEW REPORT

| Review | Trigger | Why | Runs | Status | Findings |
|--------|---------|-----|------|--------|----------|
| CEO Review | `/plan-ceo-review` | Scope & strategy | 1 | issues_found | 5 false premises, strategic pivot to Control Plane |
| CEO Voices | `/autoplan` dual | Independent challenge | 1 | codex+subagent | 0/6 confirmed, full strategic redirect |
| Eng Review | `/plan-eng-review` | Architecture & tests | 1 | issues_found | 3 P0 architectural blockers, 6 critical gaps |
| Eng Voices | `/autoplan` dual | Independent challenge | 1 | codex+subagent | 1/6 confirmed, needs control-plane refactor |
| Design Review | `/plan-design-review` | UI/UX gaps | 0 | skipped | No UI scope detected |

**VERDICT:** REVIEWED — strategic pivot applied, architectural blockers identified, revised phasing written. Ready for approval.
