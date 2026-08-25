---
name: git-push
description: Use when the user explicitly asks to commit and push the current repository changes. Suitable for analyzing current changes, generating an English commit message that follows the repo's conventions, and completing the push. Not suitable for status-only checks, local-only commits, push-disallowed contexts, or any case where the user has not explicitly asked to write to the remote.
---

# Git Push

Only use this skill when the user explicitly asks to commit and push.

## Workflow

1. Run `git pull` first to fetch the latest remote changes.
2. If the pull output contains conflict markers such as `CONFLICT` or `Merge conflict`, stop immediately — do not commit or push — and clearly list the conflicting files.
3. Analyze the changes in parallel:
   - `git status --short`
   - `git diff HEAD`
4. Generate an English commit message. The commit type must be chosen from:
   - `feat`
   - `fix`
   - `doc`
   - `style`
   - `update`
   - `refactor`
   - `test`
   - `framework`
   - `revert`
   Use lowercase for the type because this repository's commitlint enforces lowercase conventional-commit types.
5. The commit message format must be:

```text
type: brief summary of the change

- Change detail 1
- Change detail 2
```

6. Do not add any AI-generated sign-off.
7. Before staging or committing, run the CI checks locally:
   - `npx vue-tsc --noEmit`
   - `npm run test:run`
   - `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
   - `cargo test --manifest-path src-tauri/Cargo.toml`
   Stop immediately if any check fails; do not commit or push a failing change.
8. In the main repository, run:
   - `git add .`
   - `git restore --staged .env.development 2>/dev/null || true` (only skip committing `.env.development`; keep the local edits)
   - Commit with the multi-line message
   - `git push`
9. After completion, report the commit hash, commit message, current branch, and push result.

## Rules

- If there are no changes, stop and explain that no commit is needed.
- If commitlint or a git hook fails, fix the message or the underlying issue and retry; do not bypass validation.
- Do not use interactive git commands.
- Only push to the remote when the user has explicitly asked for a push.
- `.env.development` changes must be removed from the staging area before commit; do not implement the exclusion by modifying `.gitignore`.
