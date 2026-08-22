# Git Commit & Push Command

Automatically commit and push the current changes with a well-structured commit message.

## Instructions

Your task is to analyze the current git changes and commit & push them with a properly formatted commit message.

### Step 1: Pull latest code
Run `!git pull` to fetch the latest changes from the remote.

**Conflict detection**:
- If the pull succeeds with no conflicts, continue to the next step.
- If a merge conflict occurs (output contains "CONFLICT" or "Merge conflict"), **immediately stop all subsequent actions** and notify the user:
  ```
  ⚠️  Merge conflict detected. Auto commit & push is not possible.
  Please resolve the conflicts manually and re-run this command.

  Conflicting files: [list the conflicting files]
  ```
- If the pull fails for any other reason, also stop and inform the user.

### Step 2: Analyze changes
Run the following commands in parallel:
- `!git status` — show all changes and untracked files
- `!git diff HEAD` — show all changes (staged and unstaged)

### Step 3: Generate the commit message

Create a commit message based on the changes, following these rules:

**Commit type rules** (from commitlint.config.cjs):
- `Feat` — new feature or capability
- `Fix` — bug fix
- `Update` — update to existing functionality
- `Refactor` — code refactoring
- `Style` — formatting / code style changes
- `Doc` — documentation changes
- `Test` — test-related changes
- `Framework` — framework / build configuration changes
- `Revert` — revert a previous change

**Message format**:
```
Type: brief summary of the change

- Change detail 1
- Change detail 2
- Change detail 3
...
```

**Requirements**:
1. Pick the most appropriate type from the list above.
2. First line: a short summary (what changed overall).
3. After a blank line: a list of the important changes.
4. One detail per line, prefixed with `-`.
5. Be specific about what was added / modified / fixed.
6. The commit message **must be written in English**.
7. **Do not add** auto-generated trailers such as "Generated with [Claude Code]" or "Co-Authored-By: Claude".

### Step 4: Quality gate (must pass before committing)

Run the following checks **in parallel**:

1. **Frontend type-check**: `!npx vue-tsc --noEmit`
2. **Frontend tests**: `!npm run test:run`
3. **Rust clippy**: `!cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`
4. **Rust tests**: `!cargo test --manifest-path src-tauri/Cargo.toml`

If **any** check fails, **stop immediately** and notify the user:
```
❌ Quality gate failed. Cannot commit.

[show the failing check name and key error lines]
```
Do NOT proceed to commit or push — the user must fix the issues first.

### Step 5: Commit and push

1. Stage all changes: `!git add .`
2. Explicitly unstage `.env.development` (keep the local edits, just do not commit the file):
   - `!git restore --staged .env.development 2>/dev/null || true`
3. Create the commit with the generated message (using HEREDOC). If the commit step errors out, stop immediately and notify the user.
4. Push to remote: `!git push`

### Step 6: Confirmation

After the push, report:
- Commit hash and message
- Branch name
- Push status

### Important notes

- If a conflict is detected during pull, abort immediately and notify the user.
- If there are no changes, do not push.
- If commitlint validation fails, adjust the message format and retry.
- Do not ask for confirmation — analyze and execute directly.
- `.env.development` changes must be excluded from the commit (only skip the commit; do not modify `.gitignore`).
