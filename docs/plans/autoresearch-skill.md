# Autoresearch Skill — OpenSeed + OpenMax + OpenBench

## Problem

Karpathy 的 autoresearch 证明了一个强力模式：agent 自主循环跑实验，keep/discard，overnight 跑 100 次。
当前该模式仅限 ML 训练（val_bpb），且没有文献检索环节——agent 靠自身知识生成假设。

**目标**：将 autoresearch 抽象成 eli skill，接入 openseed（文献）、openmax（编排）、openbench（评估），
形成 literature-informed 自动化实验 pipeline。

---

## 原版机制精要（不可失真）

### Karpathy 原版核心设计

| 元素 | 实现 | 为什么这样设计 |
|------|------|----------------|
| **单文件约束** | 只能改 `train.py`，`prepare.py` 只读 | 防止 agent 篡改评估函数（alignment in miniature） |
| **固定预算** | 5 min wall clock，超 10 min 视为失败 | 保证实验可比性，~12 次/小时 |
| **val_bpb 指标** | bits per byte，vocab-size 无关 | 跨架构可比——换 vocab、换模型都能比 |
| **git-as-memory** | commit before run → keep=advance / discard=reset | 分支只保留成功链，TSV 记录全部（含失败） |
| **results.tsv** | 不入 git，本地 append-only log | agent 每轮读 TSV 避免重复失败实验 |
| **NEVER STOP** | 不允许暂停问人 | 用户可能在睡觉，agent 必须自主到底 |
| **simplicity criterion** | 同等 val_bpb 下更简单的代码优先 | 防止 agent 堆复杂度换微小收益 |
| **program.md** | 纯 markdown 指令，无 Python 编排 | agent 本身就是循环引擎 |

### Karpathy 原版 program.md 关键语义

```
实验循环（逐字保真）：
1. 看 git state
2. 改 train.py
3. git commit
4. uv run train.py > run.log 2>&1  （重定向一切，不让输出灌 context）
5. grep "^val_bpb:\|^peak_vram_mb:" run.log
6. grep 空 → crashed → tail -n 50 run.log → 尝试修 or 放弃
7. 记 results.tsv（不 commit，untracked）
8. val_bpb 降低 → keep，advance branch
9. val_bpb 不变或升高 → git reset 回上一个 commit
```

**NEVER STOP 原文**：
> Once the experiment loop has begun, do NOT pause to ask the human if you should continue.
> Do NOT ask "should I keep going?" The human might be asleep. You are autonomous.
> If you run out of ideas, think harder — read papers, re-read files, combine near-misses,
> try radical changes. The loop runs until the human interrupts you, period.

**simplicity criterion 原文**：
> All else being equal, simpler is better. A 0.001 val_bpb improvement that adds 20 lines
> of hacky code? Probably not worth it. A 0.001 val_bpb improvement from deleting code?
> Definitely keep. An improvement of ~0 but much simpler code? Keep.

**VRAM 原文**：
> VRAM is a soft constraint. Some increase is acceptable for meaningful val_bpb gains,
> but it should not blow up dramatically.

### uditgoenka 通用化关键抽象

| 抽象 | 原版 | 通用化 |
|------|------|--------|
| **指标** | `val_bpb`（硬编码） | 任意 shell 命令输出数字 |
| **scope** | `train.py` 一个文件 | 任意 file glob |
| **rollback** | `git reset` | `git revert`（保留失败记忆）→ fallback `reset` |
| **guard** | 无 | 可选 guard 命令（类似 CI check） |
| **noise handling** | 无 | multi-run median / min-delta / confirmation run |
| **bounded mode** | 无（永远循环） | `Iterations: N` 支持 CI 集成 |
| **子命令** | 无 | plan/debug/fix/security/ship/scenario/predict/learn |
| **TSV 格式** | 5 列（commit/val_bpb/memory_gb/status/description） | 7 列（加 iteration/delta/guard） |
| **rollback 策略** | `git reset`（丢失历史） | `git revert`（保留失败 commit 为记忆）→ fallback `reset` |
| **原子性** | 隐含（只改一个文件） | 显式检查：>5 文件变更需审视，描述含 "and" 则拆分 |

