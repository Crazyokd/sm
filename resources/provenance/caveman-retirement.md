# Caveman 清退

按用户决定，清退 Caveman 的 Codex 原生插件与同名 marketplace、7项独立用户级Skill、其唯一命名来源和22条接管记录。同时移除7条已无资源对应的禁用选择器，保留其他启停配置的顺序与值。

插件与缓存使用 Codex 原生 `plugin remove caveman@caveman` 和 `plugin marketplace remove caveman` 生命周期清理；不编辑上游插件正文。独立Skill只删除仍匹配SM所有权的22个文件，本地修改继续报冲突。其他插件、MCP、Hook、用户规则、会话与凭据不因本次清退而修改。

Caveman自身的运行数据、历史统计和原生信任数据不导入SM。本文记录清退意图，实际本机完成状态与验证证据见交付MR。
