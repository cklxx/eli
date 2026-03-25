---
name: feishu-create-doc
description: Create a Feishu cloud document from Lark-flavored Markdown, with optional folder or wiki placement.
---

# feishu-create-doc

> **Tool calling:** Use `sidecar(tool="<tool_name>", params={...})` to call tools in this skill.

Creates a new Feishu cloud document from Lark-flavored Markdown content. Returns `doc_id`, `doc_url`, and `message`.

## Quick Reference

| Intent | Tool | Key Params |
|--------|------|------------|
| Create a document | `feishu_create_doc` | markdown (required), title, folder_token, wiki_node, wiki_space |

## Parameters

| Parameter | Required | Description |
|-----------|----------|-------------|
| markdown | Yes | Lark-flavored Markdown content |
| title | No | Document title |
| folder_token | No | Parent folder token (`fldcnXXXX`); omit to create in personal root |
| wiki_node | No | Wiki node token or URL (`wikcnXXXX`); mutually exclusive with folder_token/wiki_space |
| wiki_space | No | Wiki space ID; special value `my_library` for personal wiki; mutually exclusive with wiki_node/folder_token |

**Priority:** wiki_node > wiki_space > folder_token

## Content Guidelines

The markdown content should be well-structured, visually rich, and readable:

- **Clear structure**: Heading depth no more than 4 levels; use callouts to highlight key information
- **Visual rhythm**: Break up long text with dividers, columns, and tables
- **Visual diagrams**: Prefer Mermaid/PlantUML for flows and architecture diagrams
- **Restraint**: Don't overuse callouts; bold only core terms

When the user has explicit style/formatting preferences, follow those instead.

### Common Extended Syntax

- Callout: `<callout emoji="💡" background-color="light-blue">content</callout>`
- Columns: `<grid cols="2"><column>left</column><column>right</column></grid>`
- Enhanced table: `<lark-table header-row="true"><lark-tr><lark-td>content</lark-td></lark-tr></lark-table>`
- Image: `<image url="https://..." width="800" align="center" caption="description"/>`
- File: `<file url="https://..." name="document.pdf"/>`
- Mermaid diagram: ` ```mermaid ` code block
- Mention user: `<mention-user id="ou_xxx"/>`
- Text color: `<text color="red">red text</text>`

## Pitfalls

| Wrong | Right |
|-------|-------|
| Start markdown with an H1 identical to the title | The title is already the document title; start markdown from body content |
| Manually add a table of contents | Feishu auto-generates the TOC |
| Use `doc_media` insert for URL-based images | Use `<image url="..."/>` syntax |
| Create an extremely long document in one call | Use update-doc append mode to create in segments |

---

> Detailed references: use `fs.read` to view
> - `$SKILL_DIR/references/examples.md` -- full usage examples
> - `$SKILL_DIR/LARK_MARKDOWN_REFERENCE.md` -- complete Lark-flavored Markdown syntax reference
