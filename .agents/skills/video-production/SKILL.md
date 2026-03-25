---
name: video-production
description: Generate short videos with Seedance and save validated output files.
---

# video-production

Generate short videos via the ARK Seedance backend.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Generate a video | `generate` | `--prompt`, `--duration`, `--output` |

## Required Env

- `ARK_API_KEY`
- `SEEDANCE_ENDPOINT_ID`

## Usage

```bash
python3 $SKILL_DIR/run.py generate --prompt 'cute cat animation' --duration 5 --output /tmp/cat.mp4
```

## Parameters

| Name | Type | Required | Notes |
|------|------|----------|-------|
| prompt | string | yes | Video description |
| duration | number | no | Seconds, default `5` |
| output | string | no | Output path (default `/tmp/seedance_<ts>.mp4`) |

## Constraints

- Only `action=generate` is supported.
- Backend must return a video `url`; missing URL fails fast.
- Output file must be written and non-empty, otherwise `success=false`.
