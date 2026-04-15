---
name: openbench
description: Agent A/B 测试平台 — 对比不同 agent 配置（模型、提示词、工具、轮次），统计显著性评估最优方案。
triggers:
  intent_patterns:
    - "benchmark|A/B|ab test|对比测试|openbench|tournament|实验"
  context_signals:
    keywords: ["benchmark", "A/B", "openbench", "对比", "实验", "tournament"]
  confidence_threshold: 0.7
priority: 6
requires_tools: [bash]
max_tokens: 200
cooldown: 60
enabled: false
disabled_reason: "Depends on the external openbench CLI/repo and is not self-contained in this workspace."
---

# openbench

Agent A/B 测试平台。自动化 plan → run → evaluate → repeat，找到最优 agent 配置。

openbench 支持单变量 A/B 实验、多 agent 循环赛、以及自然语言驱动的自动研究循环（LLM 生成假设 → 执行 → 评估 → 迭代）。

## 使用场景

| 场景 | 示例 |
|------|------|
| **模型对比** | haiku vs sonnet 在编码任务上的正确率和成本 |
| **提示词优化** | 简洁 vs 结构化 system prompt 的效果差异 |
| **工具配置** | 有无 Bash 工具对数学题正确率的影响 |
| **轮次调优** | max_turns=5 vs 20 的完成率和 token 消耗 |
| **自动研究** | 用自然语言描述优化目标，LLM 自动生成并执行实验 |

## 调用

### 运行单个实验

```bash
openbench run experiments/my_test.py
```

### 多 agent 循环赛

```bash
openbench tournament experiments/my_tournament.py
```

### 自然语言驱动的自动研究

```bash
openbench research "找到在编码任务上性价比最高的模型+提示词组合" --max-iter 5 --max-cost 10
```

### 查看对比报告

```bash
openbench compare experiment_name
```

### 查看历史

```bash
openbench list                    # 列出所有实验
openbench show experiment_name    # 查看详细结果
openbench runs experiment_name    # 列出所有运行
openbench lineage experiment_name # 版本演进追踪
openbench tui                     # 交互式浏览器
```

## 实验定义（Python）

```python
from openbench.types import AgentConfig, DiffSpec, Experiment, TaskItem

experiment = Experiment(
    name="prompt_style",
    description="简洁 vs 详细 system prompt",
    diff=DiffSpec(field="system_prompt", description="prompt verbosity"),
    agent_a=AgentConfig(
        name="minimal",
        model="claude-haiku-4-5",
        system_prompt="Be concise.",
        allowed_tools=["Bash"],
        max_turns=10,
    ),
    agent_b=AgentConfig(
        name="detailed",
        model="claude-haiku-4-5",
        system_prompt="You are a senior engineer. Think step by step...",
        allowed_tools=["Bash"],
        max_turns=10,
    ),
    tasks=[
        TaskItem(prompt="Write fizzbuzz", expected="1,2,Fizz,...", check_fn='"Fizz" in output'),
    ],
    num_samples=3,  # pass@k
)
```

## 常用参数

| 参数 | 说明 |
|------|------|
| `--dry-run` | 预览不执行 |
| `--samples N` | 每 (agent, task) 的试验次数 |
| `--max-iter N` | 自动研究最大迭代数 |
| `--max-cost $X` | 预算上限（USD） |
| `--model MODEL` | Claude 模型 |
| `--target` | 优化目标：quality / cost / latency |
| `--yes` | 跳过确认 |

## 关键发现（200+ 实验）

- haiku + Bash ≈ sonnet（数学任务）→ 给小模型工具比升级模型更划算
- Sonnet 编码任务 token 消耗少 66%
- max_turns=20 是甜蜜点（= 2× 预期工具调用数）
- 结构化提示词可能反而伤害效果 → 偏好极简提示
- "Be careful" > 步骤列表 → 姿态 > 流程

## 安装位置
