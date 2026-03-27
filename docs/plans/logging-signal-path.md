# 2026-03-27 · sidecar 日志收敛 + no-reply 关键 tracing

## How Would This Fail?

1. sidecar 继续按每个注册、每次 dispatch、每次 typing 细节刷屏，真正有用的信息被噪音淹掉
2. Rust turn 主链路缺少关键节点 tracing，消息丢在 `run_model → render_outbound → dispatch_outbound` 之间时看不出来
3. 为了排查问题直接把全文本和全 context 打出来，会把日志变成另一种噪音甚至泄露过多内容

## Scope

- 标准化 sidecar 文本日志格式
- 增加颜色和日志级别过滤，默认 `INFO`
- 把高频 sidecar 日志降到 `DEBUG`
- 在 Rust 主链路只补长度、计数和跳过原因，不打印整段用户内容

## Plan

1. sidecar logger 支持 `ELI_SIDECAR_LOG_LEVEL`/`LOG_LEVEL`、TTY 颜色和短时间戳
2. registry/runtime/bridge 把高频注册、typing、逐条 outbound 日志降到 `DEBUG`
3. framework 和 agent_run 补 turn 级别 tracing：`run_model completed`、`collect_outbounds completed`、`agent.run finished`
4. builtin `dispatch_outbound` 对真正会丢消息的分支发 `WARN`

## Verification

- `cargo fmt --all -- --check`
- `cargo clippy --workspace -- -D warnings`
- `cargo test --workspace`
- `cd sidecar && bun test`
