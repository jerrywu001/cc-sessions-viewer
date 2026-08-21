// 实时 tail：监听打开会话所在 JSONL 文件的写入事件。
//
// 设计：
//   - 单订阅模型 —— 同一时刻只追一个文件（当前 ChatView 打开的那条）。
//     watch_session(agent, path) 替换上一个 watcher；unwatch_session() 清空。
//   - 触发：notify 派来 Modify / Create 事件后，debounce 一小段（避免 IDE / agent
//     频繁追加 1 行就 emit 一次），再走一次"整文件 read_session"，把新增的 Msg
//     切片 emit 给前端。
//   - 整文件 re-parse 的代价：Claude 的解析器有跨行状态（queued user 消息缓冲、
//     工具结果配对等），增量解析需要重写解析器；MVP 选择"整文件再读一次 +
//     基于 Msg 数量取尾巴"，简单、可读、足够快（实测十几 MB 会话 < 50 ms）。
//   - 文件截断 / 删除：emit `session:reset`（前端整文件重拉）或 `session:gone`。
//
// 前端事件契约：
//   session:append   { path, messages: Msg[] }    新增的尾段
//   session:reset    { path }                      文件被截断或替换 → 整文件重拉
//   session:gone     { path }                      文件不再存在
//
// 这一层不缓存 mtime —— 文件系统事件本身就是触发源，不需要轮询。
//
// 注意：这里不能直接盯单个 JSONL 文件。很多 CLI / 编辑器会用“先写临时文件，再 rename
// 覆盖”的原子替换模式落盘；如果只 watch 旧文件 inode，替换后 watcher 会失联，后续再有
// 新内容也收不到。这里统一 watch 父目录，每次事件 debounce 后回头检查目标文件当前状态，
// 这样 append / truncate / replace / recreate 都能兜住。

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::agents;
use crate::types::Msg;

/// 当前活跃 watcher 的内部状态。Drop 后 notify 回调会自然停。
struct WatchState {
    /// 让 watcher 活着 —— drop 后回调停。
    _watcher: RecommendedWatcher,
    /// 当前打开的目标文件。
    #[allow(dead_code)]
    path: PathBuf,
    /// notify 实际监听的目录（各目标文件的父目录）。
    #[allow(dead_code)]
    watch_roots: Vec<PathBuf>,
    #[allow(dead_code)]
    agent: String,
}

/// 单 watcher 槽：同一时刻只追一个文件，新订阅会替换旧 watcher。
static STATE: OnceLock<Mutex<Option<WatchState>>> = OnceLock::new();

/// 每个文件路径独立维护"上次 emit 的 Msg 数量"。即使 watch 被换了又换回来，
/// 仍能用这个 cache 接上上次的进度，避免误把整段当 append。
/// key = 绝对路径串；value = last_msg_count
static LAST_COUNT: OnceLock<Mutex<std::collections::HashMap<String, usize>>> = OnceLock::new();

/// 真正的 debounce：每次文件事件都 bump 一次序号并延后处理；只有睡眠结束后序号仍然
/// 是最新的那次事件才会触发整文件重读。这样既能合并 burst 写入，又不会把"唯一的一次"
/// 事件直接丢掉。
static DEBOUNCE_SEQ: OnceLock<Mutex<std::collections::HashMap<String, u64>>> = OnceLock::new();

fn state() -> &'static Mutex<Option<WatchState>> {
    STATE.get_or_init(|| Mutex::new(None))
}

fn last_count_map() -> &'static Mutex<std::collections::HashMap<String, usize>> {
    LAST_COUNT.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

fn debounce_seq_map() -> &'static Mutex<std::collections::HashMap<String, u64>> {
    DEBOUNCE_SEQ.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// 每个路径上次「整文件重解析」时的廉价指纹 (mtime, 字节数)。
/// 因为我们 watch 的是父目录（见文件头注释），同目录里**别的**会话文件被追加也会派事件；
/// 若每次都对目标大文件（可达数十 MB）做全量 read_session，就会被无关写入反复全量重读、CPU 打满。
/// 处理前先比指纹：目标文件没变就直接跳过昂贵的重解析。真正的追加会改 mtime/size,照常被捕获。
type FileFingerprint = Vec<(PathBuf, std::time::SystemTime, u64)>;
static LAST_STAT: OnceLock<Mutex<std::collections::HashMap<String, FileFingerprint>>> =
    OnceLock::new();

