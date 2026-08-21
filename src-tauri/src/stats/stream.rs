// 流式统计编排：把 (scope, range, requestId) 翻译成 SessionFeed 序列，喂给
// `Aggregator`，并在合适的节奏 emit `stats://progress` / `stats://done` / `stats://error`。
//
// 关键决定：
//   - **后台线程**：start_agent_stats 是 #[tauri::command]，但不能阻塞主线程
//     等所有 JSONL 解析完；扔到 std::thread::spawn 里跑，前端 listen 接事件。
//   - **取消代际**：和 search 用同一套 AtomicU64 模式。新请求 / 显式 cancel_stats
//     都 bump 全局 gen；正在跑的 worker 每处理一个文件 check 一次，过时即 bail。
//   - **进度节奏**：每处理 16 个文件或每 250 ms（看哪个先到）emit 一次 partial
//     快照，避免太频繁的 IPC 抖动。完成时 emit 一次 final done。
//   - **数据源**：SessionSource::read_turns(path) 走的是与 read_session 不同的轻量
//     解析路径——只抽 model / usage / tools / bash / mcp，不构造 UI Block。
//   - **scope**：'all' = 所有已接入统计的 agent；单 agent 名称 = 只扫该 agent；
//     'session:<path>:<agent>' = 单个 session（per-session 统计页面用）。
//   - **range**：'today' / 'days7' / 'days30' / 'month' / 'months3' / 'months6' /
//     'custom:YYYY-MM-DD:YYYY-MM-DD'。窗口按本地日历日切，
//     和 codeburn / 各家 dashboard 一致 —— 「Today」= 本地 00:00:00 到现在，
//     不是滚动 24h（滚动会把昨晚的长会话错算到今天的总成本里，曾经把数字
//     放大 ~7x）。两层过滤：
//     1. stream 层按 session.mtime 粗筛（mtime 早于窗口下界的 session 直接跳过
//        read_turns —— 文件 mtime ≥ max(turn.timestamp_ms)，所以一定可以提前 skip）；
//     2. aggregator 层 per-turn 终判：单个 turn.timestamp_ms 落在窗口外的，整个
//        turn 在所有维度上（cost / calls / tokens / by_X / daily）都不算。
//     单走第 1 层就有这个 bug：今天被摸过的老 session（resume / 续写一条），
//     里头跨周的历史 turn 全被算进 today 总数。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Emitter};

use crate::agents;
use crate::stats::aggregate::{Aggregator, SessionFeed};
use crate::types::{StatsDone, StatsError, StatsProgress};

/// 单调代际。每次 start_agent_stats / cancel_stats 都 bump 一次；老的 worker
/// 看到 gen ≠ 自己的 request_id 立即 bail。
static STATS_GEN: AtomicU64 = AtomicU64::new(0);

/// 立刻取消任何在跑的统计 worker。bump 一次 gen 即可，worker 自己探到差异退出。
pub fn cancel() {
    STATS_GEN.fetch_add(1, Ordering::SeqCst);
}

/// 让 #[tauri::command] start_agent_stats / start_session_stats 调用。
/// 函数立即返回；后续工作在后台线程里跑，结果通过 `stats://progress` /
/// `stats://done` / `stats://error` 事件 emit 给前端。
pub fn start(app: AppHandle, scope: String, range: String, request_id: u64) {
    // 注册本次请求：让旧的 worker 立刻让位
    STATS_GEN.store(request_id, Ordering::SeqCst);

    thread::spawn(move || {
        let result = run_worker(&app, &scope, &range, request_id);
        if request_id != STATS_GEN.load(Ordering::SeqCst) {
            // 这一轮在跑过程中被取消（新请求 / cancel）—— 沉默退出，不 emit。
            return;
        }
        match result {
            Ok(final_stats) => {
                let _ = app.emit(
                    "stats://done",
                    StatsDone {
                        request_id,
                        stats: final_stats,
                    },
                );
            }
            Err(e) => {
                let _ = app.emit(
                    "stats://error",
                    StatsError {
                        request_id,
                        error: e,
                    },
                );
            }
        }
    });
}

