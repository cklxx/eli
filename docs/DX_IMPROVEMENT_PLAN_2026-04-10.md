# 2026-04-10 · DX Improvement Plan

## Goal

Turn `eli` from a strong author-driven system into a collaboration-ready platform.

The plan has two objectives:

1. improve day-to-day developer and agent workflow
2. make the repository safer to scale to many maintainers

---

## Principles

- prefer workflow hardening over premature rewrites
- encode common tasks as commands, not tribal knowledge
- keep docs current and clearly separate historical snapshots
- make CI reflect the real multi-language system
- treat hooks, tools, and sidecar integration as platform contracts

---

## Plan

### Phase 1 · Make the entrypoints trustworthy

### Deliverables

- current `README.md`
- current `docs/index.md`
- historical docs clearly labeled as snapshots

### Outcome

New contributors can find one reliable documentation entrypoint without reverse-engineering the repo history.

---

### Phase 2 · Standardize local workflows

### Deliverables

- `just doctor`
- `just check`
- `just test-rust`
- `just test-py`
- `just test-sidecar`
- `just test-all`
- `just release-check`
- supporting scripts in `scripts/`

### Outcome

Humans and agents can run the right validation path with stable, memorable commands.

---

### Phase 3 · Make CI match the system

### Deliverables

- fast Rust CI lane
- Python smoke lane
- sidecar test/build lane
- scheduled or gated heavier integration lane
- path-aware job triggering where useful

### Outcome

The repository stops behaving like a Rust-only project in automation.

---

### Phase 4 · Improve maintainer readiness

### Deliverables

- current `CONTRIBUTING.md`
- PR and issue templates
- `CODEOWNERS`
- review expectations by change type

### Outcome

The project becomes easier to maintain by many people without depending on implicit author context.

---

### Phase 5 · Harden platform contracts

### Deliverables

- hook contract spec
- tool contract spec
- sidecar contract spec
- compatibility and deprecation guidance
- ADRs for major architecture decisions

### Outcome

The framework becomes safer to extend and easier to evolve without accidental contract drift.

---

### Phase 6 · Make performance work repeatable

### Deliverables

- standard perf command(s), for example `just perf`
- trace collection workflow
- stable artifact location for measurements
- optional benchmark automation for critical paths

### Outcome

Performance work moves from one-off investigation to repeatable engineering workflow.

---

## Rollout Order

### First

1. docs entrypoints
2. local commands and scripts
3. CI expansion

### Next

4. contributing and review surfaces
5. ownership and ADRs
6. contract documentation

### Then

7. performance tooling
8. deeper runtime observability improvements

---

## Success Criteria

The plan is working if these become true:

- new contributors can start from one trustworthy docs entrypoint
- the common local workflows are command-driven and repeatable
- sidecar and Python regressions no longer bypass CI silently
- maintainers have an explicit contribution path
- major extension surfaces have written contracts

---

## Final Recommendation

Do not spend the next cycle on a large rewrite.

Spend it on hardening the project around the core:

1. trustworthy docs
2. stable local workflows
3. system-shaped CI
4. maintainer governance
5. contract clarity

That is the shortest path from a strong project to a top-tier, many-maintainer AI framework.