---

## 工具能力审计

### eli skill 系统

| 项目 | 详情 |
|------|------|
| **存放位置** | `.agents/skills/<name>/SKILL.md`（项目级）、`~/.eli/skills/<name>/SKILL.md`（全局） |
| **命名规则** | `^[a-z0-9]+(?:-[a-z0-9]+)*$`，1-64 字符 |
| **格式** | YAML frontmatter（name/description/triggers/priority/exclusive_group/cooldown）+ Markdown body |
| **触发机制** | 三路加权打分：intent_patterns（正则，权重 0.6）+ tool_signals（0.25）+ context_keywords（0.15） |
| **Token 预算** | Skills section 默认 8KB，系统 prompt 总上限 32KB |
| **变量替换** | `$SKILL_DIR` → skill 目录路径，`$PYTHON` → python 命令 |
| **优先级** | 项目 > 全局（同名项目覆盖全局），支持 `priority` 数值 + `exclusive_group` |
| **子 skill** | 不是原生概念——用 references/ 子目录 + 条件读取实现 |
| **现有 skill 数量** | `.agents/skills/` 下 35+ 个 |

**对 autoresearch 的影响**：
- Skill 名必须是 `autoresearch`，目录 `.agents/skills/autoresearch/SKILL.md`
- 子命令（plan/debug/fix）用 references/ 目录 + SKILL.md 内路由实现，不是独立 skill
- 8KB token 限制意味着 SKILL.md 本体要精简，详细协议放 references/
- 可用 `$SKILL_DIR` 引用 references/ 文件路径

### OpenSeed — 文献检索层

**实际能力**：
| 工具 | 能力 | 限制 |
|------|------|------|
| `library_stats` | 快速概览：论文数量、状态分布、top tags | 只显示 top 10 tags |
| `list_papers` | 分页浏览论文列表 | 无排序选项，无日期字段 |
| `search_papers` | 关键词匹配（标题/摘要/tags） | **非语义搜索**——"machine learning" 只匹配 2/17 篇，漏掉大量相关论文 |
| `get_paper` | 获取论文详情（摘要 + AI 摘要） | 无 PDF 全文，只有摘要 + AI 生成的结构化摘要 |
| `get_graph` | 引用关系图 | **当前返回空数组**——功能存在但无数据 |
| `search_memories` | 搜索对话记忆 | **当前为空**——需要 OpenSeed UI 产生记忆 |
| `ask_research` | 跨论文综合分析 | 调用 Claude API，昂贵且慢，黑盒 |

**关键缺失**：

| 缺失能力 | 对 autoresearch 的影响 | 建议 |
|----------|----------------------|------|
| **无写入 API**（不能 add_paper/update/create_memory） | 无法在循环中记录"论文 X 的假设 Y 被实验验证/否定" | 用本地 hypothesis_queue.md 替代，不依赖 openseed 写入 |
| **非语义搜索** | 关键词不精确时漏论文 | 多轮搜索（同义词展开），或 ask_research 做初始检索 |
| **引用图空** | 无法从一篇论文扩展到相关工作 | 跳过 get_graph，改用 search_papers 多关键词覆盖 |
| **无外部搜索**（arxiv/Scholar） | 本地库只有 17 篇，覆盖有限 | 文献注入标记为 best-effort，agent 自身知识为主力 |
| **无 add_paper** | 发现相关论文无法入库 | 提 feature request 或 fork 加 add_paper tool |

### OpenMax — 编排层

**实际能力**：
| 工具 | 能力 | 限制 |
|------|------|------|
| `report_progress` | 向 lead agent 推送进度（task/pct/msg） | **单向推送**，无返回通道；lead 不在线时静默丢弃 |
| `execute_with_codex` | 同步调用 Codex CLI 执行单个编码任务 | **串行阻塞**（一次一个）；当前版本 flag 不兼容已报错 |
| `report_done` | 推送完成状态 + 成本统计 | lead 不在线时硬失败（不是静默） |

