// 统计聚合器：把若干个 (session_meta, Vec<Turn>) 喂进去，吐出前端用的 AgentStats。
//
// 设计点：
//   1. 增量友好：`Aggregator::new()` 起一个空累加器，`feed_session(...)` 把一个会话的
//      turns 加进去；任意时刻 `snapshot(scope)` 可以拿到当前累加值的快照 ——
//      stream 模块每处理 N 个文件就 emit 一次 partial 进度。
//   2. 排序在 finalize() / snapshot() 里做（HashMap → 排序 Vec）；中间状态用 HashMap
//      避免在每次累加时排序 N 次。
//   3. 时间范围 **per-turn** 过滤：构造时传入 `(lo_ms, hi_ms)` 窗口，feed_session
//      逐 turn 检查 `turn.timestamp_ms`（缺失退回 session mtime），超窗的 turn 在
//      所有维度（cost / calls / tokens / by_model / activities / daily 等）上一律不算。
//      stream 层仍按 session.mtime 做粗筛 —— 只是优化（mtime 早于 lo 的 session 不必
//      `read_turns`），并不替代真正的过滤判定。
//      历史 bug：早期实现只有 stream 层的 mtime 过滤，没有 per-turn 过滤 ——
//      "Today" 视图下，今天被摸过的 session 哪怕只加了一条新消息，整段历史
//      （可能跨多周）的 cost / tokens / calls 都被算进 today 总数。
//   4. cache_hit_rate：cache_read / (input + cache_read + cache_creation)；
//      分母为 0 时返回 0。
//
// 单元测试在文件末尾 —— 全在内存构造 Turn，不碰文件系统。

use std::collections::{HashMap, HashSet};

use crate::stats::classifier;
use crate::stats::pricing;
use crate::stats::types::Turn;
use crate::types::{
    ActivityStat, AgentStats, DailyActivity, ModelStat, NamedCount, ProjectStats, SessionStat,
    UsageSummary,
};
use crate::util::yyyymmdd_local;

/// 累加状态。HashMap-based 中间槽 —— `snapshot()` 时再排序成 Vec。
#[derive(Default)]
pub struct Aggregator {
    session_count: usize,
    message_count: usize,
    call_count: u64,
    usage: UsageSummary,
    cost_usd: f64,
    unpriced_call_count: u64,
    estimated_call_count: u64,

    /// project key = dir_name；display 走 ProjectStats.display_path
    projects: HashMap<String, ProjectStats>,
    /// 日活分桶
    daily: HashMap<String, DailyActivity>,
    /// 这一批所有 sessions（用于 Top Sessions 排行）—— 已聚合好 per-session 的数。
    /// 累积顺序 = 喂入顺序；snapshot 时按 cost_usd 降序截 Top N。
    sessions: Vec<SessionStat>,
    /// 模型聚合
    models: HashMap<String, ModelStat>,
    /// 工具调用次数（tool name → count）
    tools: HashMap<String, u64>,
    /// shell 主命令调用次数
    shells: HashMap<String, u64>,
    /// MCP server 调用次数
    mcps: HashMap<String, u64>,
    /// 活动分类聚合（key 来自 classifier::TaskCategory::key()）
    activities: HashMap<&'static str, ActivityStat>,
    /// 跨文件 message-id 去重表 —— Claude 的同一条 assistant 消息会因为
    /// 会话 fork / sub-agent JSONL 被多个文件复制；按 `message.id` 跳过避免
    /// cost / token 双倍计算。codeburn 用同名机制（`seenMsgIds`）。
    seen_message_ids: HashSet<String>,
    /// Per-turn 时间窗口。`None` = 不限。turn.timestamp_ms 落在 [lo, hi] 之外时整段跳过。
    /// stream 层会先按 session mtime 粗筛，这一层是终判。
    lo_ms: Option<u64>,
    hi_ms: Option<u64>,
}

/// 单个 session 喂入聚合器时需要的元数据。aggregator 不读文件，由 stream 层传入。
pub struct SessionFeed<'a> {
    pub agent: &'a str,
    pub project_dir_name: &'a str,
    pub project_display: &'a str,
    pub session_id: &'a str,
    pub path: &'a str,
    pub title: &'a str,
    pub last_modified: u64,
    pub message_count: usize,
    pub turns: &'a [Turn],
}

impl Aggregator {
    pub fn new() -> Self {
        Self::default()
    }

    /// 带时间窗口的聚合器（"Today" / "7d" / "30d" 走这条）。
    /// `lo_ms` / `hi_ms` 是 unix 毫秒；任一为 None 表示该侧无界。
    pub fn new_with_range(lo_ms: Option<u64>, hi_ms: Option<u64>) -> Self {
        Self {
            lo_ms,
            hi_ms,
            ..Self::default()
        }
    }

