# Resource Telemetry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a local, confidence-aware resource telemetry protocol and usage summary to SM.

**Architecture:** Extend the current JSONL observation envelope and SQLite-backed observations with strict lifecycle and attribution fields. Derive bounded summaries from stored observations and surface them through health/status/diagnostics and the existing activity page.

**Tech Stack:** Rust 2024, serde/serde_json, rusqlite, chrono, existing Axum/minijinja web UI.

**Spec:** `docs/superpowers/specs/2026-11-01-resource-telemetry-design.md`

## Global Constraints

- Keep the hook hot path local and non-blocking with respect to external services.
- Use the current unreleased observation schema strictly; do not add compatibility readers.
- Redact and bound event payloads; never persist raw prompt/tool output/file content.
- Do not analyze or mutate upstream resource content.
- Do not add an external telemetry dependency in this phase.

## Review Focus

- Explicit activation with a valid self resource must outrank path inference and retain correlation IDs.
- Shared or ambiguous paths must remain unattributed instead of being guessed.
- Records missing the new telemetry fields must fail validation rather than silently changing evidence quality.
- Retired or upstream IDs must not become self-resource optimization evidence.
- A failed or truncated event must remain visible in health and usage quality counters.

### Task 1: Typed resource signals and attribution

**Files:**
- Modify: `src/observe.rs`
- Test: `tests/observations.rs`

**Interfaces:**
- Produces strict `Observation` fields for signal, attribution source, confidence, turn ID, invocation ID, and parent invocation ID.
- Produces deterministic classification helpers for lifecycle signals and attribution confidence.

- [x] Write failing tests for explicit activation, path inference, ambiguous paths, and nested correlation IDs.
- [x] Run the focused observation tests and verify they fail for missing fields/classification.
- [x] Implement typed enums, strict deserialization, payload extraction, and classification in `src/observe.rs`.
- [x] Run the focused tests and verify they pass.

### Task 2: Bounded usage summary

**Files:**
- Modify: `src/observe.rs`
- Modify: `src/diagnostics.rs`
- Test: `tests/observations.rs`, `tests/diagnostics.rs`

**Interfaces:**
- Adds `resource_usage_summary(state, window)` with total/attributed counts, confidence counts, signal counts, and per-resource counters.
- Adds the summary to `Health` without changing existing fields.

- [x] Write failing tests for activation/load/tool/completion/failure counts and the retention window.
- [x] Run the focused tests and verify they fail.
- [x] Implement bounded SQLite-backed aggregation with stable sorting.
- [x] Add the summary to diagnostics health and verify tests pass.

### Task 3: Status and activity visibility

**Files:**
- Modify: `src/web.rs`
- Modify: `assets/templates/activity.html`
- Test: `tests/web.rs`

**Interfaces:**
- Status/diagnostic JSON exposes the same health summary.
- Activity rows show confidence/source and separate activation counts from total observations.

- [x] Add failing web assertions for the new summary labels and confidence fields.
- [x] Implement the smallest template/model changes using the shared health summary.
- [x] Run web tests and verify they pass.

### Task 4: Full verification

**Files:**
- Modify: `docs/usage.md`

- [x] Document evidence levels and the meaning of usage counters.
- [x] Run `cargo fmt --check`.
- [x] Run `cargo clippy --all-targets --locked -- -D warnings`.
- [x] Run `cargo test --locked`.
- [x] Run the Hindsight Python regression check and TypeScript checks when applicable.
- [x] Review the diff and run the independent code-review pass.
