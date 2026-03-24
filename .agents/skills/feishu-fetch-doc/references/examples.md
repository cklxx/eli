# Wiki URL 处理示例

## 场景：用户发送知识库链接

用户：`帮我看下这个文档 https://xxx.feishu.cn/wiki/ABC123`

### 步骤

1. 调用 `feishu_wiki_space_node`（action: get, token: ABC123）
2. 返回 `obj_type: "docx"`, `obj_token: "doxcnXYZ789"`
3. 调用 `feishu_mcp_fetch_doc`（doc_id: doxcnXYZ789）

### 关键点

- wiki token（ABC123）和实际文档 token（doxcnXYZ789）是不同的
- 必须通过 `feishu_wiki_space_node` 解析后才能确定文档类型和实际 token
- 如果 `obj_type` 是 `sheet` 或 `bitable`，需要调用对应的工具而非 `feishu_mcp_fetch_doc`
