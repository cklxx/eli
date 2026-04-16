---
name: openmax
description: 多智能体并行编排 — 通过 REST API 远程调度 openMax，将任务分解为子任务分派到多个 AI agent 并行执行。
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
enabled: true
---

# openMax Remote Control

通过 REST API 远程调度 openMax 多智能体编排。

## 前置条件

openMax server 必须已启动：

```bash
# 在 openMax 机器上
openmax serve --api-key YOUR_KEY
```

## 配置

环境变量（在 Eli 的 `.env` 或 shell 中设置）：

```
OPENMAX_URL=http://localhost:7862    # openMax server 地址
OPENMAX_API_KEY=YOUR_KEY             # API key（如果设置了）
```

## 操作流程

### 1. 创建任务

```bash
curl -s -X POST "${OPENMAX_URL:-http://localhost:7862}/api/remote/tasks" \
  -H "Authorization: Bearer ${OPENMAX_API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{"task":"任务描述","cwd":"/project/path"}'
```

返回：`{"task_id":"abc123","status":"submitted","stream_url":"/api/remote/tasks/abc123/stream"}`

### 2. 订阅进度（SSE）

```bash
curl -s -N "${OPENMAX_URL:-http://localhost:7862}/api/remote/tasks/TASK_ID/stream" \
  -H "Authorization: Bearer ${OPENMAX_API_KEY}" \
  -H "Accept: text/event-stream"
```

事件类型：
- `progress` — 子任务进度（pct, msg）
- `done` — 子任务完成（summary）
- `input_needed` — 需要人工输入（request_id, question, choices）
- `completed` — 全部完成

### 3. 回复审批

当收到 `input_needed` 事件时：

```bash
curl -s -X POST "${OPENMAX_URL:-http://localhost:7862}/api/remote/tasks/TASK_ID/reply" \
  -H "Authorization: Bearer ${OPENMAX_API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{"request_id":"REQUEST_ID","text":"用户的回答"}'
```

### 4. 发送文本到 agent（"继续"）

```bash
curl -s -X POST "${OPENMAX_URL:-http://localhost:7862}/api/remote/tasks/TASK_ID/send" \
  -H "Authorization: Bearer ${OPENMAX_API_KEY}" \
  -H "Content-Type: application/json" \
  -d '{"pane_id":1,"text":"yes"}'
```

### 5. 查询状态

```bash
# 单个任务
curl -s "${OPENMAX_URL:-http://localhost:7862}/api/remote/tasks/TASK_ID" \
  -H "Authorization: Bearer ${OPENMAX_API_KEY}"

# 所有任务
curl -s "${OPENMAX_URL:-http://localhost:7862}/api/remote/tasks" \
  -H "Authorization: Bearer ${OPENMAX_API_KEY}"
```

### 6. 取消任务

```bash
curl -s -X DELETE "${OPENMAX_URL:-http://localhost:7862}/api/remote/tasks/TASK_ID" \
  -H "Authorization: Bearer ${OPENMAX_API_KEY}"
```

## 完整工作流示例

当用户要求并行执行任务时：

1. 用 `bash` 工具调用 `curl` 创建 openMax 任务
2. 轮询或 SSE 监控进度
3. 将进度事件翻译为人类可读消息返回给用户
4. 如果收到 `input_needed`，向用户提问，再用 `/reply` 回复
5. 任务完成后汇总结果

## CLI 模式（本地直接调用）

如果 openMax CLI 在本机可用，也可直接调用：

```bash
openmax run "Build REST API" "Add auth middleware" "Write integration tests"
openmax run "重构支付模块" --model claude-opus-4-20250805 --agents claude-code,codex
openmax sessions          # 列出已完成会话
openmax status            # 检查环境就绪状态
```