    fn turn_in_window(&self, ts: u64) -> bool {
        if let Some(l) = self.lo_ms {
            if ts < l {
                return false;
            }
        }
        if let Some(h) = self.hi_ms {
            if ts > h {
                return false;
            }
        }
        true
    }

    /// 把一个 session 喂入聚合器：累积所有 turn / call 的统计。
    /// path 可以为空 —— 仅 Top Sessions 排行需要用到它打开会话。
    ///
    /// 去重语义（与 codeburn 的 `seenMsgIds` 一致）：
    ///   - 每个 call 若带 `message_id`，先查 self.seen_message_ids；命中即 *整条
    ///     call 跳过*（usage / cost / tools / shells / mcps 都不累加），未命中则
    ///     插入 set 并照常处理。
    ///   - 一整个 session 的所有 call 都被跳过时（典型场景：fork 出来的 JSONL 完全
    ///     被祖先文件覆盖），整个 session 也不计入 session_count / 项目 / 日活 /
    ///     Top Sessions —— 否则会出现「空会话」浮在排行里。codeburn 同样行为
    ///     （`if (session.apiCalls > 0)`）。
    pub fn feed_session(&mut self, feed: &SessionFeed) {
        let mut sess_call_count: u64 = 0;
        let mut sess_usage = UsageSummary::default();
        let mut sess_cost: f64 = 0.0;

        for turn in feed.turns {
            // 终判：turn 是否在窗口内。turn.timestamp_ms == 0 时退回 session mtime
            // —— 老 JSONL 可能没 timestamp，按 session 最后活跃日算。
            let ts = if turn.timestamp_ms > 0 {
                turn.timestamp_ms as u64
            } else {
                feed.last_modified
            };
            if !self.turn_in_window(ts) {
                continue; // 整个 turn 在所有维度上都不算 —— cost / calls / tokens / by_X
            }

            let mut turn_calls_kept: u64 = 0;
            let mut turn_cost: f64 = 0.0;
            let mut turn_usage = UsageSummary::default();

            for call in &turn.calls {
                if let Some(id) = &call.message_id {
                    if !self.seen_message_ids.insert(id.clone()) {
                        continue;
                    }
                }
                // Default 是 1。Grok 显式写入 modelCalls 增量，使 calls 维度按
                // 真实次数累加；0 专门留给只承载工具信息的非计费合成记录。
                // token / cost / tools 仍只处理这条汇总 record 一次。
                let call_weight = call.call_count;
                self.call_count += call_weight;
                sess_call_count += call_weight;
                turn_calls_kept += call_weight;
                if call.pricing_missing {
                    self.unpriced_call_count += call_weight;
                }
                if call.pricing_estimated {
                    self.estimated_call_count += call_weight;
                }
                turn_cost += call.cost_usd;
                turn_usage.add_assign(&call.usage);
                self.usage.add_assign(&call.usage);
                sess_usage.add_assign(&call.usage);
                self.cost_usd += call.cost_usd;
                sess_cost += call.cost_usd;

                // 模型
                if !call.model.is_empty() {
                    let entry =
                        self.models
                            .entry(call.model.clone())
                            .or_insert_with(|| ModelStat {
                                model: call.model.clone(),
                                label: pricing::short_name(&call.model),
                                ..Default::default()
                            });
                    entry.call_count += call_weight;
                    if call.pricing_missing {
                        entry.unpriced_call_count += call_weight;
                    }
                    if call.pricing_estimated {
                        entry.estimated_call_count += call_weight;
                    }
                    entry.usage.add_assign(&call.usage);
                    entry.cost_usd += call.cost_usd;
                }

                // 工具 / Shell / MCP
                for t in &call.tools {
                    *self.tools.entry(t.clone()).or_insert(0) += 1;
                }
                for s in &call.bash_commands {
                    *self.shells.entry(s.clone()).or_insert(0) += 1;
                }
                for m in &call.mcp_servers {
                    *self.mcps.entry(m.clone()).or_insert(0) += 1;
                }
            }

            // activity 分类只算一次 —— 但要 *跳过* 整个 turn 都被去重的情况，
            // 否则空 turn 会膨胀 turn_count 但又没有任何 call 入账。
            if turn_calls_kept > 0 {
                let cat = classifier::classify(turn).key();
                let act = self.activities.entry(cat).or_insert_with(|| ActivityStat {
                    key: cat.to_string(),
                    ..Default::default()
                });
                act.turn_count += 1;
                act.call_count += turn_calls_kept;
                act.cost_usd += turn_cost;

                // 日活槽 —— 按 turn 自己的时间戳分桶（codeburn 同样做法）。`ts` 已经
                // 在外层算好并通过窗口判定，这里直接复用。
                let date = yyyymmdd_local(ts);
                let d = self.daily.entry(date.clone()).or_default();
                if d.date.is_empty() {
                    d.date = date;
                }
                d.call_count += turn_calls_kept;
                d.tokens += turn_usage.finalize().total;
                d.cost_usd += turn_cost;
            }
        }

        // 整个 session 的所有 call 都是重复 —— 这个 session 实际没有"新"数据，
        // 跳过所有 session-级累加（包括 session_count、项目、日活、Top Sessions）。
        if sess_call_count == 0 {
            return;
        }

        self.session_count += 1;
        self.message_count += feed.message_count;
        sess_usage = sess_usage.finalize();

        // 项目槽
        let proj = self
            .projects
            .entry(feed.project_dir_name.to_string())
            .or_insert_with(|| ProjectStats {
                dir_name: feed.project_dir_name.to_string(),
                display_path: feed.project_display.to_string(),
                ..Default::default()
            });
        proj.session_count += 1;
        proj.message_count += feed.message_count;
        proj.call_count += sess_call_count;
        proj.usage.add_assign(&sess_usage);
        proj.cost_usd += sess_cost;
        proj.last_modified = proj.last_modified.max(feed.last_modified);

        // 日活槽（session/message-级）：cost / calls / tokens 已经在上面 per-turn
        // 分桶完了；这里只补上 session-级别的标量（一个 session 算到它最后活跃那天）。
        let date = yyyymmdd_local(feed.last_modified);
        let d = self.daily.entry(date.clone()).or_default();
        if d.date.is_empty() {
            d.date = date;
        }
        d.session_count += 1;
        d.message_count += feed.message_count;

        // Top Sessions 槽
        let title = if feed.title.is_empty() {
            feed.session_id.to_string()
        } else {
            feed.title.to_string()
        };
        self.sessions.push(SessionStat {
            agent: feed.agent.to_string(),
            session_id: feed.session_id.to_string(),
            path: feed.path.to_string(),
            project_display: feed.project_display.to_string(),
            title,
            last_modified: feed.last_modified,
            call_count: sess_call_count,
            usage: sess_usage,
            cost_usd: sess_cost,
        });
    }

