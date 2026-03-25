---
name: feishu-update-doc
description: Update a Feishu cloud document with 7 modes including append, overwrite, targeted replace, insert, and delete.
---

# feishu-update-doc

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Updates Feishu cloud documents with 7 operation modes. Prefer targeted updates over overwrite, which clears the entire document and may destroy images, comments, and collaboration history.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| Append to end | `feishu_update_doc` | mode=append, markdown |
| Full overwrite (destructive) | `feishu_update_doc` | mode=overwrite, markdown |
| Replace matched range | `feishu_update_doc` | mode=replace_range, selection, markdown |
| Find-and-replace all | `feishu_update_doc` | mode=replace_all, selection, markdown (empty string = delete) |
| Insert before match | `feishu_update_doc` | mode=insert_before, selection, markdown |
| Insert after match | `feishu_update_doc` | mode=insert_after, selection, markdown |
| Delete matched range | `feishu_update_doc` | mode=delete_range, selection |

## Modes

| Mode | Purpose | Requires Selection | Requires Markdown |
|------|---------|-------------------|-------------------|
| append | Append to end | No | Yes |
| overwrite | Full overwrite (destructive) | No | Yes |
| replace_range | Replace a unique match | Yes | Yes |
| replace_all | Replace all occurrences | Yes | Yes (empty string = delete) |
| insert_before | Insert before match | Yes | Yes |
| insert_after | Insert after match | Yes | Yes |
| delete_range | Delete matched content | Yes | No |

Optional parameter `new_title`: plain text, 1-800 characters, combinable with any mode.

## Selection Methods (choose one)

### selection_with_ellipsis -- Content-based selection

- **Range match**: `start_text...end_text` -- matches everything from start to end; use 10-20 characters for uniqueness
- **Exact match**: `full_text` (no `...`) -- matches the complete text literally
- **Escaping**: literal `...` must be written as `\.\.\.`

### selection_by_title -- Heading-based selection

Format: `## Section Title` (with or without `#` prefix). Automatically selects the entire section from the heading up to the next heading of the same or higher level.

## Constraints

### Use small, precise replacements
The smaller the selection range, the safer the operation. For nested blocks (tables, columns), target only the specific text that needs changing.

### Protect non-rebuildable content
Images, whiteboards, spreadsheets, bitables, and tasks are stored as tokens and cannot be read out and written back intact. Avoid selecting these areas; target only plain text.

### Insert mode boundaries
- `insert_after` inserts after the **end** of the matched range
- `insert_before` inserts before the **start** of the matched range

When expanding a selection for uniqueness, verify the boundary is still the intended insertion point.

### Prefer incremental over wholesale
Use multiple small replacements for multiple changes. Overwrite destroys media, comments, and collaboration history.

## Pitfalls

| Wrong | Right |
|-------|-------|
| Large-range replace covering areas with images/whiteboards | Target only the plain text portion to avoid breaking token references |
| Casually use overwrite mode | Overwrite destroys images and comments; prefer targeted updates |
| Don't escape literal `...` in selection_with_ellipsis | Use `\.\.\.` for a literal three-dot sequence |
| Expand selection for insert but ignore boundary shift | `insert_after` inserts after the match end; `insert_before` before the match start |

---

> Detailed references: use `fs.read` to view
> - `$SKILL_DIR/references/examples.md` -- examples for all 7 modes
> - `$SKILL_DIR/references/appendix.md` -- return value format, new_title parameter details
> - Markdown syntax reference: see the feishu-create-doc skill documentation
