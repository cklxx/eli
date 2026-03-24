---
name: feishu-get
description: "1 tools: feishu_get_user"
---

Call tools via: sidecar(tool="<name>", params={...})

## feishu_get_user
获取用户信息。不传 user_id 时获取当前用户自己的信息；传 user_id 时获取指定用户的信息。返回用户姓名、头像、邮箱、手机号、部门等信息。