fn last_stat_map() -> &'static Mutex<std::collections::HashMap<String, FileFingerprint>> {
    LAST_STAT.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

/// 目标文件的廉价指纹：(修改时间, 字节数)。取不到（文件不在 / 无权限）返回 None,
/// 此时调用方退回到「照常重读」，不因拿不到指纹而漏掉更新。
fn file_fingerprint(path: &str) -> Option<(std::time::SystemTime, u64)> {
    let md = std::fs::metadata(path).ok()?;
    Some((md.modified().ok()?, md.len()))
}

fn files_fingerprint(paths: &[PathBuf]) -> Option<FileFingerprint> {
    let mut fingerprint = Vec::with_capacity(paths.len());
    for path in paths {
        let (modified, size) = file_fingerprint(&path.to_string_lossy())?;
        fingerprint.push((path.clone(), modified, size));
    }
    Some(fingerprint)
}

fn watch_root_for(path: &Path) -> Result<PathBuf, String> {
    path.parent().map(Path::to_path_buf).ok_or_else(|| {
        format!(
            "Cannot determine parent directory: {}",
            path.to_string_lossy()
        )
    })
}

/// debounce 窗口：notify 一次写入可能拆成多条事件，攒一拨再 emit。
/// 200ms 平衡：人类感知接近实时（<300ms 觉得是即时），又能压平 IDE / agent 的多次
/// 小写入。
const DEBOUNCE_MS: u64 = 200;
/// 文件系统事件在某些场景下会漏（例如 AppKit / CLI 写入模式差异）；轮询兜底能确保
/// 正在跑的会话最终还是会被补进来。频率保持低一些，避免空转。
const POLL_MS: u64 = 1500;

#[derive(Serialize, Clone)]
struct AppendPayload {
    path: String,
    messages: Vec<Msg>,
}

#[derive(Serialize, Clone)]
struct PathPayload {
    path: String,
}