**通信模型**：worker → lead 单向推送，Unix domain socket（`/tmp/openmax-{session_id}.sock`）。

**关键缺失**：

| 缺失能力 | 对 autoresearch 的影响 | 建议 |
|----------|----------------------|------|
| **无并行 agent 启动** | 不能同时跑多个假设 | v1 先做单 agent；并行需 eli 自身 Agent tool（worktree 隔离） |
| **无 agent 间通信** | 多 agent 不能共享结果 | 用共享 results.tsv（文件锁）或共享 git 分支 |
| **无实验队列** | 不能预排实验计划 | 用本地 hypothesis_queue.md 替代 |
| **无双向通信** | lead 不能叫停或重定向 worker | NEVER STOP 语义下其实不需要——用户 Ctrl+C 即可 |
| **无持久循环** | 无原生 loop primitive | 循环由 agent context window 驱动，和原版一致 |
| **execute_with_codex 当前不可用** | Codex CLI flag 不兼容 | 不依赖此工具，用 eli 自身 Bash tool 跑实验 |

**结论**：OpenMax 在 v1 中仅用作 **progress telemetry**（report_progress/report_done），不参与控制流。

### OpenBench — 评估层

**实际能力**：
| 项目 | 详情 |
|------|------|
| **benchmark 数量** | 95+（MMLU, HumanEval, GSM8K, GPQA, ARC, AIME, etc.） |
| **接口** | CLI（`bench eval`）+ Python API（`inspect_ai.eval()`） |
| **输出** | 二进制 .eval 文件（默认）或 JSON，写到 `./logs/`——**不输出到 stdout** |
| **速度** | `--limit 20` MMLU via Groq: ~5-15 秒；全量 MMLU: ~15-45 分钟 |
| **自定义 benchmark** | 支持：`bench eval /path/to/eval.py` 或 plugin 机制 |
| **可复现性** | `--seed` flag 固定采样 |
| **MCP 集成** | **无**——纯 CLI/Python |

**关键缺失**：

| 缺失能力 | 对 autoresearch 的影响 | 建议 |
|----------|----------------------|------|
| **无 stdout 单分数输出** | 不能直接当 metric_cmd 用 | 写 20 行 Python wrapper（`openbench_metric.py`） |
| **无 run 对比** | 不能自动比较两次结果 | 从 Python API 读 `EvalLog` 自己 diff |
| **无 composite score** | 多 benchmark 无聚合 | 写加权公式（如 0.4*MMLU + 0.3*HumanEval + 0.3*GSM8K） |
| **全量 benchmark 太慢** | 不适合 5min 循环 | 用 `--limit 20-50` 做快速 gate，全量只在最终验证 |
| **alpha 状态** | API 可能变 | 锁定版本（v0.5.3），wrapper 隔离变更 |

---

## 修订后方案设计

### 架构总览

```
/autoresearch 触发 → eli skill (.agents/skills/autoresearch/SKILL.md)
    │
    ├── Phase 0: Setup（交互式收集 scope/metric/guard/literature/orchestration）
    │
    ├── Phase 1: Literature Inform [可选 - OpenSeed]
    │   ├── search_papers(多轮同义词) → get_paper(section=methods)
    │   ├── 生成 hypothesis_queue.md（本地文件，不入 git）
    │   └── 跳过 get_graph（当前返回空）
    │
    ├── Phase 2-7: Experiment Loop
    │   ├── Review: results.tsv + git log + hypothesis_queue.md
    │   ├── Modify: 编辑 scope 内文件，单一变更，git commit
    │   ├── Run: metric_cmd > run.log（或 openbench wrapper）
    │   ├── Evaluate: 提取指标 + guard + 对比 best
    │   ├── Decision: keep(advance) / discard(revert) / crash(fix or skip)
    │   ├── Log: append results.tsv + report_progress
    │   └── [stuck 检测] 连续 5 次 discard → re-trigger Phase 1
    │
    └── Phase 8: Report
        ├── summary: baseline → best, iterations, keeps/discards/crashes
        ├── report_done（if openmax enabled）
        └── 文献假设验证报告（if literature enabled）
```

