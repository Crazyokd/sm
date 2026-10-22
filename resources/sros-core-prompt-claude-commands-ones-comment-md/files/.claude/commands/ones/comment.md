# ONES 评论管理

查看工作项评论或添加新评论。

## 参数

$ARGUMENTS

解析规则：
- `<工作项ID>` → 查看该工作项的评论列表
- `<工作项ID> <评论内容>` → 添加评论

## 前置依赖

认证流程见 `common.md`。

---

## 执行流程

### Phase 1：认证 + 获取信息

按 `common.md` 认证，然后并行获取：
- 工作项详情：`GET /team/UNrQ5Ny5/task/<ID>/info`（获取 task_uuid）
- 成员列表：`GET /team/UNrQ5Ny5/members?limit=500`（评论作者 UUID→姓名）

### Phase 2A：查看评论（无评论内容参数时）

```
GET /team/UNrQ5Ny5/task/<task_uuid>/messages
```

过滤 `type == "discussion"` 的消息，格式化输出：

```
## #<ID> 评论（共 N 条）

**张三** · 2026-03-20 14:30
> 评论内容第一条

**李四** · 2026-03-21 09:15
> 评论内容第二条
```

- `from` 字段通过成员列表解析为姓名
- `send_time` 微秒级时间戳 ÷ 1000000 转格式化时间
- 无评论时提示"暂无评论"

### Phase 2B：添加评论（有评论内容参数时）

使用 python3 urllib 发送（避免 shell 转义问题）：

```python
import json, urllib.request, ssl, random, string

payload = json.dumps({
    "uuid": "".join(random.choices(string.ascii_letters + string.digits, k=8)),
    "text": "<评论内容>"
}).encode("utf-8")

# POST /team/UNrQ5Ny5/task/<task_uuid>/send_message
```

成功响应：`{"code": 200, "errcode": "OK"}`

输出：`已为 #<ID> 添加评论。`
