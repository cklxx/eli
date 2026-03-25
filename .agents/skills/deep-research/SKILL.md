---
name: deep-research
description: Conduct deep research with multi-source search, evidence compilation, and structured report generation.
triggers:
  intent_patterns:
    - "research|调研|调查|分析|analysis|market"
  tool_signals:
    - web_search
    - web_fetch
  context_signals:
    keywords: ["调研", "研究", "analysis", "research"]
  confidence_threshold: 0.6
priority: 8
exclusive_group: research
max_tokens: 200
cooldown: 300
requires_tools: [bash]
output:
  format: markdown
  artifacts: true
  artifact_type: document
---

# deep-research

Multi-source search, evidence compilation, and structured report generation for any research topic.

## Quick Reference

| Intent | Command | Key Params |
|--------|---------|------------|
| Basic research | `python3 $SKILL_DIR/run.py --topic '...'` | `--topic` (required) |
| Custom queries | `python3 $SKILL_DIR/run.py --topic '...' --queries '[...]'` | `--queries` |
| Advanced depth | `python3 $SKILL_DIR/run.py --topic '...' --depth advanced` | `--depth` |

## Usage

```bash
python3 $SKILL_DIR/run.py --topic 'Research topic' --queries '["keyword1","keyword2"]' --max_results 5 --depth basic
```

## Parameters

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| topic | string | yes | Research topic |
| queries | string[] | no | Search keywords; defaults to 3 auto-generated queries |
| max_results | int | no | Results per query, default 5 |
| depth | string | no | `basic` or `advanced` |
| fetch_urls | string[] | no | Additional URLs to fetch full text from |

## Output

Returns JSON containing `searches` (search results), `fetched_pages` (fetched pages), and `summary_prompt` (synthesis prompt).

The LLM organizes results into a structured report: Problem, Findings/Evidence, Confidence Level, Impact/Recommendations.