    /// 当前累加状态的快照，可在流式期间多次调用。`scope` 透传到 AgentStats.scope
    /// 给前端做"全 agent / 单 agent / 单 session"区分。
    pub fn snapshot(&self, scope: &str) -> AgentStats {
        // 项目按 cost 降序
        let mut projects: Vec<ProjectStats> = self.projects.values().cloned().collect();
        projects.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        // tie-break 用 total tokens（cost 全 0 时退化为 token 排序）
        if projects.iter().all(|p| p.cost_usd == 0.0) {
            projects.sort_by_key(|p| std::cmp::Reverse(p.usage.total));
        }

        // 日活按日期升序
        let mut daily: Vec<DailyActivity> = self.daily.values().cloned().collect();
        daily.sort_by(|a, b| a.date.cmp(&b.date));

        // Top Sessions：按 cost 降序，截前 10
        let mut top_sessions: Vec<SessionStat> = self.sessions.clone();
        top_sessions.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if top_sessions.iter().all(|s| s.cost_usd == 0.0) {
            top_sessions.sort_by_key(|s| std::cmp::Reverse(s.usage.total));
        }
        top_sessions.truncate(10);

        // 模型：按 cost 降序，再 tokens 兜底
        let mut by_model: Vec<ModelStat> = self
            .models
            .values()
            .cloned()
            .map(|mut m| {
                m.usage = m.usage.finalize();
                m.cache_hit_rate = cache_hit_rate(&m.usage);
                m
            })
            .collect();
        by_model.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        if by_model.iter().all(|m| m.cost_usd == 0.0) {
            by_model.sort_by_key(|m| std::cmp::Reverse(m.call_count));
        }

        let by_tool = sort_named(&self.tools);
        let by_shell = sort_named(&self.shells);
        let by_mcp = sort_named(&self.mcps);

        let mut by_activity: Vec<ActivityStat> = self.activities.values().cloned().collect();
        by_activity.sort_by(|a, b| {
            b.cost_usd
                .partial_cmp(&a.cost_usd)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(b.turn_count.cmp(&a.turn_count))
        });

        let total_usage = self.usage.finalize();
        AgentStats {
            scope: scope.to_string(),
            session_count: self.session_count,
            message_count: self.message_count,
            call_count: self.call_count,
            days_active: daily.len(),
            usage: total_usage,
            cost_usd: self.cost_usd,
            unpriced_call_count: self.unpriced_call_count,
            estimated_call_count: self.estimated_call_count,
            cache_hit_rate: cache_hit_rate(&total_usage),
            projects,
            daily_activity: daily,
            top_sessions,
            by_model,
            by_tool,
            by_shell,
            by_mcp,
            by_activity,
        }
    }
}

