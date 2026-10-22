---
name: forge-current-session
description: Summarize the active Codex session into one human-readable developer-knowledge document, with optional Feishu publishing through the existing current-session forge runner. Use when the user asks to summarize, distill, crystallize,沉淀,整理当前 session,输出一篇博客/文档, or turn the current conversation into a shareable document. Only generate AI artifacts, rules, playbooks, or skill changes when the user explicitly asks for them.
---

# Forge Current Session

Use this skill to turn the active Codex session into one publishable document. The output should be a concrete article, not a transcript dump, status report, or generic learning advice.

The deterministic runner scopes every topic and source reference to the active `CODEX_THREAD_ID`, writes into a unique experiment directory, and leaves unrelated knowledge-repo changes untouched.

## Default Document Run

For the user's normal request, generate exactly one developer-knowledge document and publish it unless they ask for a local-only draft:

```bash
python3 "$HOME/.agents/skills/forge-current-session/scripts/run.py" \
  --max-articles 1 \
  --max-ai-artifacts 0 \
  --no-hermes-review
```

This keeps the run focused on the human-facing document path: current session -> knowledge article -> Git commit/push -> Feishu publish. It disables AI artifact generation and Hermes skill review because those are not needed for a single blog/document.

Use a local-only draft when the user asks to preview first or avoid side effects:

```bash
python3 "$HOME/.agents/skills/forge-current-session/scripts/run.py" \
  --max-articles 1 \
  --max-ai-artifacts 0 \
  --no-hermes-review \
  --no-publish \
  --no-push \
  --no-commit
```

Only use the broader artifact mode when the user explicitly asks for reusable AI artifacts or skill/rule/playbook candidates:

```bash
python3 "$HOME/.agents/skills/forge-current-session/scripts/run.py" \
  --max-articles 1 \
  --max-ai-artifacts 3
```

Only pass `--session-id` when the user explicitly wants a session other than the active one. Run `--help` for the complete option list.

## Document Quality

The article must capture what someone can reuse from the session:

- the concrete problem or goal;
- the system boundaries and source-of-truth decisions;
- the implementation or investigation path;
- failures, corrections, and verification evidence;
- the final operational rule or reusable practice.

Avoid filler such as where to learn the topic, generic advice, or chronological chat narration. If the session contains no durable knowledge, say so instead of publishing a padded document.

## Completion

Wait for exit code 0. Report the experiment `run_root`, the single generated `knowledge_documents` path, Git commits if any, and Feishu URLs from `published_documents`. Do not claim success from partial logs or a still-running process. If more than one knowledge document is generated, report that as a deviation.
