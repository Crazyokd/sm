---
name: git-hunk-staging
description: Stage only your own hunks when committing in a working tree that may contain others' or concurrent uncommitted changes. Use when committing in a shared/active working tree, after parallel work, or whenever a file holds changes you did not author. Trigger: "git add whole file", "commit swept in WIP", "concurrent edits", "stage only my changes".
---

# Git Hunk Staging

Never commit someone else's work by accident. In a working tree that may hold
concurrent or foreign uncommitted changes, whole-file `git add <file>` stages
EVERYTHING in that file — including half-finished, broken, or unreviewed work
you did not author. It then gets committed, pushed, and possibly deployed.

## Rules

1. **Confirm ownership before staging.** Run `git status` and `git diff` and
   verify every change you are about to commit is yours. If files are being
   modified by another process (mtime changing, other agents working), treat
   them as hostile to your commit.
2. **Stage only your hunks.**
   - Interactive: `git add -p <file>` and answer per hunk.
   - Non-interactive shell: `git diff -- <file> | git apply --cached` after
     trimming the diff to your hunks, or `git add -e` to delete foreign hunks
     in the editor.
3. **Verify the commit before pushing.** `git show --stat HEAD` and
   `git show HEAD -- <file>`; confirm no foreign changes are in the commit.
4. **Do not reset + reapply in a live tree.** `git checkout -- <file>` then
   re-applying your changes races with concurrent writers and re-pollutes the
   file. Coordinate with the other worker instead, or isolate in a worktree.

## Why

Real incident (2026-08-30): a commit used whole-file `git add` and swept in a
parallel developer's in-progress requirement-lifecycle WIP (API endpoint
renames, store logic rewrites) that was incomplete and failing tests. The
contaminated commit was pushed and deployed to a server running the half-done
code. The failure was two stacked errors: not confirming concurrent writers,
and not staging per-hunk.
