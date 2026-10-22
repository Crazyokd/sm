# Comprehensive resource governance implementation plan

> For agentic workers: use superpowers:subagent-driven-development or superpowers:executing-plans; independent implementation tasks use dispatching-parallel-agents.

**Goal:** Bring the user's existing user and project AI resources under reviewed, visible management and detect subsequent omissions.

**Architecture:** Keep catalog source and scope dimensions. Add a typed bundle projection for native files/fragments, explicit native installer declarations for external plugins, and a bounded inventory of the supported resource locations. Reviewed adoption verifies existing bytes/fragments before recording ownership; normal application retains unmanaged-file conflicts.

**Tech stack:** Existing Rust, SQLite, MiniJinja and TypeScript; no new dependencies or Python manager.

**Spec:** User decisions in README: one mainline; user/project only; upstream follows upstream without optimization; self changes take effect only after MR review, merge and pull/apply. Existing disabled states and local credentials are preserved.

## Global constraints

- Never commit authentication, runtime memory, session history or credentials.
- Do not manually install plugin caches or silently enable disabled resources.
- Read-only discovery reports errors and missing directories; coverage must not silently become complete.
- Initial takeover preserves active content and native trust; local edits remain conflicts.

## Review focus

- Symlinks, traversal and whole config files must not bypass projection boundaries.
- Native install failure must not claim success or leave disabled plugins enabled.
- Existing config credentials/comments and unmanaged entries must survive fragment takeover.
- A new resource added after deployment must appear as unmanaged.
- Missing/upstream failures retain old owned files and report partial results.

## Tasks

- [ ] Add Bundle shared payload model and safe native file/fragment renderer; test rules/prompts, traversal, fragments and modes.
- [ ] Add explicit native installation manifest and bounded native lifecycle; test failure, enablement preservation and isolated roots.
- [ ] Add bounded resource discovery and ownership/alias/drift coverage; test new unmanaged resources and private native config.
- [ ] Integrate reviewed exact-content adoption, startup/nightly inventory, CLI and visual coverage view; test no application of unreviewed content.
- [ ] Import existing self resources and register upstream sources/native packages without vendoring upstream bodies; validate real disabled/secret/ownership state.
- [ ] Run Rust/TypeScript checks, independent code/security review, MR pipeline, merge and real local pull/apply; verify UI and native discovery.
