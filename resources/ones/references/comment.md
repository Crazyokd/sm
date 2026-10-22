# 评论

查看或新增 ONES 评论时，用这个流程。

## 查看评论

1. 先查 task info 拿到 task UUID
2. 再查 members 解析评论作者
3. 获取 messages
4. 只保留 `type == "discussion"`
5. 按作者、时间、内容格式化输出

时间字段是微秒。

## 新增评论

推荐请求体：

```python
payload = {
    "uuid": "<8 random chars>",
    "text": "<comment text>"
}
```

发送到：

`POST /team/UNrQ5Ny5/task/<task_uuid>/send_message`

如果用户想追加处理状态，优先用 [common.md](./common.md) 里的 AI 标记。

## 输出

- 查看模式：按时间顺序展示评论
- 写入模式：确认更新了哪条工单
