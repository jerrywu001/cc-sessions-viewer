// 托盘弹窗快速统计：一次扫描同时产出 today / 7d / month 三个时间窗口的 per-agent 汇总。
//
// 设计：复用 SessionSource::read_turns + pricing，但不经 Aggregator（那个太重，
// 带排行 / 分类 / 图表等我们不需要的东西）。直接按 turn 累加 token + cost。
// 三个时间窗口在单次遍历里同时判定，避免扫三遍。
//
// 性能：跟全局统计走同样的文件列表，但跳过了 activity 分类 / by_model / by_tool /
// daily timeline 等高开销维度。大约比 stream::run_worker 快 2–3×。

use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};

use chrono::{Datelike, Duration as CDuration, Local, TimeZone};

use crate::agents;
use crate::types::{TrayAgentSummary, TrayStats};

struct Boundaries {
    today_ms: u64,
    week_ms: u64,
    month_ms: u64,
}

const TRAY_AGENT_NAMES: [&str; 6] = ["claude", "codex", "grok", "kimicode", "opencode", "pi"];
static ENABLED_TRAY_AGENTS: OnceLock<RwLock<HashSet<String>>> = OnceLock::new();

fn enabled_tray_agents() -> &'static RwLock<HashSet<String>> {
    ENABLED_TRAY_AGENTS
        .get_or_init(|| RwLock::new(TRAY_AGENT_NAMES.into_iter().map(str::to_owned).collect()))
}

/// Sync the user-facing agent visibility setting into the native tray worker.
/// Grok/Claude/Codex/Kimi/opencode are supported by tray stats; agy is intentionally
/// ignored because it has no usage statistics source.
pub fn set_enabled_agents(agents: &[String]) {
    let allowed: HashSet<&str> = TRAY_AGENT_NAMES.into_iter().collect();
    let next: HashSet<String> = agents
        .iter()
        .map(String::as_str)
        .filter(|agent| allowed.contains(agent))
        .map(str::to_owned)
        .collect();
    if let Ok(mut current) = enabled_tray_agents().write() {
        *current = next;
    }
}

fn is_enabled(agent: &str) -> bool {
    enabled_tray_agents()
        .read()
        .map(|agents| agents.contains(agent))
        .unwrap_or(true)
}

fn compute_boundaries() -> Result<Boundaries, String> {
    let now = Local::now();
    let midnight = Local
        .with_ymd_and_hms(now.year(), now.month(), now.day(), 0, 0, 0)
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
    // 30d = 过去 30 个日历日（含今天），和 Statistics 页面的 days30 口径一致
    Ok(Boundaries {
        today_ms: to_ms(midnight),
        week_ms: to_ms(midnight - CDuration::days(6)),
        month_ms: to_ms(midnight - CDuration::days(29)),
    })
}

struct AgentAcc {
    today_tokens: u64,
    today_cost: f64,
    today_unpriced_calls: u64,
    today_estimated_calls: u64,
    week_tokens: u64,
    week_cost: f64,
    week_unpriced_calls: u64,
    week_estimated_calls: u64,
    month_tokens: u64,
    month_cost: f64,
    month_unpriced_calls: u64,
    month_estimated_calls: u64,
    session_count: usize,
    seen_ids: HashSet<String>,
}

fn append_agent_summary(result: &mut TrayStats, agent_name: &str, acc: AgentAcc) {
    // Visibility is controlled by the Settings agent toggles. Keep an enabled
    // agent in the tray even when it has no activity in the current 30-day
    // window, otherwise an installed but idle agent is indistinguishable
    // from a disabled one. agy is excluded by TRAY_AGENT_NAMES.
    result.total_today_tokens += acc.today_tokens;
    result.total_today_cost += acc.today_cost;
    result.total_today_unpriced_calls += acc.today_unpriced_calls;
    result.total_today_estimated_calls += acc.today_estimated_calls;
    result.total_week_tokens += acc.week_tokens;
    result.total_week_cost += acc.week_cost;
    result.total_week_unpriced_calls += acc.week_unpriced_calls;
    result.total_week_estimated_calls += acc.week_estimated_calls;
    result.total_month_tokens += acc.month_tokens;
    result.total_month_cost += acc.month_cost;
    result.total_month_unpriced_calls += acc.month_unpriced_calls;
    result.total_month_estimated_calls += acc.month_estimated_calls;
    result.agents.push(TrayAgentSummary {
        agent: agent_name.to_string(),
        today_tokens: acc.today_tokens,
        today_cost: acc.today_cost,
        today_unpriced_calls: acc.today_unpriced_calls,
        today_estimated_calls: acc.today_estimated_calls,
        week_tokens: acc.week_tokens,
        week_cost: acc.week_cost,
        week_unpriced_calls: acc.week_unpriced_calls,
        week_estimated_calls: acc.week_estimated_calls,
        month_tokens: acc.month_tokens,
        month_cost: acc.month_cost,
        month_unpriced_calls: acc.month_unpriced_calls,
        month_estimated_calls: acc.month_estimated_calls,
        session_count: acc.session_count,
    });
}

