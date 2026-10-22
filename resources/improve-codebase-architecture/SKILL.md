---
name: improve-codebase-architecture
description: Review a codebase for evidence-backed architecture improvements, coupling reduction, clearer module ownership, and better testability. Use when the user asks for architecture review, refactoring opportunities, consolidation of tightly coupled code, or a more AI-navigable codebase. Prefer CodeGraph for structural analysis and stay read-only unless implementation or documentation changes are explicitly requested.
---

# Improve Codebase Architecture

Find the smallest architecture changes that remove demonstrated friction. Do not manufacture abstractions or redesign code that is already easy to change and test.

## Contract

- Default to a read-only review. Do not edit code, `CONTEXT.md`, or ADRs unless the user asks.
- Follow repository instructions and existing domain language before this skill's terminology.
- Ground every finding in current source, callers, tests, or documented decisions.
- Prefer a small consolidation or deletion over a new interface, layer, or dependency.
- Distinguish a current code limitation from an architectural necessity.

## Gather Context

1. Read the applicable `AGENTS.md` files and repository instructions.
2. Read `CONTEXT.md`, `CONTEXT-MAP.md`, and relevant `docs/adr/` entries when present. Their absence is not a reason to create them.
3. Inspect the current working tree so unrelated user changes stay out of scope.
4. Use CodeGraph when available:
   - Start broad architecture questions with `codegraph_context`.
   - Use `codegraph_trace` for a specific source-to-target flow.
   - Use one `codegraph_explore` call for the relevant symbol bodies.
   - If `codegraph_explore` truncates an omitted symbol, use `codegraph_node` for that symbol instead of restarting discovery with native search.
   - Use `codegraph_impact` before recommending a change to a shared symbol.
5. If CodeGraph reports pending files, read only those files directly. If CodeGraph is unavailable, fall back to `rg` and focused file reads.

Do not repeat structural exploration with a second search method after CodeGraph has answered it.

## Evaluate Candidates

Look for evidence that a change would improve at least one of these:

- **Locality:** one behavior currently requires coordinated edits across several places.
- **Interface cost:** callers must understand implementation details, ordering, or hidden invariants.
- **Ownership:** the same domain rule is implemented by multiple modules.
- **Testability:** important behavior can only be tested through fragile internal details.
- **Change amplification:** a small requirement repeatedly causes broad edits.
- **Pass-through code:** a layer forwards data without hiding meaningful complexity.

Apply the deletion test before proposing a new abstraction: if removing a module makes complexity disappear, delete or inline it; if complexity spreads into callers, the module may be earning its place.

Reject candidates that are speculative, cosmetic, unsupported by current change history, or justified only by future flexibility.

For dependency-heavy candidates, read [DEEPENING.md](DEEPENING.md). Use [LANGUAGE.md](LANGUAGE.md) only as optional analytical vocabulary; repository terminology wins.

## Report

Present findings first, ordered by expected value and confidence. For each finding include:

- Exact files or symbols involved.
- Concrete evidence and the maintenance or testing cost.
- The smallest viable change.
- Tests or checks that would prove the change safe.
- Confidence: `high`, `medium`, or `speculative`.

End with one top recommendation and explain why it should be first. If no candidate clears the evidence bar, say that clearly instead of inventing work.

Use a diagram only when it materially clarifies a flow or dependency relationship. Do not create an HTML report unless the user explicitly requests one.

## Follow-up

- If the user asks to implement a candidate, define success criteria, make the smallest scoped change, and verify behavior before and after.
- If the user wants an interactive design challenge or documentation updates, treat that as a separate task. Do not edit glossary or ADR files merely because this review discovered a candidate.
- Only revisit an ADR when current evidence demonstrates material friction, and identify the conflict explicitly.
