---
name: gitlab-api
description: 通过 GitLab REST API 查 CI 日志、操作 MR/Issue/Pipeline。适用于查 job trace、列 pipeline jobs、查/建/改 MR，以及任意 GitLab 仓库下涉及 GitLab 的操作。触发词：GitLab、MR、merge request、pipeline、CI 日志、job trace、issue。
---

# GitLab API 访问

需要查 CI 日志、操作 MR/Issue 时，**优先用 REST API 而不是 `glab` CLI**——容器里 `glab` 不一定装。优先复用本机已有认证；环境确实提供 `GITLAB_PRIVATE_TOKEN` 时可用 `curl`，否则使用已认证的 `glab api`。认证缺失或请求失败就明确报告，不假定 token 已设置，不扫描、打印或截取凭据。

- Base URL：`http://git.standard-robots.com/api/v4`
- 通用 header：`-H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"`

## 项目路径

不要写死项目路径。进入目标仓库后，先从 `origin` remote 推导 GitLab project path：

```bash
GITLAB_API="${GITLAB_API:-http://git.standard-robots.com/api/v4}"
PROJECT_PATH="$(
  git remote get-url origin |
  sed -E 's#^git@[^:]+:##; s#^https?://[^/]+/##; s#\.git$##'
)"
PROJECT_ENC="$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$PROJECT_PATH")"
```

如果当前目录不是目标仓库，手动指定即可：

```bash
GITLAB_API="${GITLAB_API:-http://git.standard-robots.com/api/v4}"
PROJECT_PATH="SROS/platforms/easygo"
PROJECT_ENC="$(python3 -c 'import sys, urllib.parse; print(urllib.parse.quote(sys.argv[1], safe=""))' "$PROJECT_PATH")"
```

## 常用片段

```bash
# 查 CI job 元信息（status、runner、failure_reason 等）
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "$GITLAB_API/projects/$PROJECT_ENC/jobs/<job_id>"

# 拉 CI job 完整日志（即 GitLab UI 上显示的 raw trace）
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "$GITLAB_API/projects/$PROJECT_ENC/jobs/<job_id>/trace"

# 列 pipeline 下所有 job
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "$GITLAB_API/projects/$PROJECT_ENC/pipelines/<pipeline_id>/jobs"

# 查 MR 详情（含 head_sha、head_pipeline、description）
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "$GITLAB_API/projects/$PROJECT_ENC/merge_requests/<iid>"

# 创建 MR
curl -s -X POST -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "$GITLAB_API/projects/$PROJECT_ENC/merge_requests" \
  --data-urlencode "source_branch=<branch>" \
  --data-urlencode "target_branch=main" \
  --data-urlencode "title=<title>" \
  --data-urlencode "description=<body>" \
  --data-urlencode "remove_source_branch=true" \
  --data-urlencode "squash=true"

# 更新 MR 描述
curl -s -X PUT -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "$GITLAB_API/projects/$PROJECT_ENC/merge_requests/<iid>" \
  --data-urlencode "description=<new body>"
```

## 注意事项

- 含中文、反引号、换行的 body **必须用 `--data-urlencode`**，不要用 `--data` 拼字符串
- 项目路径必须从仓库 remote 推导或手动指定，不要把某个仓库名写死到命令里
- 项目路径中的 `/` 必须 URL-encoded 成 `%2F`；优先用上面的 `PROJECT_ENC` 片段生成
- 返回是 JSON，建议管道接 `python3 -c "import json,sys; ..."` 提取关键字段
- 创建 MR 时默认带上 `remove_source_branch=true` 和 `squash=true`
