---
name: feishu-bitable
description: |
  飞书多维表格（Bitable）的创建、查询、编辑和管理。支持 27 种字段类型、高级筛选、批量操作。
---

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

## 执行前必读

- **写记录前**：先调用 `feishu_bitable_app_table_field.list` 获取字段 type/ui_type
- **创建数据表**：明确需求时在 `create` 时通过 `table.fields` 一次性定义字段；探索场景用默认表 + 逐步修改
- **默认表空行**：`app.create` 自带的默认表有空记录，插入前先 `record.list` + `batch_delete` 清理
- **批量上限**：单次 ≤ 500 条，超过需分批
- **并发限制**：同一数据表不支持并发写，需串行调用 + 延迟 0.5-1 秒

## 快速索引

| 用户意图 | 工具 | action | 必填参数 | 常用可选 |
|---------|------|--------|---------|---------|
| 查表有哪些字段 | feishu_bitable_app_table_field | list | app_token, table_id | - |
| 查记录 | feishu_bitable_app_table_record | list | app_token, table_id | filter, sort, field_names |
| 新增一行 | feishu_bitable_app_table_record | create | app_token, table_id, fields | - |
| 批量导入 | feishu_bitable_app_table_record | batch_create | app_token, table_id, records (≤500) | - |
| 更新一行 | feishu_bitable_app_table_record | update | app_token, table_id, record_id, fields | - |
| 批量更新 | feishu_bitable_app_table_record | batch_update | app_token, table_id, records (≤500) | - |
| 创建多维表格 | feishu_bitable_app | create | name | folder_token |
| 创建数据表 | feishu_bitable_app_table | create | app_token, name | fields |
| 创建字段 | feishu_bitable_app_table_field | create | app_token, table_id, field_name, type | property |
| 创建视图 | feishu_bitable_app_table_view | create | app_token, table_id, view_name, view_type | - |

## 核心约束

### 字段类型与值格式必须严格匹配

| type | ui_type | 字段类型 | 正确格式 | 常见错误 |
|------|---------|----------|---------|-----------|
| 11 | User | 人员 | `[{id: "ou_xxx"}]` | 传字符串 `"ou_xxx"` |
| 5 | DateTime | 日期 | `1674206443000`（毫秒） | 传秒时间戳或字符串 |
| 3 | SingleSelect | 单选 | `"选项名"` | 传数组 `["选项名"]` |
| 4 | MultiSelect | 多选 | `["选项1", "选项2"]` | 传字符串 |
| 15 | Url | 超链接 | `{link: "...", text: "..."}` | 只传字符串 URL |
| 17 | Attachment | 附件 | `[{file_token: "..."}]` | 传外部 URL |

**强制流程**：先 `field.list` 获取 type/ui_type → 按上表构造格式 → 错误码 `125406X` / `1254015` 表示格式不匹配。

**人员字段**：默认 open_id（ou_...），格式 `[{id: "ou_xxx"}]`，只能传 id 字段。

## 不要这样做

| 错误做法 | 正确做法 |
|---------|---------|
| 写记录前不查字段类型 | 必须先 `field.list` 获取 type/ui_type |
| 人员字段传字符串 `"ou_xxx"` | 必须传数组对象 `[{id: "ou_xxx"}]` |
| 日期字段传字符串 `"2026-02-27"` | 必须传毫秒时间戳 `1740614400000` |
| 单选字段传数组 `["选项"]` | 传字符串 `"选项"` |
| 单次批量超过 500 条 | 分批调用 |
| 并发写同一数据表 | 串行调用 + 延迟 0.5-1 秒 |

---

> 详细参考：使用 `fs.read` 读取
> - `$SKILL_DIR/references/examples.md` — 完整使用示例（查字段、批量导入、高级筛选）
> - `$SKILL_DIR/references/errors.md` — 错误码与排查
> - `$SKILL_DIR/references/appendix.md` — 资源层级、筛选 operator、使用限制、字段配置详解
