---
name: autoresearch
description: Autonomous experiment loop — modify code, measure, keep or discard, repeat indefinitely. Based on Karpathy's autoresearch.
triggers:
  intent_patterns:
    - "autoresearch|experiment loop|autonomous experiment|自动实验"
  context_signals:
    keywords: ["autoresearch", "experiment", "optimize", "benchmark", "实验循环"]
  confidence_threshold: 0.6
priority: 8
requires_tools: [bash]
max_tokens: 200
cooldown: 60
---

# autoresearch

Autonomous experiment loop. You modify code, measure a metric, keep improvements, discard regressions. Loop forever until the human stops you.

Based on [Karpathy's autoresearch](https://github.com/karpathy/autoresearch). The agent IS the loop — no external orchestration.

## Setup (BLOCKING — do this before anything else)

Ask the user for ALL of the following in ONE message:

```
1. scope         — file glob to modify (e.g. "train.py", "src/**/*.rs")
2. metric_cmd    — shell command that prints a single number to stdout
3. direction     — "lower_is_better" or "higher_is_better"
4. guard_cmd     — [optional] must exit 0 to keep a change (e.g. "cargo clippy ...")
5. time_budget   — [optional] seconds per experiment, default 300
6. mode          — [optional] "unbounded" (default) or "bounded(N)"
7. branch_tag    — [optional] tag for autoresearch/<tag> branch, default today's date
```

After collecting answers:

1. Verify git repo is clean (`git status --porcelain` is empty)
2. Create branch: `git checkout -b autoresearch/<tag>`
3. Dry-run `metric_cmd` — must produce a parseable number
4. Dry-run `guard_cmd` (if provided) — must exit 0
5. Initialize `autoresearch-results.tsv` with header row (add to .gitignore)
6. Run baseline: execute metric_cmd, record as iteration 0 with status "baseline"
7. Confirm setup and begin the loop

## The Loop

Read the full protocol: `$SKILL_DIR/references/loop-protocol.md`

Summary:
```
LOOP FOREVER:
  1. Review  — read results.tsv (last 20), git log --oneline -20
  2. Modify  — edit scope files with ONE experimental idea, git commit
  3. Run     — metric_cmd > run.log 2>&1 (timeout = 2x time_budget)
  4. Evaluate — extract metric, run guard if present, compare to best
  5. Decide  — improved? KEEP. Not improved? REVERT. Crashed? fix or skip
  6. Log     — append results.tsv
  → GOTO 1
```

## 8 Critical Rules

1. **NEVER STOP.** Do not pause. Do not ask "should I continue?" The human may be asleep. Loop until interrupted.
2. **Single change per iteration.** If description needs "and", split it.
3. **Commit BEFORE running.** Every experiment has a git hash.
4. **Redirect all output.** `> run.log 2>&1` — never flood context with experiment output.
5. **Mechanical metrics only.** No "looks better" or "seems cleaner". Numbers or nothing.
6. **Never modify the metric.** The evaluation is sacred. Only modify files matching `scope`.
7. **Simplicity wins.** Same metric + simpler code = keep. Tiny gain + ugly complexity = discard.
8. **Git is memory.** Read results.tsv and git log before each iteration to avoid repeating failures.

## Reference Files

- `$SKILL_DIR/references/loop-protocol.md` — Phase-by-phase protocol with exact commands
- `$SKILL_DIR/references/results-format.md` — TSV column spec and examples