/// 单次扫描：把所有匹配 scope/range 的 (project, session, turns) 喂进 Aggregator。
/// 进度节奏：每 16 个文件 或 每 250 ms emit 一次 partial。
fn run_worker(
    app: &AppHandle,
    scope: &str,
    range: &str,
    request_id: u64,
) -> Result<crate::types::AgentStats, String> {
    let agents_to_scan: Vec<&'static str> = match scope {
        "all" => vec!["claude", "codex", "grok", "opencode", "kimicode"],
        "claude" => vec!["claude"],
        "codex" => vec!["codex"],
        "grok" => vec!["grok"],
        "opencode" => vec!["opencode"],
        "kimicode" => vec!["kimicode"],
        other => {
            // session 模式：scope = "session:<agent>:<path>"
            if let Some(rest) = other.strip_prefix("session:") {
                return run_session_scope(app, rest, request_id);
            }
            // 受「agent 显隐设置」控制的全部口径：scope = "all:claude,codex"
            // —— 只聚合启用的 agent（隐藏的不计入 All agents 统计）。
            if let Some(list) = other.strip_prefix("all:") {
                list.split(',')
                    .filter_map(|name| match name {
                        "claude" => Some("claude"),
                        "codex" => Some("codex"),
                        "grok" => Some("grok"),
                        "opencode" => Some("opencode"),
                        "kimicode" => Some("kimicode"),
                        _ => None,
                    })
                    .collect()
            } else {
                return Err(format!("unknown stats scope: {other}"));
            }
        }
    };

    // 时间窗口（毫秒 unix）。返回 (lo, hi) —— hi 为 None 时 = 无上限。
    let (lo_ms, hi_ms) = parse_range(range)?;

    // 1) 先把所有要扫的 (agent, session_meta) 收集起来，得到 total。
    //    用 list_sessions(.., 0, usize::MAX) 拿到全量元数据 —— 这一步本身很轻
    //    （只读文件 mtime，不解析）。
    struct Pending {
        agent_name: &'static str,
        project_dir_name: String,
        project_display: String,
        session: crate::types::SessionMeta,
    }
    let mut pending: Vec<Pending> = Vec::new();

    for agent_name in &agents_to_scan {
        if request_id != STATS_GEN.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let src = match agents::source(agent_name) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let projects = match src.list_projects(false, false) {
            Ok(p) => p,
            Err(_) => continue,
        };
        for p in projects {
            if request_id != STATS_GEN.load(Ordering::SeqCst) {
                return Err("cancelled".into());
            }
            // 用 discover_stats_sessions 而不是 list_sessions —— 前者会带上
            // Claude 的 `<sessionId>/subagents/*.jsonl`（独立计费的子代理 JSONL），
            // 否则统计会少掉一整块（cost / 调用数 / 模型分布都被低估）。
            let sessions = match src.discover_stats_sessions(&p.dir_name) {
                Ok(s) => s,
                Err(_) => continue,
            };
            for s in sessions {
                if !in_window(s.modified, lo_ms, None) {
                    continue;
                }
                pending.push(Pending {
                    agent_name,
                    project_dir_name: p.dir_name.clone(),
                    project_display: p.display_path.clone(),
                    session: s,
                });
            }
        }
    }
    let total = pending.len();

    // 带 range 的聚合器：stream 这层按 session.mtime 做了粗筛（mtime < lo 的根本
    // 不 read_turns），aggregator 这层再按 turn.timestamp_ms 做终判 —— 没有这一步，
    // 今天被摸过的老 session 会把跨周的历史 turn 全算进 "Today" 总数。
    let mut agg = Aggregator::new_with_range(lo_ms, hi_ms);
    emit_progress(app, request_id, 0, total, &agg, scope);

    let mut processed: usize = 0;
    let mut last_emit = Instant::now();
    const EMIT_EVERY_N: usize = 16;
    const EMIT_EVERY: Duration = Duration::from_millis(250);

    for pp in pending {
        if request_id != STATS_GEN.load(Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let src = match agents::source(pp.agent_name) {
            Ok(s) => s,
            Err(_) => {
                processed += 1;
                continue;
            }
        };
        // 单个文件坏掉不要整盘挂；空 turns 走聚合器，session_count 仍递增。
        let turns = src.read_turns(&pp.session.path).unwrap_or_default();
        let feed = SessionFeed {
            agent: pp.agent_name,
            project_dir_name: &pp.project_dir_name,
            project_display: &pp.project_display,
            session_id: &pp.session.id,
            path: &pp.session.path,
            title: &pp.session.title,
            last_modified: pp.session.modified,
            message_count: pp.session.message_count,
            turns: &turns,
        };
        agg.feed_session(&feed);
        processed += 1;

        if processed.is_multiple_of(EMIT_EVERY_N) || last_emit.elapsed() >= EMIT_EVERY {
            emit_progress(app, request_id, processed, total, &agg, scope);
            last_emit = Instant::now();
        }
    }
    Ok(agg.snapshot(scope))
}

