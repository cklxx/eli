# Autoresearch Loop Protocol

This is the complete, authoritative protocol for the autonomous experiment loop.
Follow it exactly. Do not improvise on the structure — improvise only on what experiments to try.

---

## Phase 0: Precondition Check

Before entering the loop, verify:

```bash
# Git repo exists and is clean
git status --porcelain  # must be empty (except autoresearch-results.tsv)

# Not on main/master
git branch --show-current  # must be autoresearch/<tag>

# No detached HEAD
git symbolic-ref HEAD  # must succeed
```

If any check fails, fix it before proceeding. Do not enter the loop with dirty state.

---

## Phase 1: Review

**Every iteration starts here.** Read what happened before.

```bash
# What was tried and what happened
tail -20 autoresearch-results.tsv

# What commits survived (the chain of kept improvements)
git log --oneline -20

# Current best metric (last "keep" or "baseline" in TSV)
grep -E '\t(keep|baseline)\t' autoresearch-results.tsv | tail -1
```

**Use this to:**
- Avoid repeating experiments that already failed
- Identify what directions are working (look at kept experiments)
- Find combinations: if A improved and B improved, maybe A+B is worth trying
- Spot patterns: are architectural changes working better than hyperparameter tuning?

**If you're stuck (5+ consecutive discards):**
- Re-read the scope files from scratch — fresh eyes find new angles
- Try the opposite of what you've been doing
- Combine two previously-kept changes in a new way
- Try radical changes (different architecture, different algorithm)
- Read comments/papers referenced in the code for inspiration
- Try simplifying: remove something and see if the metric holds

---

## Phase 2: Modify

Make ONE change to files matching the scope glob.

**Rules:**
- One logical change per iteration. If your description needs "and" linking unrelated things, split.
- Match the existing code style (naming, error handling, formatting).
- Prefer small, targeted changes over sweeping rewrites.
- The simplicity criterion applies: all else equal, simpler wins.

**Commit before running:**
```bash
git add <changed files within scope>
git commit -m "experiment(<scope>): <one-line description of what you changed and why>"
```

The commit exists BEFORE you know if it works. This gives you a hash to log and a clean revert target.

---

## Phase 3: Run

Execute the metric command with output capture and timeout.

```bash
timeout $((TIME_BUDGET * 2)) bash -c "$METRIC_CMD" > run.log 2>&1
exit_code=$?
```

**Critical: redirect everything.** `> run.log 2>&1`. Never let experiment output flood your context window.
This is not optional — large outputs will degrade your ability to reason about subsequent experiments.

**Timeout handling:**
- timeout = 2x time_budget (generous, catches hangs without false positives)
- If timeout fires (exit code 124): treat as crash, revert, log, move on
- If you need to check output: `grep` or `tail` specific lines from run.log, never `cat` the whole thing

---

## Phase 4: Evaluate

Extract the metric and compare to the current best.

```bash
# Extract metric (adapt to your metric_cmd's output format)
metric=$(tail -5 run.log | grep -oE '[0-9]+\.?[0-9]*' | tail -1)

# If metric is empty, the run crashed
if [ -z "$metric" ]; then
    echo "CRASH — no metric output"
    tail -50 run.log  # read the error
fi
```

**Run the guard (if configured):**
```bash
eval "$GUARD_CMD" > guard.log 2>&1
guard_exit=$?
```

**Compare to current best:**
- Read the best metric from results.tsv (last line with status "keep" or "baseline")
- For `lower_is_better`: improved if `metric < best`
- For `higher_is_better`: improved if `metric > best`
- Noise threshold (optional): if `|metric - best| < min_delta`, treat as no improvement

---

## Phase 5: Decide

### Case 1: Improved + guard pass → KEEP

The experiment worked. The commit stays. The branch advances.

```bash
# Nothing to do — the commit is already on the branch
echo "KEEP: $metric (was $best)"
```

Update the "current best" for future comparisons.

### Case 2: Improved + guard fail → REVERT + REWORK

The metric improved but guard failed (e.g., clippy warnings, test failures).

```bash
# Revert the change
git revert HEAD --no-edit

# Read guard output to understand what broke
cat guard.log
```