/// 订阅一条会话的 file watch。再次调用会替换上一个 watcher（旧 watcher 自动 drop）。
/// 不存在的路径返回错误；前端可以选择降级到不 tail。
pub fn watch_session(app: AppHandle, agent: String, path: String) -> Result<(), String> {
    let src = agents::source(&agent)?;
    // 实际盯的磁盘文件由 agent 决定：文件型 = 会话文件自身；agy = transcript_full 优先；
    // opencode（虚拟路径）= 库的 -wal 文件。notify 挂在目标文件的父目录上（原子替换兜底）。
    let targets = src.watch_targets(&path);
    if targets.is_empty() {
        return Err(format!("No watchable file for: {path}"));
    }
    if let Some(missing) = targets.iter().find(|target| !target.exists()) {
        return Err(format!("File does not exist: {}", missing.display()));
    }
    let mut watch_roots = Vec::new();
    for target in &targets {
        let root = watch_root_for(target)?;
        if !watch_roots.contains(&root) {
            watch_roots.push(root);
        }
    }
    let p = PathBuf::from(&path);

    // 先把 baseline 写好，避免 watcher 起来后回调先到 process_change 时拿不到 count。
    let initial = src.read_session(&path).unwrap_or_default();
    {
        let mut m = last_count_map().lock().map_err(|e| e.to_string())?;
        m.insert(path.clone(), initial.len());
    }
    // 记下初始指纹（使用对应目标真实落盘文件的指纹）,后续无关目录事件才能被廉价短路掉
    if let Some(fp) = files_fingerprint(&targets) {
        if let Ok(mut m) = last_stat_map().lock() {
            m.insert(path.clone(), fp);
        }
    }

    let app_handle = app.clone();
    let agent_for_cb = agent.clone();
    let path_for_cb = path.clone();
    let mut watcher: RecommendedWatcher =
        notify::recommended_watcher(move |res: notify::Result<Event>| {
            let Ok(ev) = res else { return };
            if !matches!(
                ev.kind,
                EventKind::Modify(_) | EventKind::Create(_) | EventKind::Remove(_)
            ) {
                return;
            }
            let seq = {
                let mut m = match debounce_seq_map().lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                let next = m.get(&path_for_cb).copied().unwrap_or(0) + 1;
                m.insert(path_for_cb.clone(), next);
                next
            };
            let app_for_job = app_handle.clone();
            let agent_for_job = agent_for_cb.clone();
            let path_for_job = path_for_cb.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(DEBOUNCE_MS));
                let latest = debounce_seq_map()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&path_for_job).copied());
                if latest != Some(seq) {
                    return;
                }
                process_change(&app_for_job, &agent_for_job, &path_for_job);
            });
        })
        .map_err(|e| format!("notify init failed: {e}"))?;

    for watch_root in &watch_roots {
        watcher
            .watch(watch_root, RecursiveMode::NonRecursive)
            .map_err(|e| format!("watch failed: {e}"))?;
    }

    // notify 事件偶发漏报时，轮询兜底仍能把新消息补进来。process_change 内部会按
    // active watcher 校验 path/agent，并且只在 Msg 数增长时 emit 尾段，所以这里
    // 安全地定时调用即可。
    {
        let app_for_poll = app.clone();
        let agent_for_poll = agent.clone();
        let path_for_poll = path.clone();
        std::thread::spawn(move || loop {
            std::thread::sleep(Duration::from_millis(POLL_MS));
            let should_continue = {
                let slot = match state().lock() {
                    Ok(g) => g,
                    Err(_) => return,
                };
                matches!(
                    slot.as_ref(),
                    Some(active)
                        if active.agent == agent_for_poll
                            && active.path == Path::new(&path_for_poll)
                )
            };
            if !should_continue {
                return;
            }
            process_change(&app_for_poll, &agent_for_poll, &path_for_poll);
        });
    }

    // 替换上一个 watcher（如果有）—— 旧 RecommendedWatcher 随 WatchState drop。
    {
        let mut slot = state().lock().map_err(|e| e.to_string())?;
        *slot = Some(WatchState {
            _watcher: watcher,
            path: p,
            watch_roots,
            agent,
        });
    }
    Ok(())
}

/// 停止当前 watcher；没有活跃 watcher 时为空操作。前端 unmount / 切会话时调用。
pub fn unwatch_session() -> Result<(), String> {
    let mut slot = state().lock().map_err(|e| e.to_string())?;
    *slot = None;
    Ok(())
}

/// 单次文件变更处理：整文件重解析 → 跟上次 emit 的数量比 → emit 尾段或 reset。
fn process_change(app: &AppHandle, agent: &str, path: &str) {
    // 旧 watcher / 已切走的会话不再处理，避免延迟任务把过期 append 打到前端。
    {
        let slot = match state().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let Some(active) = slot.as_ref() else {
            return;
        };
        if active.agent != agent || active.path != Path::new(path) {
            return;
        }
    }

    let src = match agents::source(agent) {
        Ok(s) => s,
        Err(_) => return,
    };
    let targets = src.watch_targets(path);
    if targets.is_empty() {
        return;
    }

    if targets.iter().any(|target| !target.exists()) {
        let _ = app.emit(
            "session:gone",
            PathPayload {
                path: path.to_string(),
            },
        );
        if let Ok(mut m) = last_count_map().lock() {
            m.remove(path);
        }
        if let Ok(mut m) = debounce_seq_map().lock() {
            m.remove(path);
        }
        if let Ok(mut m) = last_stat_map().lock() {
            m.remove(path);
        }
        return;
    }

    // 廉价短路：目标文件指纹（mtime+size）与上次处理时相同 → 这次事件是同目录里**别的**文件在写,
    // 直接返回,别对大文件做全量 read_session。真有追加会改指纹,走到下面重读。
    let cur_fp = files_fingerprint(&targets);
    if let Some(ref fp) = cur_fp {
        let unchanged = last_stat_map()
            .lock()
            .ok()
            .and_then(|m| m.get(path).cloned())
            == Some(fp.clone());
        if unchanged {
            return;
        }
    }

    let msgs = match src.read_session(path) {
        Ok(m) => m,
        Err(_) => return,
    };
    // 读成功即刷新指纹（无论 Msg 数是否变化）—— 否则同一次 mtime 变更会被每个后续事件反复重读。
    if let Some(fp) = cur_fp {
        if let Ok(mut m) = last_stat_map().lock() {
            m.insert(path.to_string(), fp);
        }
    }

    let prev_count = {
        let m = match last_count_map().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        m.get(path).copied().unwrap_or(0)
    };

    if msgs.len() < prev_count {
        // 文件被截断 / 替换 → 让前端整段重拉。
        let _ = app.emit(
            "session:reset",
            PathPayload {
                path: path.to_string(),
            },
        );
        let mut m = match last_count_map().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        m.insert(path.to_string(), msgs.len());
        return;
    }

    if msgs.len() > prev_count {
        // 真有新增 —— 切尾 emit。
        let tail = msgs[prev_count..].to_vec();
        let _ = app.emit(
            "session:append",
            AppendPayload {
                path: path.to_string(),
                messages: tail,
            },
        );
        let mut m = match last_count_map().lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        m.insert(path.to_string(), msgs.len());
    }
}

