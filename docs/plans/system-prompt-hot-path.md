# 2026-03-27 · build_system_prompt 热路径收敛

## How Would This Fail?

1. framework 继续绕过 `build_system_prompt`，hook 表面和真实执行路径越来越分叉
2. 修复方式如果改 hook trait，会把 blast radius 扩到所有实现者
3. 直接在 agent 内重建 prompt 会保留隐藏耦合，测试也抓不住

## Scope

- 只修正 Rust 主热路径
- 不改 `EliHookSpec` 签名
- 不动 sidecar 协议

## Plan

1. framework 在 `build_prompt` 后解析 system prompt，并显式写入 turn state
2. builtin agent 优先消费预计算的 system prompt，没有时回退到本地构建
3. 增加测试证明 hook 产出的 system prompt 会影响 `run_model` 热路径

## Rollback

- 如果行为出现偏差，先保留 state 注入测试，回退 agent 消费逻辑到本地构建
- 不做序列化格式变更，因此回滚只涉及 Rust 代码路径
