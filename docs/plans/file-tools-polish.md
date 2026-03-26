# 2026-03-25 · File Tools Polish

**Status**: ✅ Complete

## Goal

Make the text file tools reliable on large files and keep their read/edit loop token-cheap:

- `fs.read` should stream exact text slices without normalizing line endings
- `fs.write` should avoid partial overwrites
- `fs.edit` should work on large files without loading the whole file first

## Results

| Goal | Implementation | Test |
|------|---------------|------|
| Stream reads, preserve line endings | `read_text_window` via `BufReader::read_line` | `test_fs_read_preserves_original_newlines` |
| Atomic writes | `AtomicTextWriter` (temp file → `persist()`) | `test_fs_write_preserves_existing_permissions` |
| Large-file streaming edit | `replace_stream` sliding window, flush prefix | `test_fs_edit_streams_large_files` (>50MB) |
| Preserve file permissions | `existing_permissions` + `apply_permissions` in `AtomicTextWriter` | `test_fs_write_preserves_existing_permissions` (unix) |
| CRLF preservation | Byte-exact streaming, no normalization | `test_fs_edit_preserves_crlf_and_trailing_newline` |
| Start offset skip | `copy_prefix_lines` streams prefix to writer | `test_fs_edit_start_skips_earlier_matches` |
