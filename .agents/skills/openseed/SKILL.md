---
name: openseed
description: AI 驱动的学术研究工作流 — 论文发现、阅读、分析、对比、知识图谱、自动综述报告。
triggers:
  intent_patterns:
    - "paper|论文|arxiv|research paper|openseed|学术|文献|综述"
  tool_signals:
    - mcp__openseed__search_papers
    - mcp__openseed__get_paper
    - mcp__openseed__ask_research
  context_signals:
    keywords: ["论文", "paper", "arxiv", "openseed", "文献", "综述", "学术"]
  confidence_threshold: 0.6
priority: 7
requires_tools: [bash]
max_tokens: 200
cooldown: 60
---

# openseed

AI 驱动的学术研究工作流管理。一条命令，从发现论文到生成综述报告。

openseed 集成 ArXiv + Semantic Scholar 搜索、PDF 全文提取、Claude 智能分析（摘要/评审/对比/代码生成）、知识图谱、以及自动化多轮研究流水线。

## 使用场景

| 场景 | 示例 |
|------|------|
| **论文搜索** | 按关键词搜索 ArXiv + Semantic Scholar，按引用排序 |
| **自动综述** | 指定主题，自动发现论文 → 分析 → 生成结构化报告 |
| **论文精读** | 下载 PDF，AI 生成结构化摘要（中/英文） |
| **同行评审** | AI 模拟 peer review 风格的论文评审 |
| **论文对比** | 两篇或多篇论文的方法/结论/贡献并排对比与综合 |
| **代码生成** | 从论文描述生成实验代码框架 |
| **知识图谱** | 引用/被引关系、论文聚类可视化，发现研究脉络 |
| **研究记忆** | 回忆之前的研究对话和结论 |
| **研究缺口** | 分析库中论文覆盖的领域，发现研究空白 |
| **阅读规划** | 基于依赖关系推荐最优阅读顺序 |
| **实验追踪** | 关联论文与实验，记录复现结果 |
| **论文监控** | 监控 ArXiv 关键词，自动发现新论文 |
| **LaTeX 导出** | 将综合分析导出为 related-work 章节 + BibTeX |
| **Web 面板** | 启动本地 dashboard 浏览论文库 |

## 调用

### 搜索与发现

```bash
openseed paper search "diffusion models" --count 20   # 关键词搜索
openseed paper add https://arxiv.org/abs/1706.03762    # 添加论文
openseed paper fetch <paper_id>                        # 下载 PDF 并提取全文
openseed paper watch add "transformer" --interval daily # 监控 ArXiv 新论文
openseed paper watch list                              # 查看监控列表
```

### 阅读与管理

```bash
openseed paper list                        # 列出所有论文
openseed paper list --status unread        # 按阅读状态筛选
openseed paper show <paper_id>             # 查看论文详情
openseed paper next                        # 推荐下一篇最相关的未读论文
openseed paper done <paper_id> --note "..." # 标记已读并添加笔记
openseed paper status <paper_id> reading   # 更新阅读状态
openseed paper tag <paper_id> "tag_name"   # 打标签
openseed paper remove <paper_id>           # 删除论文
openseed paper export --format bibtex      # 导出 BibTeX
```

### AI 分析

```bash
openseed agent summarize <paper_id>        # 结构化摘要
openseed agent summarize <paper_id> --cn   # 中文摘要
openseed agent review <paper_id>           # 同行评审
openseed agent compare <id1> <id2>         # 两篇论文对比
openseed agent synthesize <id1> <id2> ...  # 多篇论文综合分析
openseed agent ask "What is RLHF?"         # 基于库的问答
openseed agent codegen <paper_id>          # 生成实验代码
openseed agent gaps                        # 分析研究缺口
openseed agent reading-order "topic"       # 推荐阅读顺序
openseed agent pipeline "topic"            # 搜索 → 筛选 → 自动分析
openseed agent export-latex                # 导出 LaTeX related-work
openseed agent chat                        # 交互式多轮研究对话
```

### 自动研究

```bash
openseed research run "ViT image classification" --count 15 --depth 2  # 全自动研究
openseed research list                     # 列出研究会话
openseed research show <session_id>        # 查看报告
openseed research export <session_id>      # 导出研究会话（可分享）
openseed research import session.json      # 导入研究会话
```

### 知识图谱

```bash
openseed paper graph <paper_id>            # 查看引用/被引关系
openseed paper cluster                     # 论文聚类分析
```

### 实验追踪

```bash
openseed experiment list                   # 列出实验
openseed experiment show <exp_id>          # 查看实验详情
openseed experiment link <paper_id>        # 关联实验到论文
```

### 研究洞察（Alerts）

```bash
openseed alerts list                       # 查看研究洞察（矛盾/印证）
openseed alerts read <alert_id>            # 标记已读
openseed alerts useful <alert_id>          # 标记有用（质量反馈）
openseed alerts dismiss <alert_id>         # 忽略
openseed alerts backfill                   # 从未分析论文中提取 claims
```

### Web 面板

```bash
openseed web --port 8765                   # 启动本地论文库 dashboard
```

### 其他

```bash
openseed ask "any question"                # 快速问答（不进入 chat 模式）
openseed doctor                            # 环境诊断
openseed setup                             # 配置 auth 和模型
```

## MCP 集成

openseed 同时提供 MCP server，已注册到 Claude Code。在对话中可直接通过 MCP 工具访问：

| MCP 工具 | 用途 | 成本 |
|----------|------|------|
| `library_stats` | 库概览统计 | 低 |
| `search_papers` | 关键词搜索论文 | 低 |
| `list_papers` | 按状态浏览论文 | 低 |
| `get_paper` | 论文详情（渐进式加载） | 低 |
| `get_graph` | 引用/被引关系 | 低 |
| `search_memories` | 回忆研究对话 | 低 |
| `ask_research` | 跨论文综合分析（调用 Claude API） | **高** |

**工具选择原则**：优先用便宜工具（stats → search → get_paper），自己推理；只在无法回答时才用 `ask_research`。

## 安装位置

`/Users/bytedance/code/openseed`
