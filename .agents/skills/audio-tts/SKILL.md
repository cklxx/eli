---
name: audio-tts
description: Generate speech audio using macOS native TTS (say + afconvert), outputting m4a files.
triggers:
  intent_patterns:
    - "语音|读出来|朗读|TTS|配音|发个语音"
  context_signals:
    keywords: ["语音", "朗读", "tts", "配音", "音频"]
  confidence_threshold: 0.6
priority: 6
requires_tools: [bash, write]
max_tokens: 200
cooldown: 30
output:
  format: markdown
  artifacts: true
  artifact_type: audio
---

# audio-tts

Generate speech audio using macOS built-in `say` command and convert to m4a format.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Speak text | `$PYTHON $SKILL_DIR/run.py speak --text '...'` | `--text` (required) |
| Custom voice | `$PYTHON $SKILL_DIR/run.py speak --text '...' --voice Samantha` | `--voice` |

## Usage

```bash
$PYTHON $SKILL_DIR/run.py speak --text 'Hello, this is a speech test'
```

## Parameters

### speak

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| text | string | yes | Text to speak |
| voice | string | no | Voice name (e.g. Ting-Ting / Samantha), defaults to system default |
| rate | int | no | Speech rate in WPM (passed to `say -r`) |
| output | string | no | Output path, default `/tmp/tts_<ts>.m4a` |

## Constraints

- Requires macOS (built-in `say` and `afconvert` commands).