### 文件结构

```
.agents/skills/autoresearch/
  SKILL.md                                # 入口：setup gate + 路由 + critical rules
  references/
    autonomous-loop-protocol.md           # Phase 0-8 完整协议
    core-principles.md                    # 7 条原则（保真原版 + 通用化扩展）
    results-logging.md                    # TSV 格式规范
    literature-inform.md                  # OpenSeed 文献注入协议 + 降级策略
    orchestration.md                      # OpenMax 编排协议（仅 telemetry）
    benchmark-integration.md              # OpenBench 集成 + wrapper 脚本
  templates/
    openbench_metric.py                   # OpenBench → single float stdout wrapper
```

### Phase 详细设计

#### Phase 0: Interactive Setup

一次性 batched 收集（AskUserQuestion 一次问完）：
```
必填：
1. scope: 可修改的文件 glob（如 `src/**/*.rs`, `train.py`）
2. metric_cmd: shell 命令，输出单个数字（如 `cargo test 2>&1 | grep ... | awk ...`）
3. metric_direction: higher_is_better | lower_is_better

可选（有默认值）：
4. guard_cmd: 守护命令（默认无）
5. time_budget: 单次实验预算（默认 300s）
6. mode: bounded(N) | unbounded（默认 unbounded）
7. noise_strategy: none | min-delta(0.001) | multi-run(3) | confirmation（默认 min-delta）
8. literature: on | off（默认 on if openseed available）
9. orchestration: on | off（默认 on if OPENMAX_SESSION_ID set）
```

**dry run 验证**（setup 结束前必做）：
- 跑一次 metric_cmd，确认能输出数字
- 跑一次 guard_cmd（如有），确认返回 0
- 确认 git repo 干净，创建 `autoresearch/<tag>` 分支

#### Phase 1: Literature Inform

**触发条件**：循环开始前 + 连续 5 次 discard（stuck 检测）

```
1. 从 scope 文件提取技术关键词
2. openseed.search_papers(keyword1) + search_papers(synonym1) + ...
   （多轮搜索弥补非语义搜索的缺陷）
3. 对每篇命中论文：openseed.get_paper(id, section="methods")
4. 跳过 get_graph（返回空，不浪费调用）
5. 生成 hypothesis_queue.md：
   ---
   # Hypothesis Queue
   | # | hypothesis | source_paper | expected_effect | complexity | status |
   |---|-----------|-------------|-----------------|-----------|--------|
   | 1 | 用 RoPE 替换绝对位置编码 | Attention Is All You Need → RoFormer | 降低 val_bpb ~0.5% | medium | untried |
   | 2 | ... | ... | ... | ... | ... |
6. 循环中 agent 自由选择：自己的想法 > queue 中的假设
7. 尝试过的假设更新 status: tried-keep / tried-discard
```

**降级策略**：
- OpenSeed 不可用 → 跳过，仅靠 agent 自身知识（和原版一致）
- 搜索结果为空 → 用 ask_research 做一次综合查询（昂贵但一次性）
- ask_research 也无结果 → 完全降级，纯靠 agent

#### Phase 2-7: Experiment Loop

**Phase 2: Review**
```bash
# 看过去的实验
tail -20 autoresearch-results.tsv
git log --oneline -20
# 看文献假设（如果有）
cat hypothesis_queue.md | grep "untried"
```

**Phase 3: Modify**
- 编辑 scope 内文件
- 单一变更原则：描述不需要 "and"
- `git commit -m "experiment(<scope>): <description>"`
- commit BEFORE run（有 hash 可记录）

**Phase 4: Run**
```bash
timeout $((time_budget * 2)) bash -c "$metric_cmd" > run.log 2>&1
```
- 超时 = 2x time_budget → kill + 视为失败
- 重定向一切，不让输出灌 context（保真原版设计）

