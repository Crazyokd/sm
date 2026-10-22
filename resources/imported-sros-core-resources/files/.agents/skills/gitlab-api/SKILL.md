---
name: gitlab-api
description: 通过 GitLab REST API 查 CI 日志、操作 MR/Issue/Pipeline。适用于查 job trace、列 pipeline jobs、查/建/改 MR，以及任何 SROS/sros-core 项目下涉及 GitLab 的操作。触发词：GitLab、MR、merge request、pipeline、CI 日志、job trace、issue。
---

# GitLab API 访问

需要查 CI 日志、操作 MR/Issue 时，**优先用 REST API 而不是 `glab` CLI**——容器里 `glab` 不一定装。环境变量 `GITLAB_PRIVATE_TOKEN` 已注入到 DevContainer，直接 `curl` 即可。

- Base URL：`http://git.standard-robots.com/api/v4`
- 项目路径：`SROS%2Fsros-core`（URL-encoded 的 `SROS/sros-core`）
- 通用 header：`-H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN"`

## 常用片段

```bash
# 查 CI job 元信息（status、runner、failure_reason 等）
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "http://git.standard-robots.com/api/v4/projects/SROS%2Fsros-core/jobs/<job_id>"

# 拉 CI job 完整日志（即 GitLab UI 上显示的 raw trace）
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "http://git.standard-robots.com/api/v4/projects/SROS%2Fsros-core/jobs/<job_id>/trace"

# 列 pipeline 下所有 job
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "http://git.standard-robots.com/api/v4/projects/SROS%2Fsros-core/pipelines/<pipeline_id>/jobs"

# 查 MR 详情（含 head_sha、head_pipeline、description）
curl -s -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "http://git.standard-robots.com/api/v4/projects/SROS%2Fsros-core/merge_requests/<iid>"

# 创建 MR
curl -s -X POST -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "http://git.standard-robots.com/api/v4/projects/SROS%2Fsros-core/merge_requests" \
  --data-urlencode "source_branch=<branch>" \
  --data-urlencode "target_branch=main" \
  --data-urlencode "title=<title>" \
  --data-urlencode "description=<body>" \
  --data-urlencode "remove_source_branch=true" \
  --data-urlencode "squash=true"

# 更新 MR 描述
curl -s -X PUT -H "PRIVATE-TOKEN: $GITLAB_PRIVATE_TOKEN" \
  "http://git.standard-robots.com/api/v4/projects/SROS%2Fsros-core/merge_requests/<iid>" \
  --data-urlencode "description=<new body>"
```

## 注意事项

- 含中文、反引号、换行的 body **必须用 `--data-urlencode`**，不要用 `--data` 拼字符串
- 项目路径中的 `/` 必须 URL-encoded 成 `%2F`
- 返回是 JSON，建议管道接 `python3 -c "import json,sys; ..."` 提取关键字段
- 创建 MR 时默认带上 `remove_source_branch=true` 和 `squash=true`
