---
name: create-subagent
description: >-
  Create custom subagents for specialized AI tasks. Use when you want to create
  a new type of subagent, set up task-specific agents, configure code reviewers,
  debuggers, or domain-specific assistants with custom prompts.
disable-model-invocation: true
---
# Creating Custom Subagents

Create a focused Codex subagent using its native TOML configuration.

Infer the subagent's purpose, scope, and constraints from the conversation. Ask only for missing information that affects the configuration.

## Resolve Ownership

Locate the source of truth before editing. If SM manages the resource, modify its source and deliver the required review proposal or MR under the existing repository policy. Installation paths below describe consumers; do not bypass resource ownership by editing an installed copy.

Preserve unrelated changes and existing agent definitions. Creating a definition does not itself authorize running its business workflow.

## Native Codex Agents

For the Codex configuration represented in this environment:

- Project definitions target `.codex/agents/<name>.toml`.
- User definitions target `~/.codex/agents/<name>.toml`.
- Definitions use TOML fields such as `name`, `description`, and `developer_instructions`.

Confirm the target Codex version's supported fields and discovery behavior before adding optional settings.

Minimal definition:

```toml
name = "code-reviewer"
description = "Review the current change for correctness, security, and maintainability."
developer_instructions = """
Review the requested change and report actionable findings with file locations.
Focus on modified code and its relevant callers and tests.
Distinguish confirmed defects from questions and suggestions.
Do not modify code or perform unrelated operations.
"""
```

When converting an existing definition, preserve its instruction body and correctly escape TOML strings, including quotes, backslashes, and triple quotes. Do not blindly copy another runtime's `model`, `tools`, or permission fields; verify their native equivalents first. Preserve supported existing settings unless the requested change requires otherwise.

Verification:

1. Parse the proposed TOML and confirm required fields have the intended values.
2. Check that the parsed instruction string preserves the original instructions when converting.
3. Confirm the resource projects to the intended scope and path without replacing unrelated agents.
4. After the normal review and application process, verify discovery in the target runtime using a harmless task within the agent's intended scope.
5. Report parsing, application, discovery, and invocation separately; an unexecuted check remains unverified.
