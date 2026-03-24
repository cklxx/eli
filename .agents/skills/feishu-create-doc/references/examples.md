# feishu-create-doc 使用示例

## 示例 1：创建简单文档

```json
{
  "title": "项目计划",
  "markdown": "## 项目概述\n\n这是一个新项目。\n\n## 目标\n\n- 目标 1\n- 目标 2"
}
```

## 示例 2：创建到指定文件夹

```json
{
  "title": "会议纪要",
  "folder_token": "fldcnXXXXXXXXXXXXXXXXXXXXXX",
  "markdown": "## 周会 2025-01-15\n\n## 讨论议题\n\n1. 项目进度\n2. 下周计划"
}
```

## 示例 3：使用飞书扩展语法

高亮块、表格等飞书特有功能：

```json
{
  "title": "产品需求",
  "markdown": "<callout emoji=\"💡\" background-color=\"light-blue\">\n重要需求说明\n</callout>\n\n## 功能列表\n\n<lark-table header-row=\"true\">\n| 功能 | 优先级 |\n|------|--------|\n| 登录 | P0 |\n| 导出 | P1 |\n</lark-table>"
}
```

## 示例 4：创建到知识库节点下

```json
{
  "title": "技术文档",
  "wiki_node": "wikcnXXXXXXXXXXXXXXXXXXXXXX",
  "markdown": "## API 接口说明\n\n这是一个知识库文档。"
}
```

## 示例 5：创建到知识空间根目录

```json
{
  "title": "项目概览",
  "wiki_space": "7448000000000009300",
  "markdown": "## 项目概览\n\n这是知识空间根目录下的一级文档。"
}
```

## 示例 6：创建到个人知识库

```json
{
  "title": "学习笔记",
  "wiki_space": "my_library",
  "markdown": "## 学习笔记\n\n这是创建在个人知识库中的文档。"
}
```
