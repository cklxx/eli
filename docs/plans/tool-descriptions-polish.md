# 2026-03-26 · 工具参数描述精简

## 原则

参数描述只保留**参数名 + 参数类型 + 工具描述**推导不出的信息。具体来说只保留：

1. **非显而易见的默认值** — `"Default 20."`, `"Default false."`
2. **单位或格式约束** — `"0-based line number."`, `"ISO date."`
3. **跨工具引用** — `"Use next_offset from previous response."`
4. **关键行为修饰** — background 时 timeout 被忽略

名字已经说清楚的，删。工具描述已经说过的，不重复。

## 附带：bash `description` 参数移除

`7ab6385` 声称移除了 description 参数，但 bash 仍有且 required。每次调用模型被迫生成这个字段，浪费输出 token。移除 schema 定义 + 从 required 中去掉。handler 端 `maybe_send_user_facing_notice` 继续读 `args.get("description")` 不受影响。

## 逐工具改动

### bash
| 参数 | 现状 | 改为 |
|------|------|------|
| `cmd` | "Shell command to execute." | 删除 description |
| `description` | required 参数 | **从 schema 和 required 中移除** |
| `cwd` | "Absolute working directory for the command." | "Absolute path. Defaults to workspace." |
| `timeout_seconds` | "Kill the process after N seconds (default 30). Ignored when background=true." | 保留不动 — 含默认值 + 行为交互 |
| `background` | "Run asynchronously. Returns a shell_id immediately — poll with bash.output, stop with bash.kill." | "Returns shell_id; poll with bash.output." |

### bash.output
| 参数 | 现状 | 改为 |
|------|------|------|
| `shell_id` | "The background shell ID returned by bash." | 删除 description |
| `offset` | "Character offset to resume reading from (use next_offset from previous call)." | "Resume from next_offset of previous call." |
| `limit` | "Max characters to return per call." | 删除 description |

### bash.kill
| 参数 | 现状 | 改为 |
|------|------|------|
| `shell_id` | "The background shell ID to terminate." | 删除 description |

### fs.read
| 参数 | 现状 | 改为 |
|------|------|------|
| `path` | "File path (absolute or relative to workspace)." | "Absolute or workspace-relative." |
| `offset` | "Line number to start reading from (0-based)." | "0-based line number." |
| `limit` | "Max number of lines to return. Set this for large files to avoid wasted tokens." | "Max lines." |

### fs.write
| 参数 | 现状 | 改为 |
|------|------|------|
| `path` | "File path (absolute or relative to workspace)." | "Absolute or workspace-relative." |
| `content` | "Full file content to write." | 删除 description |

### fs.edit
| 参数 | 现状 | 改为 |
|------|------|------|
| `path` | "File path (absolute or relative to workspace)." | "Absolute or workspace-relative." |
| `old` | "Exact text to find and replace (first occurrence only)." | 删除 description — 工具描述已说 |
| `new` | "Replacement text." | 删除 description |
| `start` | "Line number to start searching from (0-based, optional)." | "0-based line to start search." |

### skill
| 参数 | 现状 | 改为 |
|------|------|------|
| `name` | "Skill name (e.g. 'deploy', 'feishu-calendar')." | "e.g. 'deploy', 'feishu-calendar'." |

### tape.search
| 参数 | 现状 | 改为 |
|------|------|------|
| `query` | "Keyword to search for in tape entries." | 删除 description |
| `limit` | "Max results (default 20)." | "Default 20." |
| `start` | "Optional start date (ISO)." | "ISO date." |
| `end` | "Optional end date (ISO)." | "ISO date." |
| `kinds` | "Entry kinds to filter (default: message, tool_result)." | "Default: message, tool_result." |

### tape.reset
| 参数 | 现状 | 改为 |
|------|------|------|
| `archive` | "Save a tape snapshot before wiping (default false)." | "Default false." |

### tape.handoff
| 参数 | 现状 | 改为 |
|------|------|------|
| `name` | "Anchor name (default: handoff)." | "Default: handoff." |
| `summary` | "What was accomplished — used for context when resuming later." | "Context for resuming later." |

### tape.anchors
| 参数 | 现状 | 改为 |
|------|------|------|
| `limit` | "Max anchors to return (default 20)." | "Default 20." |

### decision.set
| 参数 | 现状 | 改为 |
|------|------|------|
| `text` | "The decision to record." | 删除 description |

### decision.remove
| 参数 | 现状 | 改为 |
|------|------|------|
| `index` | "The decision number to remove (1-based, from decision.list)." | "1-based, from decision.list." |

### web.fetch
| 参数 | 现状 | 改为 |
|------|------|------|
| `url` | "The URL to fetch." | 删除 description |
| `headers` | "Custom HTTP headers as key-value pairs." | 删除 description |
| `timeout` | "Request timeout in seconds (default 10)." | "Seconds. Default 10." |

### subagent
| 参数 | 现状 | 改为 |
|------|------|------|
| `prompt` | (65字 长描述) | "Self-contained task. Include all context — the sub-agent has no shared history." |
| `cwd` | "Absolute working directory for the sub-agent. Defaults to the current workspace." | "Absolute path. Defaults to workspace." |
| `cli` | "Force a specific CLI binary (e.g. 'claude', 'codex', 'kimi'). Auto-detected if omitted." | "e.g. 'claude', 'codex'. Auto-detected if omitted." |

### message.send
| 参数 | 现状 | 改为 |
|------|------|------|
| `text` | "Message text to send to the user." | 删除 description |
| `media_path` | "Optional local media path to send along with the message on channels that support media." | "Local file path." |
| `media_paths` | "Optional list of local media paths to send along with the message on channels that support media." | "Multiple local file paths." |
| `image_path` | "Deprecated alias for media_path; kept for backward compatibility." | "Deprecated; use media_path." |

### sidecar
| 参数 | 现状 | 改为 |
|------|------|------|
| `tool` | "The sidecar tool name to execute (e.g. feishu_calendar_event)." | "e.g. feishu_calendar_event." |
| `params` | "Parameters for the tool." | 删除 description |