/// session-scope：只扫一个文件，多次进度对一个文件意义不大，直接一次性 done。
fn run_session_scope(
    app: &AppHandle,
    rest: &str,
    request_id: u64,
) -> Result<crate::types::AgentStats, String> {
    // 拼接形式："<agent>:<path>"；agent 不含 ':' 所以 splitn 2 足够。
    let mut it = rest.splitn(2, ':');
    let agent_name = it.next().ok_or_else(|| "missing agent".to_string())?;
    let path = it.next().ok_or_else(|| "missing path".to_string())?;
    let src = agents::source(agent_name)?;
    // 反查 session meta —— 用 list_sessions 找到这个 path（昂贵但一次性）
    // 更高效的做法是 read_turns + scan path 自身的 file_name，但需要先有 project；
    // 这里追求实现简单。
    let projects = src.list_projects(false, false).unwrap_or_default();
    let mut meta: Option<crate::types::SessionMeta> = None;
    let mut project_display = String::new();
    let mut project_dir = String::new();
    'outer: for p in projects {
        if let Ok(page) = src.list_sessions(&p.dir_name, 0, usize::MAX, false, false) {
            for s in page.sessions {
                if s.path == path {
                    project_display = p.display_path.clone();
                    project_dir = p.dir_name.clone();
                    meta = Some(s);
                    break 'outer;
                }
            }
        }
    }
    let meta = meta.ok_or_else(|| format!("session not found: {path}"))?;
    let scope_label = format!("session:{agent_name}");

    // 同伴文件 = Claude sub-agent JSONL（其它 agent 返回空）。把它们和 parent 一起
    // 喂给同一个 Aggregator，让单会话视图跟全局 by-session 一致；共享的
    // `seen_message_ids` 自动处理跨文件 message-id 复制。
    let companions = src.discover_session_companions(path);
    let total_steps = 1 + companions.len();

    let mut agg = Aggregator::new();
    emit_progress(app, request_id, 0, total_steps, &agg, &scope_label);

    let turns = src.read_turns(path).unwrap_or_default();
    agg.feed_session(&SessionFeed {
        agent: agent_name,
        project_dir_name: &project_dir,
        project_display: &project_display,
        session_id: &meta.id,
        path: &meta.path,
        title: &meta.title,
        last_modified: meta.modified,
        message_count: meta.message_count,
        turns: &turns,
    });
    emit_progress(app, request_id, 1, total_steps, &agg, &scope_label);

    for (i, companion) in companions.iter().enumerate() {
        let c_turns = src.read_turns(&companion.path).unwrap_or_default();
        agg.feed_session(&SessionFeed {
            agent: agent_name,
            project_dir_name: &project_dir,
            project_display: &project_display,
            session_id: &companion.id,
            path: &companion.path,
            title: &companion.title,
            last_modified: companion.modified,
            message_count: companion.message_count,
            turns: &c_turns,
        });
        emit_progress(app, request_id, 2 + i, total_steps, &agg, &scope_label);
    }
    Ok(agg.snapshot(&scope_label))
}

