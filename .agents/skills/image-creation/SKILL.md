---
name: image-creation
description: Generate or refine images with Seedream (text-to-image and image-to-image).
triggers:
  intent_patterns:
    - "生成图片|画|draw|image|图片|插图|illustration|设计图|海报"
  context_signals:
    keywords: ["图片", "image", "draw", "画", "生成", "设计"]
  confidence_threshold: 0.6
priority: 7
exclusive_group: image
requires_tools: [bash, write]
max_tokens: 200
cooldown: 30
output:
  format: markdown
  artifacts: true
  artifact_type: image
---

# image-creation

Generate images via Seedream text-to-image, or refine existing images with image-to-image.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Text to image | `generate` | `--prompt`, `--style`, `--size`, `--watermark` |
| Image to image | `refine` | `--image_path`, `--prompt`, `--watermark` |

## Required Env

- `ARK_API_KEY` (required)
- `SEEDREAM_TEXT_ENDPOINT_ID` (optional; fallback: `SEEDREAM_TEXT_MODEL` then built-in default)
- `SEEDREAM_I2I_ENDPOINT_ID` (required for `refine`)

## Usage

```bash
# Text to image
$PYTHON $SKILL_DIR/run.py generate --prompt 'white cat in moonlight' --style realistic --watermark false

# Image to image
$PYTHON $SKILL_DIR/run.py refine --image_path /tmp/cat.png --prompt 'add starry sky background' --watermark false
```

## Parameters

### generate

| Name | Type | Required | Notes |
|------|------|----------|-------|
| prompt | string | yes | Image description |
| style | string | no | Style tag (default: `realistic`) |
| size | string | no | `WIDTHxHEIGHT`, default `1920x1920` |
| watermark | bool | no | Default `false`; enable API watermark |
| output | string | no | Output file path (default `/tmp/seedream_<ts>.png`) |

### refine

| Name | Type | Required | Notes |
|------|------|----------|-------|
| image_path | string | yes | Input image path |
| prompt | string | yes | Refinement instruction |
| watermark | bool | no | Default `false`; enable API watermark |
| output | string | no | Output path (default `/tmp/seedream_refined_<ts>.png`) |

## Constraints

- Backend minimum pixels: `1920*1920`. Smaller inputs (e.g. `1024x1024`) are auto-upscaled.
- `success=true` only when the output file is actually written and non-empty.
- Backend response must contain `b64_json` or `url`; otherwise the call fails.
- Default output path is `/tmp` unless `output` is provided.
- `watermark` defaults to `false` (no "AI generated" watermark). Set to `true` only when you explicitly need it.
