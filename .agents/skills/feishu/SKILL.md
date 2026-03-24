---
name: feishu
description: "飞书基础能力：群聊管理、电子表格读写、用户授权撤销。适用于搜索/查看群信息、操作 Sheets 表格、撤销授权等场景。"
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 快速索引

| 用户意图 | 工具 | 关键参数 |
|---------|------|---------|
| 搜索群聊 | feishu_chat | action=search, keyword |
| 查看群信息 | feishu_chat | action=get, chat_id |
| 读取电子表格 | feishu_sheet | action=read, url/spreadsheet_token |
| 写入/追加表格数据 | feishu_sheet | action=write/append, url/spreadsheet_token, data |
| 创建电子表格 | feishu_sheet | action=create |
| 导出电子表格 | feishu_sheet | action=export, format |
| 撤销飞书授权 | feishu_oauth | action=revoke |

## 工具说明

### feishu_chat
以用户身份调用飞书群聊管理工具。Actions: search（搜索群列表，支持关键词匹配群名称、群成员）, get（获取指定群的详细信息，包括群名称、描述、头像、群主、权限配置等）。

### feishu_sheet
【以用户身份】飞书电子表格工具。支持创建、读写、查找、导出电子表格。

电子表格（Sheets）类似 Excel/Google Sheets，与多维表格（Bitable/Airtable）是不同产品。

所有 action（除 create 外）均支持传入 url 或 spreadsheet_token，工具会自动解析。支持知识库 wiki URL，自动解析为电子表格 token。

Actions:
- info：获取表格信息 + 全部工作表列表（一次调用替代 get_info + list_sheets）
- read：读取数据。不填 range 自动读取第一个工作表全部数据
- write：覆盖写入,高危,请谨慎使用该操作。不填 range 自动写入第一个工作表（从 A1 开始）
- append：在已有数据末尾追加行
- find：在工作表中查找单元格
- create：创建电子表格。支持带 headers + data 一步创建含数据的表格
- export：导出为 xlsx 或 csv（csv 必须指定 sheet_id）

### feishu_oauth
飞书用户撤销授权工具。仅在用户明确说"撤销授权"、"取消授权"、"退出登录"、"清除授权"时调用 revoke。不需要传入 user_open_id，系统自动从消息上下文获取当前用户。

## 不要这样做

- ❌ 用户说"重新授权"时调用 feishu_oauth revoke → ✅ 授权流程由系统自动处理，"重新授权"≠"撤销授权"
- ❌ 用 feishu_sheet 操作多维表格 → ✅ 电子表格（Sheets） ≠ 多维表格（Bitable），多维表格用 feishu-bitable skill
- ❌ 用 feishu_chat search 查群成员列表 → ✅ 查群成员用 feishu-chat skill 的 feishu_chat_members
