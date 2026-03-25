# 2026-03-25 · File Tools Polish

## Goal

Make the text file tools reliable on large files and keep their read/edit loop token-cheap:

- `fs.read` should stream exact text slices without normalizing line endings
- `fs.write` should avoid partial overwrites
- `fs.edit` should work on large files without loading the whole file first

## Failure Modes

- `fs.read` strips CRLF or trailing newlines, so `fs.edit` cannot match what it just read
- `fs.edit` rejects large files because it reads the entire file before replacing
- overwrite paths leave partially written files behind on failure
- new write paths change file permissions on existing files

## Execution

1. Replace whole-file reads with line-preserving streaming reads.
2. Move write/edit paths onto temp-file rewrite plus atomic persist.
3. Add regression tests for CRLF, offsets, large-file edits, and existing permissions.
4. Run `fmt`, `clippy`, and `test`, then prune any extra abstraction.
