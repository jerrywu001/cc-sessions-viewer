# Pi Fixtures

All fixture payloads are synthetic and sanitized. They contain no credentials,
provider configuration, user paths, session body, screenshots, or external
tool output from a real Pi installation.

| File | Coverage |
| --- | --- |
| `v3-linear-tool-loop.jsonl` | v3 header, assistant thinking/tool calls, ID-paired tool results, images, variable details, persisted usage/cost, repeated tool calls, and `bashExecution`. |
| `branched-metadata.jsonl` | Branches, empty `session_info` name, cross-branch label, compaction formats, branch summary, and hidden custom message. |
| `aborted-v2.jsonl` | v2 legacy hook message plus aborted assistant with thinking, error message, and persisted usage. |
| `v1-linear.jsonl` | v1 linear session without persistent entry ids. |
| `invalid-tree-diagnostics.jsonl` | Bad JSON tail, duplicate id, dangling parent, self parent, cycle, unordered timestamps, and a non-assistant physical last entry. |

Pi session roots are resolved in this order: non-empty
`PI_CODING_AGENT_SESSION_DIR`, `settings.json.sessionDir` below the resolved
agent directory, then `<agent-dir>/sessions`. The agent directory itself is
non-empty `PI_CODING_AGENT_DIR` or `~/.pi/agent`. The one-shot Pi
`--session-dir` flag is intentionally not discoverable because it has no
persistent index.
