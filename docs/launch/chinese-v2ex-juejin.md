# 中文社区发布帖

<!--
发布到：V2EX (创意/分享节点)、掘金、Rust 中文社区、知乎
标题见各平台版本
-->

---

## V2EX 版本

**标题：用 Rust 写了一个 AI Agent 框架，可以部署到 Telegram/飞书群里当队友**

各位好，分享一个个人项目：**[Eli](https://github.com/cklxx/eli)**，一个用 Rust 写的 hook-first AI agent 框架。

### 起因

我需要一个 AI agent 常驻在团队的群聊里——不是那种一问一答的 chatbot，而是能跑工具、记上下文、主动跟进的「AI 队友」。市面上的 agent 框架（LangChain、CrewAI、AutoGen）全是 Python，部署麻烦，并发拉胯，每台机器都要装 Python 环境。

### 为什么用 Rust

- **单一二进制部署**：`cargo install` 完事，不需要 virtualenv，不需要 pip，扔到服务器就跑
- **真正的异步并发**：群聊里多人同时说话、多个工具并行跑、LLM 流式输出——tokio 天然支持，不需要和 GIL 搏斗
- **类型安全**：工具 schema 编译期检查，不会在运行时才发现参数对不上
- **性能**：框架层开销 ~2ms/轮，LLM 调用才是瓶颈

### 核心设计

每条消息走 7 阶段 hook 流水线：

```
resolve_session → load_state → build_prompt → run_model → save_state → render_outbound → dispatch_outbound
```

12 个 hook 点，后注册的覆盖先注册的。内置功能就是默认插件，你随时可以替换。

- 21 个内置工具（shell、文件系统、web fetch、子 agent、tape 操作等）
- Tape 系统：只追加的对话历史，支持锚定、搜索、分叉
- LLM 可切换：OpenAI / Claude / Copilot / DeepSeek / Ollama，一个环境变量搞定
- 多渠道：CLI、Telegram、飞书/钉钉/Slack/Discord（通过 OpenClaw sidecar）

### 现状

v0.3.0，小团队日常在用。不是 LangChain 的替代品——更小、更快、部署更简单。如果你需要 500 个集成，用 LangChain；如果你需要一个能快速部署到群聊里的 agent，试试 eli。

GitHub：https://github.com/cklxx/eli
主页：https://eliagent.github.io

欢迎 star、issue、PR。

---

## 掘金版本

**标题：我用 Rust 造了个 AI Agent 框架，替代 Python 全家桶**

> 一个人、一门语言、一个二进制文件，部署一个能在群聊里跑工具的 AI 队友。

（正文同 V2EX 版本，增加以下段落：）

### 和 Python 框架的对比

|   | Eli | LangChain | CrewAI | AutoGen |
|---|-----|-----------|--------|---------|
| 语言 | Rust | Python | Python | Python |
| 部署 | 单一二进制 | pip + 依赖地狱 | pip + 依赖 | pip + 依赖 |
| 架构 | Hook 流水线（12 个切入点） | Chain/Graph | 角色扮演 | 多 agent 对话 |
| 渠道支持 | CLI、Telegram、飞书、Slack、Discord | 无（纯库） | 无（纯库） | 无（纯库） |
| 记忆 | Tape（只追加、可分叉） | 多种 Memory 类 | 共享内存 | 聊天历史 |

Eli 更年轻、更小。优势是 Rust 的性能、类型安全和单二进制部署。如果你需要成熟生态和几百个集成，用 LangChain。如果你要的是一个快速、自包含、能处理真实并发的 agent——试试 eli。

---

## 知乎版本

**标题：为什么我用 Rust 而不是 Python 来写 AI Agent 框架？**

（以问答形式展开，核心论点同上，增加更多技术细节讨论：async trait 的痛点、teloxide 的坑、流式 SSE 处理的不同 provider 差异等）
