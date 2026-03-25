# Skill SKILL.md 统一优化

## 问题

读了全部 31 个 SKILL.md，核心问题：

1. **中英混杂** — 有的全中文，有的全英文，有的混着来。标题 "Usage" vs "调用" vs "用法"，"Parameters" vs "参数"，"Constraints" vs "核心约束"。读起来割裂。
2. **结构不统一** — 每个 skill 自己一套格式。有的有快速索引表，有的没有。有的有参数表，有的就一句话。
3. **description 不可读** — frontmatter 的 description 是给路由/索引用的，但很多写得像内部术语："封装 AnyGenIO/anygen-skills 为统一 CLI（help/task），支持渐进式披露与任务执行"、"SOUL 自演进技能 — 在不可变段保护下更新可演进人格"。LLM 看不懂就选不对 skill。
4. **避坑格式不一** — 有的用 ❌/✅ 行内，有的用表格，有的两种都有。
5. **详略失当** — feishu-bitable 很详细（应该的），desktop-automation 只有两行命令（太少了）。

## 方案

### 语言：统一英文

- 正文、标题、描述全用英文
- 代码/命令/参数名保持英文
- frontmatter description 用英文

### 统一模板

两类 skill 用同一个骨架，只在"调用方式"处分叉：

```
---
name: xxx
description: 一句话说清做什么、什么时候用
[triggers/priority/etc. 保持原有，不动]
---

# {name}

> [sidecar 类] **调用方式：** `sidecar(tool="<tool>", params={...})`
> [CLI 类] 无此行

{1-2 句说明这个 skill 的核心能力}

## 速查

| 意图 | 工具/命令 | 关键参数 |
|------|----------|---------|

## [详情区] — 根据 skill 复杂度选用

### 工具说明 / 用法 / 参数 — 按需

## 约束（如有重要规则）

## 避坑

| 错误 | 正确 |
|------|------|
```

### 具体改动

| 维度 | 现状 | 目标 |
|------|------|------|
| 标题 | "快速索引"/"Usage"/"调用" | 统一 "速查" |
| 反模式 | ❌/✅ 混用 | 统一表格，列名"错误"/"正确" |
| description | 术语堆砌 | 动词开头，说清"做什么" |
| sidecar 提示 | 部分有 | sidecar 类全有，CLI 类无 |
| 参考文件指引 | 有的有有的没 | 保留原有指引，格式统一 |

### 不改的东西

- frontmatter 中的 triggers、priority、cooldown、requires_tools 等运行时配置
- `$SKILL_DIR/references/` 下的参考文件
- 实际逻辑内容（约束、参数、工具说明的实质内容）
- run.py 文件

## 执行

按类分批改：
1. feishu 系列（18 个）
2. 工具/开发类（13 个）

每个文件重写后 diff 检查内容不丢失。
