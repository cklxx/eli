# TODOs

## P1: 分段式 Prompt Builder + 模式切换

**What:** 把 `build_prompt` 拆成独立的 section builder（身份、工具、技能、记忆、安全、频道格式化、运行时提示等），支持 full/minimal/none 三种模式，加 prompt 总字符硬上限。

**Why:** 当前 prompt 构建是单一步骤，难以按场景裁剪。分段后可按模式选择装哪些段，也方便单独调优每个 section 的大小。

**参考:** elephant.ai 的 `manager_prompt*.go`，18 段组合 + 32K 字符上限 + 每技能体 1500 字符截断。

**Effort:** M
**Priority:** P1

---

## P1: 工具装饰器链（熔断 + 降级）

**What:** 给工具执行加装饰器链：至少包含熔断器（连续失败后自动熔断）和降级链（熔断后回退到备选工具）。可选：SLA 计时、参数校验、重试。

**Why:** 当前工具注册是扁平的，单个工具失败会直接暴露给 LLM。加熔断+降级后工具调用韧性大幅提升，LLM 不会卡在坏掉的工具上。

**参考:** elephant.ai 的 `toolregistry/registry.go`，5 层装饰器链 + `DegradationExecutor` + `circuitBreakerStore`。

**Effort:** S-M
**Priority:** P1

---

## P1: 多信号技能匹配 + 互斥组 + 冷却

**What:** 升级技能匹配引擎：加权多信号打分（intent 正则 0.6 + 最近工具 0.25 + 关键词 0.15）、互斥组（同组只激活最高分）、冷却机制（同一技能 N 秒内不重复触发）。

**Why:** 当前技能匹配较简单，容易误触发或重复触发。多信号匹配更精准，互斥组防冲突，冷却防刷屏。

**参考:** elephant.ai 的 `skills/matcher.go`，`MatchContext` + `resolveConflicts` + `cooldownTracker`。

**Effort:** M
**Priority:** P1

---

## P1: 网关快速分类路由

**What:** 在网关加一个快速分类步骤：用轻量 LLM 调用（或规则）把消息分成 direct（直接回复）、think（快确认+深度回复）、delegate（确认+后台 ReAct），简单消息不走完整 turn。

**Why:** 当前每条消息都跑完整 turn pipeline，简单问候或确认也要等全流程。分类路由后简单消息亚秒响应，体验质变。

**参考:** elephant.ai Lark 网关的「三模式大脑」，8s 快速 LLM 分类 → direct/think/delegate 三路由。

**Effort:** M
**Priority:** P1

---

## P1: Telegram → Sidecar 迁移

**What:** 把 Telegram 从 Rust 内置 channel 迁移到 sidecar 插件模式，和飞书统一架构。

**Why:** 统一 channel 架构，Rust 侧只保留 webhook 一个入口。砍掉 teloxide 重依赖，加新 channel 不再需要改 Rust。

**Steps:** 见 `docs/plans/telegram-sidecar-migration.md`（8 步，全部未开始）

**Effort:** M
**Priority:** P1
**Depends on:** 确认 openclaw telegram 插件功能对齐（webhook 模式、群组、media）

---

## P2: 技能反馈循环

**What:** 跟踪每次技能激活后是否有用（用户是否采纳、是否中断），维护 helpful ratio，下次匹配时乘以调整系数（0.7x–1.2x）。

**Why:** 技能越用越准，不好用的技能自动降权，免去手动调优 threshold。

**参考:** elephant.ai `matcher.go` 的 feedback-based score adjustment。

**Effort:** S
**Priority:** P2
**Depends on:** 多信号技能匹配先完成

---

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
