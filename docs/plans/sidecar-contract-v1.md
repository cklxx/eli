# 2026-03-27 · Rust↔sidecar contract v1

## How Would This Fail?

1. 继续在 Rust 和 TypeScript 两边各写一份隐式 shape，字段不会消失，但语义会漂
2. 一上来做运行时 validator 或 trait 重构，blast radius 会大于这次目标
3. 只补单边测试，另一侧仍然可以静默漂移

## Scope

- 收敛 3 个 Rust↔sidecar payload：channel message、tool request、notice request
- 引入显式 `contract_version`
- 增加共享 schema / fixture tests

## Plan

1. 在 Rust 侧定义 typed contract，并导出 versioned schema bundle
2. webhook channel 和 sidecar bridge 统一经过 contract payload
3. Rust 与 bun tests 共同消费一组 fixtures，锁住跨语言语义

## Rollback

- 如果 runtime 校验带来兼容问题，保留 typed structs 和 fixtures，先回退 strict version check
