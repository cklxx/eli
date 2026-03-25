# Results TSV Format

## File Setup

```bash
# Create the file with header
echo -e "iteration\tcommit\tmetric\tdelta\tguard\tstatus\tdescription" > autoresearch-results.tsv

# Add to .gitignore (results.tsv must NEVER be committed)
echo "autoresearch-results.tsv" >> .gitignore
```

## Columns

| Column | Type | Description |
|--------|------|-------------|
| iteration | int | Sequential counter. 0 = baseline. |
| commit | string | 7-char git hash. `-` if reverted before commit (crash fix). |
| metric | float | Measured value from metric_cmd. `0.000000` for crashes. |
| delta | float | Change from previous best. `+` = increased, `-` = decreased. `0.0` for baseline/crash. |
| guard | enum | `pass`, `fail`, or `-` (no guard configured). |
| status | enum | One of: `baseline`, `keep`, `discard`, `crash`. |
| description | string | One sentence. What was tried. No tabs (breaks TSV). |

## Status Values

| Status | Meaning | Git action |
|--------|---------|------------|
| `baseline` | Initial measurement before any changes. Iteration 0. | None |
| `keep` | Metric improved and guard passed. Commit stays on branch. | None (branch advances) |
| `discard` | Metric did not improve, or guard failed after rework. | `git revert HEAD --no-edit` |
| `crash` | Run failed (OOM, error, timeout). | `git revert HEAD --no-edit` |

## Example

```tsv
iteration	commit	metric	delta	guard	status	description
0	a1b2c3d	0.997900	0.0	pass	baseline	initial measurement
1	b2c3d4e	0.993200	-0.004700	pass	keep	increase learning rate to 0.04
2	-	1.005000	+0.011800	-	discard	switch to GeLU activation
3	-	0.000000	0.0	-	crash	double model width (OOM)
4	c3d4e5f	0.991500	-0.001700	pass	keep	add warmup schedule for first 10% of steps
5	-	0.991800	+0.000300	-	discard	increase depth from 8 to 12
6	-	0.989200	-0.002300	fail	discard	inline attention computation (guard: clippy warnings)
7	d4e5f6g	0.988100	-0.003400	pass	keep	switch to RoPE positional encoding
```

## Reading the TSV

```bash
# Current best (last kept or baseline)
grep -E '\t(keep|baseline)\t' autoresearch-results.tsv | tail -1

# All kept improvements
grep '\tkeep\t' autoresearch-results.tsv

# Failure rate
grep -c '\tdiscard\t' autoresearch-results.tsv
grep -c '\tkeep\t' autoresearch-results.tsv

# Last 20 experiments
tail -20 autoresearch-results.tsv

# What was already tried (avoid repeats)
awk -F'\t' '{print $7}' autoresearch-results.tsv
```

## Rules

- Append after EVERY iteration, including crashes
- Never delete rows — the TSV is append-only
- Never commit to git — stays in .gitignore
- Use tabs, not commas (commas break in descriptions)
- Keep descriptions short and specific (no multi-sentence)
- Read last 20 rows at the start of each iteration