Then: try a different implementation that achieves the same goal without breaking the guard.
Maximum 2 rework attempts. After 2 failures, discard the idea and move on.
**Never modify guard/test files — adapt the implementation instead.**

### Case 3: Not improved → REVERT

```bash
# Preferred: revert preserves the experiment in history (memory)
git revert HEAD --no-edit

# Fallback if revert has merge conflicts:
git revert --abort 2>/dev/null
git reset --hard HEAD~1
```

Why `git revert` over `git reset`:
- Revert preserves the failed experiment as a commit (the agent can see it in `git log`)
- Reset destroys the commit (agent loses memory of what was tried)
- Revert is non-destructive (no safety warnings)
- Fallback to reset only if revert creates merge conflicts

### Case 4: Crash

Classify and respond:

| Crash type | Action |
|-----------|--------|
| Syntax error / typo | Fix immediately, re-run. Don't count as iteration. |
| Runtime error (logic bug) | Max 3 fix attempts. Log each attempt. |
| Resource exhaustion (OOM) | Revert. Try smaller variant of the same idea. |
| Timeout / hang | Kill, revert, try different approach. |
| Fundamental incompatibility | Revert, log "crash", move on. Don't retry. |

```bash
# For crashes: revert to clean state
git revert HEAD --no-edit  # or git reset --hard HEAD~1 if revert conflicts

# Read the error
tail -50 run.log
```

---

## Phase 6: Log

Append one row to `autoresearch-results.tsv` after EVERY iteration, including crashes.

```bash
echo -e "$ITERATION\t$COMMIT\t$METRIC\t$DELTA\t$GUARD\t$STATUS\t$DESCRIPTION" >> autoresearch-results.tsv
```

See `$SKILL_DIR/references/results-format.md` for column spec.

**Do NOT commit results.tsv to git.** It stays untracked (in .gitignore).

**Progress summary:** Every 10 iterations, print a brief summary:
```
--- iteration 30 ---
baseline: 0.9979 → current best: 0.9842 (−1.37%)
kept: 8 | discarded: 19 | crashed: 3
last 5: discard, keep, discard, discard, crash
```

---

## Phase 7: Loop

Go back to Phase 1. **Do not stop.**

### Bounded mode

If mode is `bounded(N)`: after N iterations, print a final summary and stop.

```
=== AUTORESEARCH COMPLETE (50 iterations) ===
Baseline:    0.9979 (commit a1b2c3d)
Best:        0.9770 (commit f6g7h8i)  [−2.09%]
Iterations:  50 (12 kept, 33 discarded, 5 crashed)

Top improvements:
  1. −0.0050  increase dim to 768 (commit b2c3d4e)
  2. −0.0032  switch to Muon optimizer (commit c3d4e5f)
  3. −0.0028  add sliding window attention (commit d4e5f6g)
```

### Unbounded mode

**NEVER STOP.** Never ask "should I continue?" Never say "this seems like a good stopping point."
The human might be asleep and expects 100 experiments by morning.

If you run out of ideas:
1. Re-read the scope files completely — look for inefficiencies you missed
2. Try reversing a previous change to see if the context has shifted
3. Combine two previously-successful changes
4. Try something radical you haven't considered
5. Look for dead code, unnecessary computation, or suboptimal defaults
6. Read any comments or references in the code for new approaches

The loop runs until the human interrupts you. Period.

---

## VRAM / Resource Constraint

> VRAM is a soft constraint. Some increase is acceptable for meaningful metric gains,
> but it should not blow up dramatically.

This applies to any resource metric (memory, disk, CPU time) that isn't the primary metric.
Track it in the TSV but don't optimize for it unless it becomes a problem.

---

## The Simplicity Criterion

> All else being equal, simpler is better.
> A 0.001 improvement that adds 20 lines of hacky code? Probably not worth it.
> A 0.001 improvement from deleting code? Definitely keep.
> An improvement of ~0 but much simpler code? Keep.

This is a tiebreaker, not a veto. A large improvement justifies complexity.
But when the metric difference is marginal, always choose simpler.
