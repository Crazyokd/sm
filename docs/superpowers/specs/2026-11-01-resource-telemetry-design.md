# Resource Telemetry Design

## Intent

Make resource usage evidence explicit, attributable, and useful for later optimization without blocking Codex or Kimi hooks.

## Goals

- Record resource lifecycle signals with stable resource, turn, and invocation identifiers.
- Distinguish native activation from file reads, tool execution, and path inference.
- Preserve backward compatibility with existing JSONL events and SQLite observations.
- Expose bounded usage summaries through SM health/status and the existing activity view.
- Keep the hot path local, append-only, redacted, and free of network or model calls.

## Evidence policy

- An explicit resource ID from a native activation or resource event is high confidence.
- An explicit resource ID on a generic tool event is medium confidence.
- A unique path match from `active.json` is low confidence and remains a fallback.
- Upstream resources remain excluded from self-resource optimization records.
- Missing attribution is represented as partial coverage; it is not treated as zero usage.

## Data model

Every new observation may carry `signal`, `detected_from`, `confidence`, `turn_id`, `invocation_id`, and `parent_invocation_id`. Existing observations deserialize through defaults.

The usage summary reports total observations, attributed observations, confidence counts, signal counts, and per-resource activation/load/tool/completion/failure counts for a bounded window.

## Non-goals

- No external OpenTelemetry dependency or exporter in this phase.
- No raw prompt, tool input, output, or file content retention.
- No native client modification beyond the existing SM collector hooks.
- No automatic resource edits, retirement, or MR creation from the new counters.

## Acceptance criteria

1. Replayed explicit activation events produce high-confidence activation observations with correlation IDs.
2. Unique active-path matches produce low-confidence observations and ambiguous paths remain unattributed.
3. Existing event files and observations remain readable.
4. Usage summaries distinguish activation from generic tool execution and expose attribution coverage.
5. Health, status, diagnostics, and the activity page expose the same summary.
6. Tests cover duplicate IDs, failed activation, nested correlation, upstream exclusion, and retention-window aggregation.
