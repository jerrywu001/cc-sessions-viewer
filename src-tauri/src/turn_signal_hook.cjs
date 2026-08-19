#!/usr/bin/env node
const fs = require('fs');
const path = require('path');

function hasPromptContent(value) {
  if (typeof value === 'string') return value.trim().length > 0;
  if (Array.isArray(value)) return value.some((item) => {
    if (typeof item === 'string') return item.trim().length > 0;
    if (!item || typeof item !== 'object') return false;
    if (item.type === 'text') return hasPromptContent(item.text || item.content || '');
    if (item.type === 'image') return true;
    return hasPromptContent(item.text || item.content || item.prompt || '');
  });
  if (value && typeof value === 'object') {
    if (value.type === 'image') return true;
    return hasPromptContent(value.text || value.content || value.prompt || '');
  }
  return false;
}

function shouldSkipStarted(data) {
  const candidates = [data.prompt, data.message, data.user_prompt, data.userPrompt];
  return candidates.some((value) => value !== undefined) && !candidates.some(hasPromptContent);
}

function agyTurnState(requestedState, data) {
  if (requestedState !== 'completed') return requestedState;
  if (data.fullyIdle !== true) return '';
  const reason = typeof data.terminationReason === 'string'
    ? data.terminationReason.toLowerCase()
    : '';
  if (hasPromptContent(data.error) || reason === 'error' || reason === 'max_steps_exceeded') {
    return 'failed';
  }
  return 'completed';
}

function writeAgyResponse(requestedState) {
  const response = requestedState === 'completed' ? { decision: 'stop' } : {};
  try {
    process.stdout.write(JSON.stringify(response));
  } catch (_) {
    // Hook output failure must not turn observability into an agent failure.
  }
}

const agent = process.argv[2];
const state = process.argv[3];
const signalPath = process.argv[4];
let input = '';
process.stdin.setEncoding('utf8');
process.stdin.on('data', chunk => { input += chunk; });
process.stdin.on('end', () => {
  try {
    if (!signalPath || !state || !['claude', 'codex', 'agy', 'grok'].includes(agent)) return;
    const data = input.trim() ? JSON.parse(input) : {};
    const transcriptPath = data.transcript_path || data.transcriptPath || '';
    const sessionId = data.sessionId || data.session_id || '';
    const cwd = data.cwd || data.workspaceRoot || '';
    const promptId = data.promptId || data.prompt_id || '';
    if (agent === 'grok' && data.subagentType) return;
    if (agent === 'grok' && state === 'completed' && data.reason && data.reason !== 'end_turn') return;
    if (!transcriptPath && !(agent === 'grok' && sessionId)) return;
    if (state === 'started' && shouldSkipStarted(data)) return;
    const turnState = agent === 'agy' ? agyTurnState(state, data) : state;
    if (!turnState) return;
    const payload = {
      agent,
      path: transcriptPath,
      state: turnState,
      source: 'hook',
      ...(promptId ? { promptId } : {}),
      ...(sessionId ? { sessionId } : {}),
      ...(cwd ? { cwd } : {}),
    };
    fs.mkdirSync(path.dirname(signalPath), { recursive: true });
    fs.appendFileSync(signalPath, JSON.stringify(payload) + '\n', 'utf8');
  } catch (_) {
    // Observability hook: never block the agent.
  } finally {
    if (agent === 'agy') writeAgyResponse(state);
  }
});
