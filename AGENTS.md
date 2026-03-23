# Repository Guidelines

## Project Structure & Module Organization

Core Rust code lives under `crates/`:

- `crates/eli/src/main.rs`: CLI entrypoint.
- `crates/eli/src/framework.rs`: inbound turn orchestration and outbound dispatch.
- `crates/eli/src/hooks.rs`: hook traits, runtime ordering, and hook reports.
- `crates/eli/src/builtin/`: builtin agent runtime, CLI commands, channels, config, tape services, and tools.
- `crates/eli/src/channels/`: CLI, Telegram, and webhook channel adapters.
- `crates/eli/src/skills.rs` / `crates/eli/src/tools.rs`: skill discovery/rendering and tool registry.
- `crates/conduit/src/`: provider-agnostic LLM clients, execution, auth, and tape storage.

Supporting code lives in:

- `sidecar/src/`: Node/TypeScript OpenClaw bridge used by webhook channels.
- `docs/`: architecture, channel, deployment, and extension docs.
- `.github/workflows/`: CI for `check`, `test`, `fmt`, and `clippy`.

## Build, Test, and Development Commands

- `cargo build --workspace`: build all Rust crates.
- `cargo run -p eli -- chat`: run the interactive CLI.
- `cargo run -p eli -- run "hello"`: run one inbound message through the framework pipeline.
- `cargo run -p eli -- gateway`: start the channel listener mode.
- `cargo test --workspace`: run the Rust test suite.
- `cargo fmt --all`: format all Rust code.
- `cargo clippy --workspace -- -D warnings`: lint Rust code with warnings denied.
- `just build` / `just install` / `just test`: convenience wrappers for common cargo workflows.
- `cd sidecar && npm start`: run the sidecar locally.
- `cd sidecar && bun test`: run sidecar tests when touching bridge code.

## Coding Style & Naming Conventions

- Rust edition `2024`, 4-space indentation, and explicit types where they improve clarity.
- Use `snake_case` for modules/functions/variables, `PascalCase` for types, and `UPPER_SNAKE_CASE` for constants.
- Keep hook ordering, prompt assembly, and tool execution deterministic; avoid hidden side effects.
- Prefer cohesive refactors when the current runtime shape is the source of the bug instead of layering narrow patches on top.
- Format with `cargo fmt` and keep `cargo clippy -- -D warnings` clean.
- In `sidecar/`, preserve the existing TypeScript ESM style and keep runtime contracts aligned with the Rust side.

## Testing Guidelines

- Add or update Rust unit tests close to the changed code with `#[cfg(test)]`.
- Prefer behavior-oriented test names such as `test_build_system_prompt_appends_workspace_agents_guidance`.
- Use `tempfile` workspaces for tests that depend on filesystem state, tapes, prompts, or `AGENTS.md`.
- Cover prompt composition, hook precedence, channel routing, tape persistence, and tool wiring when changing runtime behavior.
- If sidecar behavior changes, add or update tests under `sidecar/test/`.

## Commit & Pull Request Guidelines

- Follow Conventional Commits such as `feat:`, `fix:`, `docs:`, and `chore:`.
- Keep commits focused; separate runtime, sidecar, and docs changes when they can stand alone.
- For PRs, include:
  - what changed and why
  - impacted crates, commands, or channels
  - verification performed (`cargo test`, `cargo fmt --check`, `cargo clippy`, sidecar tests if relevant)
  - docs updates when CLI behavior, configuration, or architecture changed

## Security & Configuration Tips

- Use `.env` for local secrets; never commit credentials.
- Eli runtime settings are driven by `ELI_*` variables such as `ELI_MODEL`, `ELI_API_KEY`, `ELI_API_BASE`, and `ELI_MAX_STEPS`.
- Provider-specific credentials may also be resolved from local auth stores handled by `conduit`.
- Webhook/sidecar deployments use `sidecar/sidecar.json`; Telegram deployments require `ELI_TELEGRAM_TOKEN`.
