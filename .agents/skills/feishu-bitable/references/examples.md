# Bitable 使用场景示例

## 场景 1: 查字段类型（必做第一步）

```json
{
  "action": "list",
  "app_token": "S404b...",
  "table_id": "tbl..."
}
```

**返回**：包含每个字段的 `field_id`、`field_name`、`type`、`ui_type`、`property`

## 场景 2: 批量导入客户数据

```json
{
  "action": "batch_create",
  "app_token": "S404b...",
  "table_id": "tbl...",
  "records": [
    {
      "fields": {
        "客户名称": "Bytedance",
        "负责人": [{"id": "ou_xxx"}],
        "签约日期": 1674206443000,
        "状态": "进行中"
      }
    },
    {
      "fields": {
        "客户名称": "飞书",
        "负责人": [{"id": "ou_yyy"}],
        "签约日期": 1675416243000,
        "状态": "已完成"
      }
    }
  ]
}
```

**字段值格式**：
- 人员：`[{id: "ou_xxx"}]`（数组对象）
- 日期：毫秒时间戳
- 单选：字符串
- 多选：字符串数组

**限制**: 最多 500 条记录

## 场景 3: 筛选查询（高级筛选）

```json
{
  "action": "list",
  "app_token": "S404b...",
  "table_id": "tbl...",
  "filter": {
    "conjunction": "and",
    "conditions": [
      {
        "field_name": "状态",
        "operator": "is",
        "value": ["进行中"]
      },
      {
        "field_name": "截止日期",
        "operator": "isLess",
        "value": ["ExactDate", "1740441600000"]
      }
    ]
  },
  "sort": [
    {
      "field_name": "截止日期",
      "desc": false
    }
  ]
}
```

**filter 说明**：
- 支持 10 种 operator（is/isNot/contains/isEmpty 等，见 appendix.md）
- **isEmpty/isNotEmpty 必须传 `value: []`**（API 要求必须传空数组）
- 日期筛选可使用 `["Today"]`、`["ExactDate", "时间戳"]` 等
- `sort` 可指定多个排序字段

## 场景 4: 字段 Property 配置

创建/更新字段时需要的 `property` 参数结构（单选的 options、进度的 min/max、关联的 table_id 等），详见同目录下的 `appendix.md`。

## 场景 5: 记录值数据结构

每种字段类型在记录中对应的 `fields` 值格式（人员字段只传 id、日期是毫秒时间戳、附件需先上传等），详见同目录下的 `appendix.md`。
