-- Synthetic data only. Load into a NEW temporary database, never a real OpenCode DB.
-- Table definitions taken from anomalyco/opencode@dbcd5ce9485f3338375d1fe9d92864c91e263030
-- packages/core/src/database/schema.gen.ts (session-related subset; no credentials/events).
CREATE TABLE project (
  id TEXT PRIMARY KEY, worktree TEXT NOT NULL, vcs TEXT, name TEXT,
  icon_url TEXT, icon_url_override TEXT, icon_color TEXT,
  time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_initialized INTEGER,
  time_active INTEGER DEFAULT 0 NOT NULL, time_archived INTEGER,
  sandboxes TEXT NOT NULL, commands TEXT
);
CREATE TABLE session_v2 (
  id TEXT PRIMARY KEY, project_id TEXT NOT NULL, workspace_id TEXT, parent_id TEXT,
  fork_session_id TEXT, fork_boundary TEXT, slug TEXT NOT NULL, directory TEXT NOT NULL,
  path TEXT, title TEXT, version TEXT NOT NULL, share_url TEXT,
  summary_additions INTEGER, summary_deletions INTEGER, summary_files INTEGER,
  summary_diffs TEXT, metadata TEXT,
  cost REAL DEFAULT 0 NOT NULL, tokens_input INTEGER DEFAULT 0 NOT NULL,
  tokens_output INTEGER DEFAULT 0 NOT NULL, tokens_reasoning INTEGER DEFAULT 0 NOT NULL,
  tokens_cache_read INTEGER DEFAULT 0 NOT NULL, tokens_cache_write INTEGER DEFAULT 0 NOT NULL,
  revert TEXT, permission TEXT, agent TEXT, model TEXT,
  time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, time_busy INTEGER,
  time_idle INTEGER, time_viewed INTEGER, idle_outcome TEXT, time_compacting INTEGER,
  time_archived INTEGER, time_suspended INTEGER, resume_attempts INTEGER DEFAULT 0 NOT NULL,
  FOREIGN KEY (project_id) REFERENCES project(id) ON DELETE CASCADE
);
CREATE TABLE session_message (
  id TEXT PRIMARY KEY, session_id TEXT NOT NULL, type TEXT NOT NULL, seq INTEGER NOT NULL,
  time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL, data TEXT NOT NULL,
  FOREIGN KEY (session_id) REFERENCES session_v2(id) ON DELETE CASCADE
);
CREATE TABLE session_part (
  message_id TEXT NOT NULL, session_id TEXT NOT NULL, ordinal INTEGER NOT NULL,
  type TEXT NOT NULL, call_id TEXT, time_created INTEGER NOT NULL,
  time_updated INTEGER NOT NULL, data TEXT NOT NULL,
  PRIMARY KEY (message_id, ordinal),
  FOREIGN KEY (message_id) REFERENCES session_message(id) ON DELETE CASCADE,
  FOREIGN KEY (session_id) REFERENCES session_v2(id) ON DELETE CASCADE
);
CREATE TABLE blob (
  hash TEXT PRIMARY KEY, mime TEXT NOT NULL, bytes INTEGER NOT NULL,
  width INTEGER, height INTEGER, data BLOB NOT NULL, time_created INTEGER NOT NULL
);
CREATE UNIQUE INDEX session_message_session_seq_idx ON session_message(session_id, seq);
CREATE INDEX session_message_session_type_seq_idx ON session_message(session_id, type, seq);
CREATE INDEX session_part_session_message_idx ON session_part(session_id, message_id, ordinal);
CREATE INDEX session_v2_project_idx ON session_v2(project_id);
CREATE INDEX session_v2_parent_idx ON session_v2(parent_id);

INSERT INTO project (id, worktree, time_created, time_updated, sandboxes)
VALUES ('synthetic-project', 'X:/synthetic/project', 1000, 2000, '[]');
INSERT INTO session_v2 (id, project_id, slug, directory, title, version,
  time_created, time_updated, tokens_input, tokens_output, cost)
VALUES ('ses_...', 'synthetic-project', 'synthetic-moon', 'X:/synthetic/project', NULL,
  'synthetic', 1000, 2000, 10, 5, 0.25);
-- seq, not the clock or ID, determines transcript order.
INSERT INTO session_message VALUES ('msg_z', 'ses_...', 'user', 1, 2000, 2000,
  '{"text":"Question","time":{"created":2000}}');
INSERT INTO session_message VALUES ('msg_a', 'ses_...', 'assistant', 2, 1000, 1000,
  '{"model":{"id":"synthetic-model","providerID":"synthetic"},"agent":"build","content":[{"type":"text","text":"Stale inline answer"}],"tokens":{"input":10,"output":5},"cost":0.25}');
INSERT INTO session_part VALUES ('msg_a', 'ses_...', 0, 'text', NULL, 1000, 1000,
  '{"type":"text","text":"Answer"}');
