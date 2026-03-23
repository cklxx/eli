# Repository Guidelines

## Project Structure & Module Organization

Core code lives under `src/`:

- `src/eli/__main__.py`: Typer CLI entrypoint.
- `src/eli/framework.py`: turn orchestration and outbound routing.
- `src/eli/hookspecs.py` / `src/eli/hook_runtime.py`: hook contracts and execution helpers.
- `src/eli/builtin/`: builtin runtime, CLI wiring, settings, tools, and tape services.
- `src/eli/channels/`: channel abstractions plus CLI and Telegram adapters.
- `src/eli/skills.py` / `src/eli/tools.py`: skill discovery and tool registry.
- `src/skills/`: bundled skills shipped with Eli.

Tests live in `tests/`. Documentation lives in `docs/`.

## Build, Test, and Development Commands

- `uv sync`: install or update dependencies.
- `just install`: sync dependencies and install `prek` hooks.
- `uv run eli chat`: run the interactive CLI.
- `uv run eli gateway`: start channel listener mode.
- `uv run eli run "hello"`: run one inbound message through the full framework pipeline.
- `uv run eli hooks`: inspect discovered hook bindings.
- `uv run ruff check .`: lint checks.
- `uv run mypy src`: static type checks.
- `uv run pytest -q`: run the main test suite.
- `just test`: run pytest with doctests enabled.
- `just check`: lock validation, lint, and typing.
- `just docs` / `just docs-test`: serve or build docs.

## Coding Style & Naming Conventions

- Python 3.12+, 4-space indentation, and type hints for new or modified logic.
- Use `snake_case` for modules/functions/variables, `PascalCase` for classes, and `UPPER_CASE` for constants.
- Keep functions focused and composable; avoid hidden side effects.
- Prefer the ideal end-state design over a minimal patch when the current architecture is the real source of the bug; large refactors are allowed when they simplify the runtime and remove behavioral mismatches.
- Format and lint with Ruff. Keep line length within 120 unless an existing file clearly follows a different local convention.

## Testing Guidelines
- Framework: `pytest`.
- Name test files `tests/test_<feature>.py`.
- Prefer behavior-oriented test names such as `test_gateway_uses_enabled_channels_only`.
- Cover hook precedence, turn lifecycle, CLI/channel behavior, and tape persistence when changing runtime behavior.
- Update or add tests in the same change when behavior moves.

## Commit & Pull Request Guidelines

- Follow the Conventional Commit style used in history, for example `feat:`, `fix:`, `docs:`, `chore:`.
- Keep commits focused; avoid mixing unrelated refactors with behavior changes.
- When committing implementation work, prefer multiple focused commits split by logical change area instead of one large commit.
- For PRs, include:
  - what changed and why
  - impacted modules or commands
  - verification performed (`ruff`, `mypy`, `pytest`, docs build if relevant)
  - docs updates when CLI behavior, commands, or architecture changed

## Security & Configuration Tips

- Use `.env` for local secrets; never commit credentials.
- Eli runtime settings are driven by `ELI_*` variables such as `ELI_MODEL`, `ELI_API_KEY`, and `ELI_API_BASE`.
- Provider-specific keys such as `OPENROUTER_API_KEY` may still be consumed by downstream SDKs.
- Telegram deployments usually require `ELI_TELEGRAM_TOKEN`, and allowlists are controlled with `ELI_TELEGRAM_ALLOW_USERS` and `ELI_TELEGRAM_ALLOW_CHATS`.
