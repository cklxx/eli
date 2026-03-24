---
name: feishu
description: "3 tools: feishu_chat, feishu_sheet, feishu_oauth"
---

Call tools via: sidecar(tool="<name>", params={...})

## feishu_chat
以用户身份调用飞书群聊管理工具。Actions: search（搜索群列表，支持关键词匹配群名称、群成员）, get（获取指定群的详细信息，包括群名称、描述、头像、群主、权限配置等）。

## feishu_sheet
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

## feishu_oauth
飞书用户撤销授权工具。仅在用户明确说"撤销授权"、"取消授权"、"退出登录"、"清除授权"时调用 revoke。【严禁调用场景】用户说"重新授权"、"发起授权"、"重新发起"、"授权失败"、"授权过期"时，绝对不要调用此工具，授权流程由系统自动处理，无需人工干预。不需要传入 user_open_id，系统自动从消息上下文获取当前用户。