pub fn check_watched_session(app: AppHandle) -> Result<(), String> {
    let active_info = {
        let slot = state().lock().map_err(|e| e.to_string())?;
        slot.as_ref().map(|active| {
            (
                active.agent.clone(),
                active.path.to_string_lossy().to_string(),
            )
        })
    };
    if let Some((agent, path)) = active_info {
        process_change(&app, &agent, &path);
    }
    Ok(())
}

/// 测试用：当前是否有活跃 watch。
#[cfg(test)]
pub fn is_watching() -> bool {
    state().lock().map(|g| g.is_some()).unwrap_or(false)
}

/// 测试用：当前 watch 的路径（如果有）。
#[cfg(test)]
pub fn current_path() -> Option<String> {
    state()
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.path.to_string_lossy().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 没起过 watcher 时 is_watching 必须是 false；unwatch 永远是 Ok。
    /// 注意：unit test 共用进程，OnceLock 状态跨测试持续，所以这条要先 unwatch 一次清场。
    #[test]
    fn unwatch_is_idempotent_and_state_starts_empty() {
        let _ = unwatch_session();
        assert!(!is_watching());
        assert!(current_path().is_none());
        // 再次 unwatch 仍 Ok，不会 panic
        assert!(unwatch_session().is_ok());
    }

    /// last_count_map 的 entry 是按 path 隔离的；不同 path 互不污染。
    /// 这条直接走内部 map，避开 notify watcher（需要真实文件 + AppHandle）。
    #[test]
    fn last_count_map_is_keyed_per_path() {
        let m = last_count_map();
        {
            let mut g = m.lock().unwrap();
            g.insert("/tmp/a.jsonl".into(), 3);
            g.insert("/tmp/b.jsonl".into(), 7);
        }
        let g = m.lock().unwrap();
        assert_eq!(g.get("/tmp/a.jsonl").copied(), Some(3));
        assert_eq!(g.get("/tmp/b.jsonl").copied(), Some(7));
    }

    /// debounce 序号同样按 path 隔离；新事件只覆盖自己的路径。
    #[test]
    fn debounce_seq_is_keyed_per_path() {
        let m = debounce_seq_map();
        {
            let mut g = m.lock().unwrap();
            g.insert("/tmp/a.jsonl".into(), 1);
            g.insert("/tmp/b.jsonl".into(), 4);
        }
        let g = m.lock().unwrap();
        assert_eq!(g.get("/tmp/a.jsonl").copied(), Some(1));
        assert_eq!(g.get("/tmp/b.jsonl").copied(), Some(4));
    }

    /// 原子替换场景下必须 watch 父目录而不是目标文件本身。
    #[test]
    fn watch_root_uses_parent_directory() {
        let p = PathBuf::from("/tmp/demo/rollout.jsonl");
        let root = watch_root_for(&p).unwrap();
        assert_eq!(root, PathBuf::from("/tmp/demo"));
    }
}
