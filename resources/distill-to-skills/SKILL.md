---
name: distill-to-skills
description: Use when the user asks to distill, crystallize, retain, 沉淀, or 复盘 durable lessons from the active conversation into reusable Skills shared across AI agents.
---

# Distill to Skills

Turn the active conversation into the smallest durable change to a shared Skill
catalog. Treat the catalog as the source of truth; agent installation directories
are consumers, not independent copies.

## Select The Lesson

Keep a lesson only when it is:

- reusable in more than the current task;
- supported by verification, a user correction, or concrete source evidence;
- specific enough to change a future agent's behavior;
- safe to store without credentials or private data.

Reject transient status, raw conversation summaries, unverified conclusions,
temporary identifiers, and facts that merely describe the current run.

## Locate The Source

Resolve the loaded Skill package to its owning Git repository, then inventory the
repository's active `SKILL.md` files before writing. Preserve unrelated working-tree
changes.

When the owning repository cannot be identified unambiguously, do not write into
`.agents`, `.codex`, `.claude`, `.hermes`, or other installation directories.
Return a proposed package or patch and name the unresolved source-of-truth issue.

## Choose One Outcome

Choose exactly one:

- `no-op`: the lesson is weak, unverified, task-specific, or already covered;
- `patch`: an existing Skill owns the same trigger or responsibility;
- `create`: the lesson is reusable and no existing Skill can own it cleanly.

Default to `patch`. Prefer one broad Skill with labeled sections over narrow Skills
with overlapping triggers.

## Apply The Change

For `patch`, change only the owning Skill and any directly required support file.
Keep its existing style and strengthen the smallest relevant instruction.

For `create`, use a lowercase hyphenated directory name and write a `SKILL.md` with
only `name` and `description` in YAML frontmatter. Put all trigger wording in the
description. Keep the body imperative and under 500 lines.

Add `references/`, `scripts/`, `templates/`, or `assets/` only when the Skill cannot
stay concise or deterministic without them. Keep relative links one level deep and
verify every link. Never encode an agent-specific runtime dependency unless that
runtime is the explicit subject of the Skill.

Do not delete or archive existing Skills. Do not commit, push, publish, or modify
installation links unless the user explicitly asks.

## Verify And Report

Validate frontmatter, directory naming, non-empty instructions, relative links,
and the final Git diff. Confirm that no secret, absolute host path, session ID, or
unrelated edit entered the package.

Report:

- outcome: `no-op`, `patch`, or `create`;
- target Skill and changed paths;
- evidence that justified retention;
- validation performed;
- anything deliberately left as a proposal.