**Phase 5: Evaluate**
```bash
# 提取指标
metric=$(tail -1 run.log | grep -oE '[0-9]+\.[0-9]+')
# guard（如有）
if [ -n "$guard_cmd" ]; then eval "$guard_cmd"; guard_status=$?; fi
```
- 对比 previous best
- noise handling: 按 setup 选择的策略处理

**Phase 6: Decision**
```
improved + guard pass → KEEP
  - branch advance, 记录 commit hash
improved + guard fail → REVERT
  - git revert HEAD --no-edit
  - 尝试 rework（max 2 次），不同实现同目标
  - 2 次失败 → discard
not improved → REVERT
  - git revert HEAD --no-edit
  - fallback: git revert --abort && git reset --hard HEAD~1
crash →
  - syntax/typo: 修，不算迭代
  - runtime: max 3 次修复尝试
  - OOM: revert，试小版本
  - 根本不行: skip，记 crash
```

**Phase 7: Log**
```bash
echo -e "$iteration\t$commit\t$metric\t$delta\t$guard\t$status\t$description" >> autoresearch-results.tsv
```
- [OpenMax] `report_progress({ task: "autoresearch", pct: bounded ? iter/max*100 : 0, msg: "$status: $description ($metric)" })`
- [stuck 检测] 读 TSV 最后 5 行，全是 discard → 触发 Phase 1 re-inform

**NEVER STOP 语义**：
- unbounded mode: 永远不暂停，永远不问用户
- bounded mode: 达到 N 次后打印 summary 并停止
- 两种模式都：agent 没想法时 think harder，不是 ask

#### Phase 8: Report

```
=== AUTORESEARCH REPORT ===
Baseline:  0.9979 val_bpb (commit a1b2c3d)
Best:      0.9770 val_bpb (commit f6g7h8i)  [−2.1%]
Iterations: 83 (15 kept, 61 discarded, 7 crashed)

Top improvements:
  1. +0.0050  increase dim to 768 (commit b2c3d4e)
  2. +0.0032  switch to Muon optimizer (commit c3d4e5f)
  ...

Literature hypotheses: 3 tried, 1 kept, 2 discarded
  ✓ RoPE positioning (from RoFormer paper) → −0.003 val_bpb
  ✗ GLU activation (from LLaMA paper) → +0.001 val_bpb
  ✗ ...
```

---

## OpenBench metric_cmd Wrapper

```python
#!/usr/bin/env python3
"""openbench_metric.py — single-score eval for autoresearch pipeline."""
import sys, os
os.environ["INSPECT_LOG_DIR"] = "/tmp/openbench-scratch"

from inspect_ai import eval as inspect_eval

benchmark = sys.argv[1]  # e.g. "mmlu"
model = sys.argv[2]      # e.g. "groq/llama-3.3-70b-versatile"
limit = int(sys.argv[3]) if len(sys.argv) > 3 else 20

logs = inspect_eval(
    tasks=[benchmark],
    model=model,
    limit=limit,
    max_connections=10,
    display="none",
    log_format="eval",
)
score = logs[0].results.scores[0].metrics["accuracy"].value
print(f"{score:.6f}")
```

用法：`metric_cmd = "python3 $SKILL_DIR/templates/openbench_metric.py mmlu groq/llama-3.3-70b-versatile 20"`

---

## 三工具迭代建议

### OpenSeed 需要迭代

| 优先级 | 功能 | 理由 |
|--------|------|------|
| **P0** | `add_paper(arxiv_id)` | 没有写入就无法在研究中发现新论文入库 |
| **P0** | `create_memory(content, tags)` | 实验结果无法回写为记忆，下次会话丢失 |
| **P1** | 语义搜索（embedding-based） | 关键词搜索漏论文率太高，"machine learning" 只命中 2/17 |
| **P1** | `update_paper(id, status, tags, notes)` | 无法标记已读、加标签、加实验关联笔记 |
| **P2** | 引用图数据填充 | get_graph 返回空，结构在但没数据 |
| **P2** | `search_arxiv(query)` | 本地库 17 篇太少，需要在线扩展 |

### OpenMax 需要迭代