fn emit_progress(
    app: &AppHandle,
    request_id: u64,
    processed: usize,
    total: usize,
    agg: &Aggregator,
    scope: &str,
) {
    let partial = agg.snapshot(scope);
    let _ = app.emit(
        "stats://progress",
        StatsProgress {
            request_id,
            processed,
            total,
            partial,
        },
    );
}

fn parse_range(range: &str) -> Result<(Option<u64>, Option<u64>), String> {
    use chrono::{Datelike, Duration as CDuration, Local, Months, NaiveDate, TimeZone};

    // 「Today」= 本地 00:00:00（用户所在时区）。codeburn 也这么干 ——
    // 滚动 24h 会把昨晚 23:50 的长会话错算进今天。
    let now_local = Local::now();
    let today_midnight = Local
        .with_ymd_and_hms(
            now_local.year(),
            now_local.month(),
            now_local.day(),
            0,
            0,
            0,
        )
        .single()
        .ok_or_else(|| "failed to resolve local midnight".to_string())?;
    let to_ms = |t: chrono::DateTime<Local>| -> u64 {
        let ts = t.timestamp_millis();
        if ts < 0 {
            0
        } else {
            ts as u64
        }
    };
    if let Some(rest) = range.strip_prefix("custom:") {
        let mut parts = rest.split(':');
        let start_s = parts
            .next()
            .ok_or_else(|| "missing custom range start".to_string())?;
        let end_s = parts
            .next()
            .ok_or_else(|| "missing custom range end".to_string())?;
        if parts.next().is_some() {
            return Err("invalid custom stats range".to_string());
        }
        let start_date = NaiveDate::parse_from_str(start_s, "%Y-%m-%d")
            .map_err(|_| "invalid custom range start date".to_string())?;
        let end_date = NaiveDate::parse_from_str(end_s, "%Y-%m-%d")
            .map_err(|_| "invalid custom range end date".to_string())?;
        if start_date > end_date {
            return Err("custom range start must be before end".to_string());
        }

        let start = Local
            .from_local_datetime(
                &start_date
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| "invalid custom range start time".to_string())?,
            )
            .single()
            .ok_or_else(|| "failed to resolve custom range start".to_string())?;
        let end_next_date = end_date
            .succ_opt()
            .ok_or_else(|| "invalid custom range end date".to_string())?;
        let end_start = Local
            .from_local_datetime(
                &end_date
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| "invalid custom range end time".to_string())?,
            )
            .single()
            .ok_or_else(|| "failed to resolve custom range end".to_string())?;
        let end_next = Local
            .from_local_datetime(
                &end_next_date
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| "invalid custom range end time".to_string())?,
            )
            .single()
            .ok_or_else(|| "failed to resolve custom range end".to_string())?;
        let min_start = end_start
            .checked_sub_months(Months::new(12))
            .ok_or_else(|| "failed to compute custom range limit".to_string())?;
        if start < min_start {
            return Err("custom stats range cannot exceed 1 year".to_string());
        }
        return Ok((Some(to_ms(start)), Some(to_ms(end_next) - 1)));
    }

    match range {
        "today" => Ok((Some(to_ms(today_midnight)), None)),
        // 「过去 7 天 / 30 天」= 包含今天在内、向前数 7 / 30 个完整日历日。
        // 跟 Stripe / Linear / GitHub Insights 等仪表盘的口径一致。
        // codeburn 自己写的是 `Date(y, m, day - 7)`，实际是 8 天窗口（label 是 mislabel），
        // 我们不跟它。
        "days7" => Ok((Some(to_ms(today_midnight - CDuration::days(6))), None)),
        "days30" => Ok((Some(to_ms(today_midnight - CDuration::days(29))), None)),
        // 「本月」= 本月 1 号 00:00 → 现在。**不是** 30 天滚动（月初的时候 30 天会
        // 把上个月一大半算进来；codeburn 同样按日历月切）。
        "month" => {
            let month_start = Local
                .with_ymd_and_hms(now_local.year(), now_local.month(), 1, 0, 0, 0)
                .single()
                .ok_or_else(|| "failed to resolve month start".to_string())?;
            Ok((Some(to_ms(month_start)), None))
        }
        "months3" => {
            let lo = today_midnight
                .checked_sub_months(Months::new(3))
                .ok_or_else(|| "failed to compute -3 months".to_string())?;
            Ok((Some(to_ms(lo)), None))
        }
        // 「过去 6 个月」—— 之前是 "all"（全部时间），但全盘扫描成本巨大且基本没
        // 人真的关心 1 年前的数据。改成 6 个日历月：从本地午夜起向前减 6 个月
        // （chrono::Months 处理月末越界，e.g. 8/31 - 6 个月 = 2/28）。
        "months6" => {
            let lo = today_midnight
                .checked_sub_months(Months::new(6))
                .ok_or_else(|| "failed to compute -6 months".to_string())?;
            Ok((Some(to_ms(lo)), None))
        }
        _ => Err(format!("unknown stats range: {range}")),
    }
}