impl Default for AgentAcc {
    fn default() -> Self {
        Self {
            today_tokens: 0,
            today_cost: 0.0,
            today_unpriced_calls: 0,
            today_estimated_calls: 0,
            week_tokens: 0,
            week_cost: 0.0,
            week_unpriced_calls: 0,
            week_estimated_calls: 0,
            month_tokens: 0,
            month_cost: 0.0,
            month_unpriced_calls: 0,
            month_estimated_calls: 0,
            session_count: 0,
            seen_ids: HashSet::new(),
        }
    }
}

pub fn quick_stats() -> Result<TrayStats, String> {
    let bounds = compute_boundaries()?;
    let earliest = bounds.month_ms.min(bounds.week_ms);

    let agent_names: Vec<&str> = TRAY_AGENT_NAMES
        .into_iter()
        .filter(|agent| is_enabled(agent))
        .collect();
    let mut accs: HashMap<&str, AgentAcc> = HashMap::new();

    for &agent_name in &agent_names {
        let src = match agents::source(agent_name) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let projects = match src.list_projects(false, false) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let acc = accs.entry(agent_name).or_default();
        for p in &projects {
            let sessions = match src.discover_stats_sessions(&p.dir_name) {
                Ok(s) => s,
                Err(_) => continue,
            };
            for s in sessions {
                if s.modified < earliest {
                    continue;
                }
                let turns = match src.read_turns(&s.path) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let mut has_data = false;
                for turn in &turns {
                    let ts = if turn.timestamp_ms > 0 {
                        turn.timestamp_ms as u64
                    } else {
                        s.modified
                    };
                    if ts < earliest {
                        continue;
                    }
                    for call in &turn.calls {
                        if call.call_count == 0 {
                            continue;
                        }
                        if let Some(id) = &call.message_id {
                            if !acc.seen_ids.insert(id.clone()) {
                                continue;
                            }
                        }
                        has_data = true;
                        let tokens = call.usage.total;
                        let cost = call.cost_usd;
                        let call_weight = call.call_count;
                        if ts >= bounds.month_ms {
                            acc.month_tokens += tokens;
                            acc.month_cost += cost;
                            if call.pricing_missing {
                                acc.month_unpriced_calls += call_weight;
                            }
                            if call.pricing_estimated {
                                acc.month_estimated_calls += call_weight;
                            }
                        }
                        if ts >= bounds.week_ms {
                            acc.week_tokens += tokens;
                            acc.week_cost += cost;
                            if call.pricing_missing {
                                acc.week_unpriced_calls += call_weight;
                            }
                            if call.pricing_estimated {
                                acc.week_estimated_calls += call_weight;
                            }
                        }
                        if ts >= bounds.today_ms {
                            acc.today_tokens += tokens;
                            acc.today_cost += cost;
                            if call.pricing_missing {
                                acc.today_unpriced_calls += call_weight;
                            }
                            if call.pricing_estimated {
                                acc.today_estimated_calls += call_weight;
                            }
                        }
                    }
                }
                if has_data {
                    acc.session_count += 1;
                }
            }
        }
    }

    let mut result = TrayStats::default();
    for &agent_name in &agent_names {
        let acc = accs.remove(agent_name).unwrap_or_default();
        append_agent_summary(&mut result, agent_name, acc);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_visibility_accepts_supported_agents_and_excludes_agy() {
        set_enabled_agents(&[
            "claude".to_string(),
            "grok".to_string(),
            "agy".to_string(),
            "unknown".to_string(),
        ]);
        assert!(is_enabled("claude"));
        assert!(is_enabled("grok"));
        assert!(!is_enabled("kimicode"));
        assert!(!is_enabled("codex"));
        assert!(!is_enabled("opencode"));
        // agy is not a tray-stat agent even if the frontend sends it.
        assert!(!is_enabled("agy"));

        // Restore the default for other tests and for in-process test runners.
        set_enabled_agents(
            &TRAY_AGENT_NAMES
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>(),
        );
    }

    #[test]
    fn enabled_opencode_is_retained_when_the_current_window_is_empty() {
        let mut result = TrayStats::default();
        append_agent_summary(&mut result, "opencode", AgentAcc::default());

        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].agent, "opencode");
        assert_eq!(result.agents[0].session_count, 0);
    }

    #[test]
    fn enabled_kimi_is_retained_when_the_current_window_is_empty() {
        let mut result = TrayStats::default();
        append_agent_summary(&mut result, "kimicode", AgentAcc::default());

        assert_eq!(result.agents.len(), 1);
        assert_eq!(result.agents[0].agent, "kimicode");
        assert_eq!(result.agents[0].session_count, 0);
    }
}