| 优先级 | 功能 | 理由 |
|--------|------|------|
| **P0** | 修复 `execute_with_codex` flag 兼容 | 当前直接报错，无法使用 |
| **P1** | `spawn_agent(task, config)` → 异步启动 | 并行实验的前提，目前只有同步阻塞 |
| **P1** | 结果存储 / blackboard | worker 结果 fire-and-forget，无法被其他 agent 查询 |
| **P2** | 双向通信（lead → worker） | 支持动态调整实验方向，但 NEVER STOP 下优先级低 |
| **P2** | 持久实验队列 | 当前 Task 系统是 session-local，重启丢失 |

### OpenBench 需要迭代（或 wrap）

| 优先级 | 功能 | 理由 |
|--------|------|------|
| **P0** | stdout 单分数输出模式 | 当前必须写 wrapper 解析 log 文件 |
| **P1** | `bench compare run-A run-B` | 无内置对比，autoresearch 的 keep/discard 需要 |
| **P1** | composite score 支持 | 多 benchmark 加权聚合 |
| **P2** | 统计显著性检测 | 区分真实改善 vs 噪声 |
| **P2** | MCP server wrapper | 让 agent 直接调 benchmark 而不是 shell out |

---

## 实现计划

### Step 1: Skill 骨架 [~2h]

创建 `.agents/skills/autoresearch/SKILL.md` + `references/` 目录。
SKILL.md frontmatter:
```yaml
name: autoresearch
description: Autonomous experiment loop — modify, measure, keep/discard, repeat
triggers:
  intent_patterns:
    - "autoresearch"
    - "experiment loop"
    - "autonomous.*experiment"
  context_signals:
    keywords: ["autoresearch", "experiment", "optimize", "benchmark"]
priority: 100
```
Body: setup gate + 子命令路由 + 8 条 critical rules（精简，<4KB）。
**验证**：`eli chat` 中输入含 "autoresearch" 的 prompt 能触发 skill 展开。

### Step 2: 核心循环协议 [~3h]

写 `references/autonomous-loop-protocol.md`——Phase 0-8 完整指令。
严格保真原版语义 + 通用化扩展。
**验证**：手动在一个简单项目跑 bounded(3) 循环，确认 keep/discard/TSV/git 正确。

### Step 3: 原则 + TSV 格式 [~1h]

写 `references/core-principles.md` + `references/results-logging.md`。
**验证**：检查 TSV 格式与 git 操作的一致性。

### Step 4: OpenSeed 文献注入 [~2h]

写 `references/literature-inform.md`。
实现降级策略（openseed 不可用 → 跳过）。
**验证**：用 openseed 现有论文库生成 hypothesis queue。

### Step 5: OpenMax + OpenBench 集成 [~2h]

写 `references/orchestration.md` + `references/benchmark-integration.md`。
写 `templates/openbench_metric.py`。
**验证**：report_progress 能发出；openbench wrapper 能输出单分数。

### Step 6: 端到端验证 [~2h]

在真实场景跑 unbounded 循环 15-20 分钟：
- [ ] 文献假设被正确注入（或优雅降级）
- [ ] git history 干净（只有 keep 的 commit + revert 的 discard）
- [ ] results.tsv 完整记录所有迭代
- [ ] openmax 收到进度（或优雅降级）
- [ ] 连续失败触发 re-inform
- [ ] NEVER STOP 语义正确（agent 不暂停问人）

---

## Open Questions

1. ~~eli skill 格式~~ → **已解决**：YAML frontmatter + MD body，放 `.agents/skills/autoresearch/`
2. ~~OpenBench API~~ → **已解决**：CLI `bench eval` + Python `inspect_ai.eval()`，需 wrapper
3. **是否支持非 git 项目？** → 建议 **不支持**，git-as-memory 是核心设计，非 git 降低太多
4. **v1 scope** → 建议 **核心循环 + 文献注入**，子命令（plan/debug/fix）留 v2
5. **并行实验** → v1 单 agent 单分支，v2 考虑 eli Agent tool + worktree isolation