fn in_window(mtime: u64, lo: Option<u64>, hi: Option<u64>) -> bool {
    if let Some(l) = lo {
        if mtime < l {
            return false;
        }
    }
    if let Some(h) = hi {
        if mtime > h {
            return false;
        }
    }
    true
}

// ============================ 锁包装 ============================
// in-progress 请求标识，给 cancel 用。Mutex 而非 Atomic：要存 String scope/range
// 不只是 u64。当前没人读它，仅记一份请求元数据备查。
pub struct InProgress {
    pub scope: String,
    pub range: String,
}
pub struct InProgressLock(Mutex<Option<InProgress>>);
impl InProgressLock {
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }
}
impl Default for InProgressLock {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_range_handles_known_values() {
        let (lo, hi) = parse_range("days7").unwrap();
        assert!(lo.is_some());
        assert!(hi.is_none());
        let (lo3, hi3) = parse_range("months3").unwrap();
        assert!(lo3.is_some(), "months3 must be bounded, not unbounded");
        assert!(hi3.is_none());
        let (lo6, hi6) = parse_range("months6").unwrap();
        assert!(lo6.is_some(), "months6 must be bounded, not unbounded");
        assert!(hi6.is_none());
        assert!(parse_range("nope").is_err());
        // 之前的 "all" key 不再认 —— 旧 localStorage 值会回退到 settings 默认。
        assert!(parse_range("all").is_err());
    }

