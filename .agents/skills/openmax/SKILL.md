---
name: openmax
description: 多智能体并行编排 — 将任务分解为子任务，分派到多个 AI agent 并行执行，自动合并结果。
triggers:
  intent_patterns:
    - "parallel|并行|多agent|分解任务|openmax|dispatch|多线程开发"
  context_signals:
    keywords: ["parallel", "并行", "openmax", "多agent", "分派", "dispatch"]
  confidence_threshold: 0.7
priority: 7
requires_tools: [bash]
max_tokens: 200
cooldown: 60
---

# openmax

多智能体并行任务编排。一条命令，多个 AI agent，零人工看护。

openmax 将复杂任务自动分解为可并行的子任务，分派到独立终端窗格中的 AI agent（Claude Code / Codex / OpenCode），监控执行进度，验证交付物，合并结果。

## 使用场景

| 场景 | 示例 |
|------|------|
| **并行功能开发** | 同时构建 API、前端 UI、测试 |
| **批量修 bug** | 一次性并行修复多个 issue |
| **全栈重构** | 自动按架构分解，schema → API + 前端并行 → 集成测试 |
| **多仓库协同** | 不同 cwd 下并行执行任务 |
| **晨间启动** | 从文件加载今日任务，一键并行分派 |

## 调用

### 基础并行

```bash
openmax run "Build REST API" "Add auth middleware" "Write integration tests"
```

### 从文件加载任务

```bash
openmax run -f tasks.txt
```

### 指定模型和 agent

```bash
openmax run "重构支付模块" --model claude-opus-4-20250805 --agents claude-code,codex
```

### 高质量模式（写 → 审 → 挑战 → 重写）

```bash
openmax run "实现用户鉴权" --quality
```

### 会话恢复

```bash
openmax run "继续上次任务" --session-id abc123 --resume
```

## 常用参数

| 参数 | 说明 |
|------|------|
| `--cwd PATH` | 工作目录 |
| `--model MODEL` | lead agent 模型（默认 sonnet） |
| `--agents LIST` | 逗号分隔的 agent 偏好，如 `claude-code,codex` |
| `--max-turns N` | 最大编排轮次 |
| `--keep-panes` | 完成后保留终端窗格 |
| `--quality / -q` | 高质量模式 |
| `--verbose / -v` | 显示子任务详细输出 |
| `--no-confirm` | 跳过计划确认 |
| `-f FILE` | 从文件加载任务（每行一条） |
| `--session-id ID` | 指定会话 ID |
| `--resume` | 恢复中断的会话 |

## 管理命令

```bash
openmax sessions          # 列出已完成会话
openmax inspect [ID]      # 查看会话详情
openmax usage             # 查看 token/费用统计
openmax log               # 查看事件日志
openmax status            # 检查环境就绪状态
openmax agents            # 列出可用 agent
openmax doctor            # 环境健康检查
```

## 项目感知

openmax 自动识别项目类型（Web App / CLI / API / Library / Refactor），应用对应的分解策略和反模式检查。可在 `.openmax/archetypes/*.yaml` 自定义。

## 安装位置

`/Users/bytedance/code/openmax`
