---
name: feishu-wiki
description: "Manage Feishu wiki spaces and nodes -- list, create, move, copy nodes, and resolve wiki tokens to actual document types."
---

# feishu-wiki

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Manages Feishu wiki spaces (knowledge bases) and their document nodes, including type resolution from wiki tokens to actual document tokens.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| List wiki spaces | `feishu_wiki_space` | action=list |
| Get wiki space info | `feishu_wiki_space` | action=get, space_id |
| Create wiki space | `feishu_wiki_space` | action=create |
| List wiki nodes | `feishu_wiki_space_node` | action=list, space_id |
| Get node info (wiki -> obj_token conversion) | `feishu_wiki_space_node` | action=get, node_token |
| Create/move/copy node | `feishu_wiki_space_node` | action=create/move/copy |

## Tools

### feishu_wiki_space

Manages Feishu wiki spaces. Use when the user wants to list, inspect, or create knowledge bases.

**Actions:** list (list spaces), get (get space info), create (create space).

**Notes:**
- `space_id` can be extracted from the browser URL or retrieved via the list action
- A wiki space is a top-level knowledge base container holding hierarchically organized document nodes

### feishu_wiki_space_node

Manages nodes within a Feishu wiki space. Nodes represent documents of various types (doc, bitable, sheet, etc.).

**Actions:** list, get, create, move, copy.

**Key concepts:**
- `node_token` is the wiki node identifier
- `obj_token` is the actual document token
- Use the get action to convert a wiki `node_token` to its `obj_type` and `obj_token`, then call the appropriate tool for that document type

## Pitfalls

| Wrong | Right |
|-------|-------|
| Treat a wiki URL directly as a docx and read it | Use `feishu_wiki_space_node` get to resolve `obj_type`, then call the correct tool (doc -> read-doc, sheet -> feishu_sheet, etc.) |
| Use `feishu_wiki_space_node` list to search wiki documents | Use `feishu_search_doc_wiki` from feishu-search for searching; list only shows the hierarchy |
| Confuse `node_token` and `obj_token` | `node_token` is the wiki node ID; `obj_token` is the actual document ID; use the get action to convert |
