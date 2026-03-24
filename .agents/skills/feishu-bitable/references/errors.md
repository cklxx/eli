# Bitable 错误码与排查

| 错误码 | 错误现象 | 根本原因 | 解决方案 |
|--------|---------|---------|---------|
| 1254064 | DatetimeFieldConvFail | 日期字段格式错误 | **必须用毫秒时间戳**（如 `1772121600000`），不能用字符串（`"2026-02-27"`、RFC3339）或秒级时间戳 |
| 1254068 | URLFieldConvFail | 超链接字段格式错误 | **必须用对象** `{text: "显示文本", link: "URL"}`，不能直接传字符串 URL |
| 1254066 | UserFieldConvFail | 人员字段格式错误或 ID 类型不匹配 | 必须传 `[{id: "ou_xxx"}]`，确认 `user_id_type` |
| 1254015 | Field types do not match | 字段值格式与类型不匹配 | 先 list 字段，按类型构造正确格式 |
| 1254104 | RecordAddOnceExceedLimit | 批量创建超过 500 条 | 分批调用，每批 ≤ 500 |
| 1254291 | Write conflict | 并发写冲突 | 串行调用 + 延迟 0.5-1 秒 |
| 1254303 | AttachPermNotAllow | 附件未上传到当前表格 | 先调用上传素材接口 |
| 1254045 | FieldNameNotFound | 字段名不存在 | 检查字段名（包括空格、大小写） |

## 错误码分类

- `125408X` — property 结构错误（创建/更新字段时）→ 查 appendix.md 字段 Property 配置
- `125406X` — 字段值转换失败（写入记录时）→ 检查值格式是否匹配字段类型
- `1254015` — 字段类型不匹配 → 先 `field.list` 获取类型再构造值
