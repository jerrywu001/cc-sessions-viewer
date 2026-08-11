use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};

static SPAWN_GATE: OnceLock<Mutex<bool>> = OnceLock::new();
static SESSION_LEASES: OnceLock<Mutex<HashMap<(String, String), SessionOwner>>> = OnceLock::new();

fn spawn_gate() -> &'static Mutex<bool> {
    SPAWN_GATE.get_or_init(|| Mutex::new(false))
}

fn session_leases() -> &'static Mutex<HashMap<(String, String), SessionOwner>> {
    SESSION_LEASES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 持有期间允许创建并登记一个新的运行时子进程。退出清理会先关闭闸门，
/// 因而不会出现 cleanup drain 完注册表后又插入新 child 的竞态。
pub struct SpawnPermit {
    _guard: MutexGuard<'static, bool>,
}

pub fn spawn_permit() -> Result<SpawnPermit, String> {
    let guard = spawn_gate().lock().map_err(|e| e.to_string())?;
    if *guard {
        return Err("Application is shutting down".to_string());
    }
    Ok(SpawnPermit { _guard: guard })
}

pub fn begin_shutdown() {
    let mut guard = spawn_gate().lock().unwrap_or_else(|e| e.into_inner());
    *guard = true;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionOwner {
    Gui(u64),
    Tui(u64),
}

impl SessionOwner {
    fn label(&self) -> &'static str {
        match self {
            Self::Gui(_) => "GUI chat",
            Self::Tui(_) => "in-app terminal",
        }
    }
}

/// 独占一个可写会话；Drop 时只释放自己持有的记录，避免旧 owner 误删新 owner。
pub struct SessionLease {
    key: (String, String),
    owner: SessionOwner,
}

pub fn acquire_session(
    agent: &str,
    session_id: &str,
    owner: SessionOwner,
) -> Result<SessionLease, String> {
    let key = (agent.to_string(), session_id.to_string());
    let mut leases = session_leases().lock().map_err(|e| e.to_string())?;
    if let Some(existing) = leases.get(&key) {
        return Err(format!(
            "Session {session_id} is already open in {}. Close it before resuming elsewhere.",
            existing.label()
        ));
    }
    leases.insert(key.clone(), owner.clone());
    Ok(SessionLease { key, owner })
}

pub fn ensure_session_available(agent: &str, session_id: &str) -> Result<(), String> {
    let leases = session_leases().lock().map_err(|e| e.to_string())?;
    if let Some(existing) = leases.get(&(agent.to_string(), session_id.to_string())) {
        return Err(format!(
            "Session {session_id} is already open in {}. Close it before resuming elsewhere.",
            existing.label()
        ));
    }
    Ok(())
}

impl Drop for SessionLease {
    fn drop(&mut self) {
        if let Ok(mut leases) = session_leases().lock() {
            if leases.get(&self.key) == Some(&self.owner) {
                leases.remove(&self.key);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_lease_is_exclusive_and_released_on_drop() {
        let session_id = format!("lease-test-{}", std::process::id());
        let lease = acquire_session("codex", &session_id, SessionOwner::Gui(1)).unwrap();

        let error = acquire_session("codex", &session_id, SessionOwner::Tui(2))
            .err()
            .unwrap();
        assert!(error.contains("already open in GUI chat"));
        assert!(ensure_session_available("codex", &session_id).is_err());

        drop(lease);
        assert!(ensure_session_available("codex", &session_id).is_ok());
        assert!(acquire_session("codex", &session_id, SessionOwner::Tui(2)).is_ok());
    }

    #[test]
    fn different_agents_do_not_share_a_session_lease() {
        let session_id = format!("agent-lease-test-{}", std::process::id());
        let _codex = acquire_session("codex", &session_id, SessionOwner::Gui(1)).unwrap();

        assert!(acquire_session("claude", &session_id, SessionOwner::Gui(2)).is_ok());
    }
}
