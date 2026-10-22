# SM Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Make local execution and evidence diagnosable with bounded storage, safe retention and one redacted diagnostic export.

**Architecture:** Reuse SQLite runs with typed result.run metadata and an active snapshot; keep Hook transport as JSONL plus small private health counters. Retain reference-pinned evidence and aggregate retired rows; fingerprint ordinary file ownership. Web and CLI consume the same health/diagnostic model.

**Tech Stack:** Rust 1.88, Axum, SQLite, strict TypeScript, journald, existing dependencies.

**Spec:** docs/superpowers/specs/2026-10-22-observability-design.md

## Global Constraints

- User/project scopes only; shared client-independent Skills; upstream excluded from optimization.
- No business script execution, unreviewed application, old-repository backups, new dependencies without need, or parallel product versions.
- Preserve local edits, shared configuration, executable bits, pending records and MR evidence.

## Review Focus

- Concurrent append/health updates and partial final JSONL records must remain recoverable.
- Cancellation or process death must not leave a falsely running/successful task.
- Timeout diagnostics must remain bounded and redact nested/inline secrets.
- Retention must preserve proposal/finding evidence and all unprocessed observations.
- File fingerprints must retain the original conflict and shared-target cleanup safeguards.

### Task 1: Runtime tracing (worker A)

Files: src/telemetry.rs, src/service.rs, src/process.rs, tests/telemetry.rs, tests/service.rs, tests/process.rs.

Interfaces: public serializable RunDetails and RunTracker; final business report stays unchanged except typed result.run metadata. `telemetry::snapshot(&State)` supplies current and recent task details. State add_run stays the sole final history append. Do not edit state.rs/lib.rs/web.rs/cli.rs.

- [x] Add behavioral tests for start/progress/success/failure/interruption and stderr redaction on timeout.
- [x] Implement run lifecycle and stage/resource updates, journald summaries, bounded timeout context.
- [x] Run the owned integration tests and fmt/Clippy; record actual results.

### Task 2: Evidence and retention (worker B)

Files: src/observe.rs, src/retention.rs, tests/observations.rs, tests/retention.rs.

Interfaces: `observe::capture_health(directory)` for per-client counters; `observe::record_failure(agent,directory,error)` for CLI capture failures; `retention::maintain(&State)` for the nightly locked job; `retention::summary(&State)` for health; public UsageTotal for retired observation event totals and last_seen (no invented lifetime distinct-session count). Do not edit service.rs/cli.rs/lib.rs/state.rs/web.rs.

- [x] Test concurrent counters, native outcome normalization without retaining complete output, unknown evidence and source filtering.
- [x] Test age/cap cleanup with pending/pinned/partial records, persisted long-term totals and idempotent reingestion.
- [x] Implement the specified defaults and visible protected-over-budget result; run owned tests.

### Task 3: Compact ownership (worker C)

Files: src/model.rs, src/state.rs, src/apply.rs, tests/state.rs, tests/model_apply.rs.

Interfaces: new stored ordinary-file fingerprint, atomically migrate existing typed File ownership and compact only after conversion. Desired writes still carry actual bytes; shared native fragments retain current representation. Do not edit service.rs/lib.rs/observe.rs/web.rs/cli.rs.

- [x] Test populated-state conversion/reopen, large-state reduction and malformed-state non-destruction.
- [x] Test safe updates/removal, permissions, conflicts, shared owner failures and symlink boundaries after conversion.
- [x] Implement storage and comparison changes; run owned tests and Clippy.

### Task 4: Health, export and integration (root)

Files: src/diagnostics.rs, src/lib.rs, src/cli.rs, src/web.rs, assets/templates/jobs.html, assets/templates/activity.html, assets/static/style.css, frontend/app.ts as needed, tests/diagnostics.rs, tests/web.rs, docs/usage.md.

- [x] Add tests for bounded redacted export and CLI/web parity, current run and capture/backlog/storage display.
- [x] Integrate worker APIs and nightly maintenance; show persisted event totals without mislabeling use/session counts.
- [x] Add CLI and private download endpoint; surface stages, last events and retention warnings in existing UI.
- [ ] Run full Rust/TypeScript checks, isolated browser flows and independent Rust/security/code reviews; fix findings.
- [ ] Commit/push/create MR, wait CI, merge under existing authorization, verify main CI, build and update local service.
- [ ] Verify real migration size, capture/diagnostic output, service/timer and retained resources; remove only task-owned obsolete iteration artifacts.