fn cache_hit_rate(u: &UsageSummary) -> f64 {
    let denom = u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens;
    if denom == 0 {
        return 0.0;
    }
    u.cache_read_input_tokens as f64 / denom as f64
}

fn sort_named(map: &HashMap<String, u64>) -> Vec<NamedCount> {
    let mut v: Vec<NamedCount> = map
        .iter()
        .map(|(k, c)| NamedCount {
            name: k.clone(),
            count: *c,
        })
        .collect();
    v.sort_by(|a, b| b.count.cmp(&a.count).then(a.name.cmp(&b.name)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::types::CallRecord;
    use crate::types::UsageSummary;

    fn turn_with_one_call(user: &str, model: &str, tools: Vec<&str>, usage: UsageSummary) -> Turn {
        Turn {
            user_message: user.to_string(),
            project_path: "/work/p".to_string(),
            session_id: "sess".to_string(),
            calls: vec![CallRecord {
                model: model.to_string(),
                usage,
                cost_usd: pricing::cost_usd(model, &usage),
                tools: tools.into_iter().map(String::from).collect(),
                ..Default::default()
            }],
            timestamp_ms: 0,
        }
    }

    fn feed<'a>(turns: &'a [Turn], last_modified: u64) -> SessionFeed<'a> {
        SessionFeed {
            agent: "claude",
            project_dir_name: "p",
            project_display: "/work/p",
            session_id: "sess",
            path: "/tmp/sess.jsonl",
            title: "demo",
            last_modified,
            message_count: turns.len() * 2,
            turns,
        }
    }

    #[test]
    fn aggregates_top_level_counters() {
        pricing::seed_test_prices();
        let usage = UsageSummary {
            input_tokens: 1_000_000,
            output_tokens: 500_000,
            ..Default::default()
        }
        .finalize();
        let turns = vec![turn_with_one_call(
            "add login",
            "claude-sonnet-4-6",
            vec!["Edit"],
            usage,
        )];
        let mut agg = Aggregator::new();
        agg.feed_session(&feed(&turns, 1_700_000_000_000));
        let s = agg.snapshot("all");
        assert_eq!(s.session_count, 1);
        assert_eq!(s.call_count, 1);
        assert!(s.cost_usd > 0.0);
        assert_eq!(s.usage.total, 1_500_000);
        assert_eq!(s.days_active, 1);
    }

    #[test]
    fn call_count_weight_does_not_duplicate_usage_cost_or_tools() {
        pricing::seed_test_prices();
        let usage = UsageSummary {
            input_tokens: 1_000,
            output_tokens: 500,
            ..Default::default()
        }
        .finalize();
        let cost = pricing::cost_usd("claude-sonnet-4-6", &usage);
        let turns = vec![Turn {
            user_message: "weighted aggregate".to_string(),
            project_path: "/work/p".to_string(),
            session_id: "sess".to_string(),
            calls: vec![CallRecord {
                call_count: 4,
                model: "claude-sonnet-4-6".to_string(),
                usage,
                cost_usd: cost,
                tools: vec!["Read".to_string()],
                ..Default::default()
            }],
            timestamp_ms: 0,
        }];
        let mut agg = Aggregator::new();
        agg.feed_session(&feed(&turns, 1_700_000_000_000));
        let s = agg.snapshot("all");

        assert_eq!(s.call_count, 4);
        assert_eq!(s.projects[0].call_count, 4);
        assert_eq!(s.daily_activity[0].call_count, 4);
        assert_eq!(s.top_sessions[0].call_count, 4);
        assert_eq!(s.by_model[0].call_count, 4);
        assert_eq!(s.by_activity[0].call_count, 4);
        assert_eq!(s.usage, usage);
        assert!((s.cost_usd - cost).abs() < 1e-12);
        assert_eq!(s.by_tool[0].count, 1);
    }

    #[test]
    fn unpriced_weight_is_explicit_at_total_and_model_level() {
        let usage = UsageSummary {
            input_tokens: 100,
            output_tokens: 20,
            ..Default::default()
        }
        .finalize();
        let turns = vec![Turn {
            user_message: "custom model".to_string(),
            project_path: "/work/p".to_string(),
            session_id: "sess".to_string(),
            calls: vec![CallRecord {
                call_count: 3,
                model: "custom-grok".to_string(),
                usage,
                pricing_missing: true,
                ..Default::default()
            }],
            timestamp_ms: 0,
        }];
        let mut agg = Aggregator::new();
        agg.feed_session(&feed(&turns, 1_700_000_000_000));
        let s = agg.snapshot("grok");

        assert_eq!(s.call_count, 3);
        assert_eq!(s.unpriced_call_count, 3);
        assert_eq!(s.by_model[0].unpriced_call_count, 3);
        assert_eq!(s.cost_usd, 0.0);
    }

    #[test]
    fn estimated_weight_is_explicit_at_total_and_model_level() {
        let usage = UsageSummary {
            input_tokens: 100,
            output_tokens: 20,
            ..Default::default()
        }
        .finalize();
        let turns = vec![Turn {
            user_message: "third-party Grok model".to_string(),
            project_path: "/work/p".to_string(),
            session_id: "sess".to_string(),
            calls: vec![CallRecord {
                call_count: 2,
                model: "third-party/grok".to_string(),
                usage,
                cost_usd: 0.25,
                pricing_estimated: true,
                ..Default::default()
            }],
            timestamp_ms: 0,
        }];
        let mut agg = Aggregator::new();
        agg.feed_session(&feed(&turns, 1_700_000_000_000));
        let s = agg.snapshot("grok");

        assert_eq!(s.call_count, 2);
        assert_eq!(s.estimated_call_count, 2);
        assert_eq!(s.unpriced_call_count, 0);
        assert_eq!(s.by_model[0].estimated_call_count, 2);
        assert_eq!(s.by_model[0].unpriced_call_count, 0);
        assert!((s.cost_usd - 0.25).abs() < 1e-12);
    }

    #[test]
    fn projects_sorted_by_cost_desc() {
        let big_usage = UsageSummary {
            input_tokens: 1_000_000,
            output_tokens: 1_000_000,
            ..Default::default()
        }
        .finalize();
        let small_usage = UsageSummary {
            input_tokens: 10,
            output_tokens: 10,
            ..Default::default()
        }
        .finalize();
        let mut agg = Aggregator::new();
        let big_turns = vec![turn_with_one_call(
            "x",
            "claude-opus-4-7",
            vec![],
            big_usage,
        )];
        let small_turns = vec![turn_with_one_call(
            "y",
            "claude-haiku-4-5",
            vec![],
            small_usage,
        )];
        agg.feed_session(&SessionFeed {
            agent: "claude",
            project_dir_name: "big",
            project_display: "/work/big",
            session_id: "1",
            path: "",
            title: "big",
            last_modified: 1,
            message_count: 0,
            turns: &big_turns,
        });
        agg.feed_session(&SessionFeed {
            agent: "claude",
            project_dir_name: "small",
            project_display: "/work/small",
            session_id: "2",
            path: "",
            title: "small",
            last_modified: 2,
            message_count: 0,
            turns: &small_turns,
        });
        let s = agg.snapshot("all");
        assert_eq!(s.projects.len(), 2);
        assert_eq!(s.projects[0].dir_name, "big");
        assert_eq!(s.projects[1].dir_name, "small");
    }

    fn turn_with_call_id(user: &str, model: &str, msg_id: &str, usage: UsageSummary) -> Turn {
        Turn {
            user_message: user.to_string(),
            project_path: "/work/p".to_string(),
            session_id: "sess".to_string(),
            calls: vec![CallRecord {
                model: model.to_string(),
                message_id: Some(msg_id.to_string()),
                usage,
                cost_usd: pricing::cost_usd(model, &usage),
                ..Default::default()
            }],
            timestamp_ms: 0,
        }
    }

    #[test]
    fn daily_buckets_by_turn_timestamp_not_session_mtime() {
        // 回归：一个跨多天的 session 必须把 cost / calls / tokens 按每条 turn 自己的
        // 时间戳分桶（codeburn 同样做法），而不是全堆到 session.last_modified 那一天。
        pricing::seed_test_prices();
        let usage = UsageSummary {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        }
        .finalize();
        let mk = |user: &str, ts_ms: i64, msg_id: &str| Turn {
            user_message: user.to_string(),
            project_path: "/p".to_string(),
            session_id: "s".to_string(),
            calls: vec![CallRecord {
                model: "claude-sonnet-4-6".to_string(),
                message_id: Some(msg_id.to_string()),
                usage,
                cost_usd: pricing::cost_usd("claude-sonnet-4-6", &usage),
                ..Default::default()
            }],
            timestamp_ms: ts_ms,
        };
        // Day 1 (2024-01-01 12:00 UTC) = 1704110400000ms
        // Day 2 (2024-01-02 12:00 UTC) = 1704196800000ms
        // Day 3 (2024-01-03 12:00 UTC) = 1704283200000ms
        let turns = vec![
            mk("morning", 1_704_110_400_000, "msg_a"),
            mk("midday", 1_704_196_800_000, "msg_b"),
            mk("evening", 1_704_283_200_000, "msg_c"),
        ];
        let mut agg = Aggregator::new();
        // session mtime = day 3，旧实现会把 3 个 turn 全堆到 2024-01-03
        agg.feed_session(&feed(&turns, 1_704_283_200_000));
        let s = agg.snapshot("all");
        assert_eq!(
            s.daily_activity.len(),
            3,
            "expected 3 distinct days, got {:?}",
            s.daily_activity.iter().map(|d| &d.date).collect::<Vec<_>>()
        );
        let d1 = s
            .daily_activity
            .iter()
            .find(|d| d.date == "2024-01-01")
            .expect("day1");
        let d2 = s
            .daily_activity
            .iter()
            .find(|d| d.date == "2024-01-02")
            .expect("day2");
        let d3 = s
            .daily_activity
            .iter()
            .find(|d| d.date == "2024-01-03")
            .expect("day3");
        assert_eq!(d1.call_count, 1);
        assert_eq!(d2.call_count, 1);
        assert_eq!(d3.call_count, 1);
        assert!(d1.cost_usd > 0.0 && d2.cost_usd > 0.0 && d3.cost_usd > 0.0);
        // session_count 是 session-级标量，按 last_modified 算 → 只在 day3 出现一次
        assert_eq!(d1.session_count, 0);
        assert_eq!(d2.session_count, 0);
        assert_eq!(d3.session_count, 1);
    }

    #[test]
    fn dedup_skips_calls_with_repeated_message_id_across_sessions() {
        // 回归：fork / sub-agent JSONL 会复制同一条 assistant 消息。aggregator
        // 必须按 message_id 去重，否则 cost / token 翻倍（codeburn parity）。
        let usage = UsageSummary {
            input_tokens: 1_000,
            output_tokens: 500,
            ..Default::default()
        }
        .finalize();
        let mut agg = Aggregator::new();
        // 第一份文件
        let turns_a = vec![
            turn_with_call_id("ask", "claude-sonnet-4-6", "msg_a", usage),
            turn_with_call_id("ask2", "claude-sonnet-4-6", "msg_b", usage),
        ];
        agg.feed_session(&feed(&turns_a, 1));
        // 第二份文件复制了 msg_a + 多一条新的 msg_c
        let turns_b = vec![
            turn_with_call_id("ask", "claude-sonnet-4-6", "msg_a", usage),
            turn_with_call_id("ask3", "claude-sonnet-4-6", "msg_c", usage),
        ];
        agg.feed_session(&feed(&turns_b, 2));
        let s = agg.snapshot("all");
        // 3 条独立 call（a, b, c），不是 4 条
        assert_eq!(s.call_count, 3, "expected dedup to drop msg_a duplicate");
        assert_eq!(s.usage.input_tokens, 3_000);
        assert_eq!(s.usage.output_tokens, 1_500);
        // 2 个 session 都贡献了至少一条新 call，session_count = 2
        assert_eq!(s.session_count, 2);
    }

    #[test]
    fn dedup_drops_entire_session_when_all_calls_are_duplicates() {
        // 第二份文件完全是第一份的子集（典型 fork-without-new-messages 场景）——
        // 不应该膨胀 session_count / 项目 / Top Sessions。
        let usage = UsageSummary {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        }
        .finalize();
        let mut agg = Aggregator::new();
        let turns_a = vec![turn_with_call_id("hi", "claude-sonnet-4-6", "msg_x", usage)];
        agg.feed_session(&feed(&turns_a, 1));
        // 第二个 session 的所有 call 都是 msg_x —— 整个 session 应被丢弃
        let turns_b = vec![turn_with_call_id(
            "hi again",
            "claude-sonnet-4-6",
            "msg_x",
            usage,
        )];
        agg.feed_session(&feed(&turns_b, 2));
        let s = agg.snapshot("all");
        assert_eq!(
            s.session_count, 1,
            "duplicate-only session should not count"
        );
        assert_eq!(s.call_count, 1);
        assert_eq!(s.projects.len(), 1);
        assert_eq!(s.projects[0].session_count, 1);
        // Top Sessions 也只剩 1 条
        assert_eq!(s.top_sessions.len(), 1);
    }

    #[test]
    fn by_model_aggregates_across_sessions() {
        let mut agg = Aggregator::new();
        let usage = UsageSummary {
            input_tokens: 100,
            output_tokens: 100,
            ..Default::default()
        }
        .finalize();
        for _ in 0..3 {
            let turns = vec![turn_with_one_call("x", "claude-sonnet-4-6", vec![], usage)];
            agg.feed_session(&feed(&turns, 0));
        }
        let s = agg.snapshot("all");
        assert_eq!(s.by_model.len(), 1);
        assert_eq!(s.by_model[0].call_count, 3);
        assert!(s.by_model[0].label.starts_with("Sonnet"));
    }

    #[test]
    fn by_tool_counts_all_tool_uses() {
        let mut agg = Aggregator::new();
        let usage = UsageSummary::default();
        let turns = vec![
            turn_with_one_call("a", "claude-sonnet-4-6", vec!["Bash", "Read"], usage),
            turn_with_one_call("b", "claude-sonnet-4-6", vec!["Bash"], usage),
            turn_with_one_call("c", "claude-sonnet-4-6", vec!["Edit", "Bash"], usage),
        ];
        agg.feed_session(&feed(&turns, 0));
        let s = agg.snapshot("all");
        // Bash 出现 3 次，Edit / Read 各 1 次
        let bash = s.by_tool.iter().find(|n| n.name == "Bash").unwrap();
        assert_eq!(bash.count, 3);
        assert!(
            s.by_tool[0].name == "Bash",
            "Bash should be top: {:?}",
            s.by_tool
        );
    }

    #[test]
    fn by_activity_uses_classifier() {
        let mut agg = Aggregator::new();
        let usage = UsageSummary::default();
        let turns = vec![
            turn_with_one_call("add feature x", "claude-sonnet-4-6", vec!["Edit"], usage),
            turn_with_one_call("refactor login", "claude-sonnet-4-6", vec!["Edit"], usage),
            turn_with_one_call(
                "git push the changes",
                "claude-sonnet-4-6",
                vec!["Bash"],
                usage,
            ),
        ];
        agg.feed_session(&feed(&turns, 0));
        let s = agg.snapshot("all");
        let keys: Vec<&str> = s.by_activity.iter().map(|a| a.key.as_str()).collect();
        assert!(keys.contains(&"feature"));
        assert!(keys.contains(&"refactoring"));
        assert!(keys.contains(&"git"));
    }

    #[test]
    fn top_sessions_truncated_to_ten_and_sorted_by_cost() {
        let mut agg = Aggregator::new();
        for i in 0..15 {
            let usage = UsageSummary {
                input_tokens: (i as u64) * 1_000_000,
                output_tokens: 0,
                ..Default::default()
            }
            .finalize();
            let turns = vec![turn_with_one_call("x", "claude-sonnet-4-6", vec![], usage)];
            agg.feed_session(&SessionFeed {
                agent: "claude",
                project_dir_name: "p",
                project_display: "/work/p",
                session_id: &format!("s{i}"),
                path: "",
                title: &format!("title-{i}"),
                last_modified: 0,
                message_count: 0,
                turns: &turns,
            });
        }
        let s = agg.snapshot("all");
        assert_eq!(s.top_sessions.len(), 10);
        // 最贵的应该是最后喂入的（i=14）
        assert_eq!(s.top_sessions[0].session_id, "s14");
        assert_eq!(s.top_sessions[9].session_id, "s5");
    }

    #[test]
    fn cache_hit_rate_computed_correctly() {
        let mut agg = Aggregator::new();
        let usage = UsageSummary {
            input_tokens: 100,
            cache_read_input_tokens: 900,
            ..Default::default()
        }
        .finalize();
        let turns = vec![turn_with_one_call("x", "claude-sonnet-4-6", vec![], usage)];
        agg.feed_session(&feed(&turns, 0));
        let s = agg.snapshot("all");
        // 900 / (100 + 0 + 900) = 0.9
        assert!((s.cache_hit_rate - 0.9).abs() < 1e-9);
    }

    #[test]
    fn empty_snapshot_is_safe() {
        let agg = Aggregator::new();
        let s = agg.snapshot("all");
        assert_eq!(s.session_count, 0);
        assert!(s.projects.is_empty());
        assert!(s.daily_activity.is_empty());
        assert!(s.top_sessions.is_empty());
        assert_eq!(s.cache_hit_rate, 0.0);
    }

    #[test]
    fn range_window_filters_per_turn_not_per_session() {
        // 回归：用户截图的根因。一个 session 文件今天被摸过（mtime = today），
        // 内含 3 个 turn：day 1（在窗外）、day 2（在窗外）、day 3（窗内）。
        // 旧实现只在 stream 层按 mtime 粗筛 —— session 整段送进 aggregator，
        // 三个 turn 的 cost / tokens 全算进 "Today" 总数。
        // 修复后：构造时传 (lo, None)，aggregator 按 turn.timestamp_ms 终判，
        // 窗外的 turn 在所有维度上都不算。
        pricing::seed_test_prices();
        let usage = UsageSummary {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        }
        .finalize();
        let mk = |ts_ms: i64, msg_id: &str| Turn {
            user_message: "x".to_string(),
            project_path: "/p".to_string(),
            session_id: "s".to_string(),
            calls: vec![CallRecord {
                model: "claude-sonnet-4-6".to_string(),
                message_id: Some(msg_id.to_string()),
                usage,
                cost_usd: pricing::cost_usd("claude-sonnet-4-6", &usage),
                ..Default::default()
            }],
            timestamp_ms: ts_ms,
        };
        // Day 1/2/3 同 daily_buckets_by_turn_timestamp 那个测试的时间锚点
        let turns = vec![
            mk(1_704_110_400_000, "msg_a"), // 2024-01-01
            mk(1_704_196_800_000, "msg_b"), // 2024-01-02
            mk(1_704_283_200_000, "msg_c"), // 2024-01-03
        ];
        // 窗口：Day 3 起到无穷大（lo = day 3 00:00 UTC = 1_704_240_000_000）
        let lo = 1_704_240_000_000_u64;
        let mut agg = Aggregator::new_with_range(Some(lo), None);
        // 喂进去 —— mtime = day 3，stream 层粗筛会通过
        agg.feed_session(&feed(&turns, 1_704_283_200_000));
        let s = agg.snapshot("all");

        // 仅 day 3 那 1 个 turn 入账，不是 3 个
        assert_eq!(s.call_count, 1, "只该数 day3 的 turn，window 内");
        assert_eq!(s.session_count, 1);
        assert_eq!(s.usage.input_tokens, 100, "只统计 day3 的 input");
        assert_eq!(s.usage.output_tokens, 50);
        // daily 只有 day 3 一条
        assert_eq!(s.daily_activity.len(), 1);
        assert_eq!(s.daily_activity[0].date, "2024-01-03");
        // cost 跟 daily.day3 的 cost 应该相等 —— 顶部 KPI 必须 == daily 内窗内 cost 之和
        assert!((s.cost_usd - s.daily_activity[0].cost_usd).abs() < 1e-9);
        // By Model 也只该数 day3 的 1 个 call
        assert_eq!(s.by_model.len(), 1);
        assert_eq!(s.by_model[0].call_count, 1);
    }

    #[test]
    fn range_window_drops_session_with_zero_in_window_turns() {
        // session.mtime 落在窗内（被 stream 层放行），但所有 turn 都在窗外
        // → session_count 不应被计入，projects / top_sessions 也不应出现。
        pricing::seed_test_prices();
        let usage = UsageSummary {
            input_tokens: 100,
            output_tokens: 50,
            ..Default::default()
        }
        .finalize();
        let old_turn = Turn {
            user_message: "x".to_string(),
            project_path: "/p".to_string(),
            session_id: "s".to_string(),
            calls: vec![CallRecord {
                model: "claude-sonnet-4-6".to_string(),
                message_id: Some("only-msg".to_string()),
                usage,
                cost_usd: pricing::cost_usd("claude-sonnet-4-6", &usage),
                ..Default::default()
            }],
            timestamp_ms: 1_704_110_400_000, // 2024-01-01
        };
        let lo = 1_704_240_000_000_u64; // 2024-01-03 起
        let mut agg = Aggregator::new_with_range(Some(lo), None);
        // mtime = day 3（stream 层放行），但唯一的 turn ts 是 day 1
        agg.feed_session(&feed(std::slice::from_ref(&old_turn), 1_704_283_200_000));
        let s = agg.snapshot("all");
        assert_eq!(s.session_count, 0, "整段窗外 session 不计入");
        assert_eq!(s.call_count, 0);
        assert!(s.projects.is_empty());
        assert!(s.top_sessions.is_empty());
        assert_eq!(s.cost_usd, 0.0);
    }
}