    #[test]
    fn parse_range_custom_uses_local_full_days_and_caps_one_year() {
        use chrono::{Local, TimeZone};
        let (lo, hi) = parse_range("custom:2025-07-05:2026-07-05").unwrap();
        let expected_lo = Local
            .with_ymd_and_hms(2025, 7, 5, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let expected_hi = Local
            .with_ymd_and_hms(2026, 7, 6, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64
            - 1;
        assert_eq!(lo, Some(expected_lo));
        assert_eq!(hi, Some(expected_hi));

        assert!(parse_range("custom:2025-07-04:2026-07-05").is_err());
        assert!(parse_range("custom:2026-07-05:2026-01-05").is_err());
        assert!(parse_range("custom:bad:2026-01-05").is_err());
    }

    /// 「过去 6 个月」= 本地午夜 - 6 个日历月。之前是 "all"（全部时间），
    /// 全盘扫太重而且基本没人关心一年前的数据。这里 pin 死语义：必须有 lo，
    /// 上限敞开（= now），lo 落在 6 个月前的午夜。
    #[test]
    fn parse_range_months6_is_six_calendar_months_back_not_unbounded() {
        use chrono::{Datelike, Local, Months, TimeZone};
        let n = Local::now();
        let midnight = Local
            .with_ymd_and_hms(n.year(), n.month(), n.day(), 0, 0, 0)
            .single()
            .unwrap();
        let expected = midnight
            .checked_sub_months(Months::new(6))
            .unwrap()
            .timestamp_millis() as u64;
        let (lo, hi) = parse_range("months6").unwrap();
        assert_eq!(lo, Some(expected));
        assert!(hi.is_none());
    }

    #[test]
    fn in_window_uses_lo_correctly() {
        assert!(in_window(100, Some(50), None));
        assert!(!in_window(10, Some(50), None));
        assert!(in_window(100, None, None));
    }

    /// 「Today」必须切到本地日历日，而不是 `now - 86_400_000`（滚动 24h）。
    /// 后者会把昨天后半夜的会话错算进今天，曾经让 KPI 比 codeburn 高 ~7x。
    #[test]
    fn parse_range_today_is_local_midnight_not_rolling_24h() {
        use chrono::{Datelike, Local, TimeZone};
        let n = Local::now();
        let expected_midnight = Local
            .with_ymd_and_hms(n.year(), n.month(), n.day(), 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let (lo, hi) = parse_range("today").unwrap();
        assert_eq!(
            lo,
            Some(expected_midnight),
            "today.lo must be local midnight"
        );
        assert!(hi.is_none(), "today.hi must be open-ended (= now)");

        // 滚动 24h 的 lo 永远 ≤ 本地午夜（除非你恰好在午夜调用）；只要不重合，
        // 我们就证明实现切的是日历日，不是滚动窗口。
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let rolling = now_ms.saturating_sub(86_400_000);
        if rolling != expected_midnight {
            assert_ne!(lo, Some(rolling), "today.lo must NOT be rolling now-24h");
        }
    }

    /// days7 / days30 按本地日切：days7.lo = 本地午夜 - 6 天（含今天共 7 天）。
    /// Stripe / Linear / GitHub Insights 同口径。codeburn 自己写成 -7d / -30d
    /// 实际是 8/31 天窗口（label 与实现脱节），我们不跟它。
    #[test]
    fn parse_range_week_and_month_use_calendar_boundaries() {
        use chrono::{Datelike, Duration, Local, TimeZone};
        let n = Local::now();
        let midnight = Local
            .with_ymd_and_hms(n.year(), n.month(), n.day(), 0, 0, 0)
            .single()
            .unwrap();
        let (lo7, _) = parse_range("days7").unwrap();
        let (lo30, _) = parse_range("days30").unwrap();
        assert_eq!(
            lo7,
            Some((midnight - Duration::days(6)).timestamp_millis() as u64)
        );
        assert_eq!(
            lo30,
            Some((midnight - Duration::days(29)).timestamp_millis() as u64)
        );
    }

    /// `month` = 本月 1 号 00:00 起 —— **不是** "过去 30 天滚动"。月初的时候
    /// 滚动 30 天会把上个月一大半算进来，对账时跟 codeburn `month` 对不上。
    #[test]
    fn parse_range_month_is_first_of_current_month_not_rolling_30() {
        use chrono::{Datelike, Local, TimeZone};
        let n = Local::now();
        let expected = Local
            .with_ymd_and_hms(n.year(), n.month(), 1, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let (lo, hi) = parse_range("month").unwrap();
        assert_eq!(lo, Some(expected));
        assert!(hi.is_none());

        // month.lo 不应等于 days30.lo —— 否则就是把「本月」错算成了 30 天滚动窗口。
        // days30.lo = 今天 − 29 天，唯一会与本月 1 号天然重合的日子是「本月第 30 天」
        // （1 + 29 = 30）。于是：两者不同属正常；若相同，则今天必须正好是 30 号 ——
        // 否则就是真的退化成滚动 30 天了。这样写不依赖运行当天，不会偶发 flaky。
        let (lo30, _) = parse_range("days30").unwrap();
        if lo == lo30 {
            assert_eq!(n.day(), 30, "month.lo 仅应在本月第 30 天与 days30.lo 重合");
        }
    }
}
