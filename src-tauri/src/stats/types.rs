// 统计聚合器内部用的中间类型（不导出到前端）。
//
// 前端可见的形状放在 crate::types 里；这里只是 aggregator / classifier 之间
// 流动的"原料"。
//
// `#[allow(dead_code)]`：foundation 阶段 aggregator 还没接进来，字段暂时只被
// classifier 测试读到一部分。等 aggregator 接入后再去掉。

#![allow(dead_code)]

use crate::types::UsageSummary;

/// Provenance of a call's dollar value. A recorded Pi bill must not be
/// presented as a catalog estimate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CostSource {
    Recorded,
    Catalog,
    Estimated,
    Unpriced,
    #[default]
    Unknown,
}

/// 一次 assistant API 调用。把 JSONL 里一条 assistant 消息抽成结构化记录。
#[derive(Clone, Debug)]
pub struct CallRecord {
    /// 这条记录代表的真实 API 调用次数。大多数 agent 一条 assistant message 就是
    /// 一次调用，因此使用 `Default` 的 1 即可；Grok 的
    /// `turn_completed` 是累计汇总，需显式填入 `modelCalls` 的增量。0 仅用于不代表
    /// 模型调用、只承载工具分析的合成记录。
    pub call_count: u64,
    /// 模型名（原始串；pricing::canonical 负责归一）。
    pub model: String,
    /// 上游 API 给的消息 id（Claude 是 `message.id` = "msg_..."）。
    /// 用于跨文件去重 —— Claude 会话 fork / continue / sub-agent 之间常常出现同一条
    /// assistant 消息被多个 JSONL 复制，按 id 跳过可避免 cost / token 翻倍。
    /// None = 不参与去重（Codex 没有等价字段，但它的会话拓扑也不会复制）。
    pub message_id: Option<String>,
    /// 这次调用的 token 用量（Codex 把所有调用合并到一行 token_count 事件里，
    /// 那种情况下我们把整段 usage 记到该 session 的最后一个 call 上）。
    pub usage: UsageSummary,
    /// 这次调用花了多少美元（pricing::cost_usd 算好后填）。
    pub cost_usd: f64,
    /// usage 有效但模型未在价格表中命中。与已知免费模型的 `$0` 区分开，前端据此
    /// 明确显示“定价缺失”，而不是给用户一个看似精确的零成本。
    pub pricing_missing: bool,
    /// 成本来自 Grok 官方旗舰价格兜底，并非第三方 endpoint 的实际账单。
    pub pricing_estimated: bool,
    pub cost_source: CostSource,
    /// 这次调用里 assistant 用了哪些工具（Bash / Edit / mcp__foo__bar / ...）。
    pub tools: Vec<String>,
    /// Bash 工具的命令首词（执行了哪些 shell 命令）。
    pub bash_commands: Vec<String>,
    /// 工具名里 `mcp__<server>__<tool>` 抽出来的 server 列表。
    pub mcp_servers: Vec<String>,
    /// codex / claude 在该调用里是否进入 plan mode（影响 classifier）。
    pub has_plan_mode: bool,
    /// 是否调用了 Agent / Task spawn 类工具（影响 classifier）。
    pub has_agent_spawn: bool,
}

impl Default for CallRecord {
    fn default() -> Self {
        Self {
            call_count: 1,
            model: String::new(),
            message_id: None,
            usage: UsageSummary::default(),
            cost_usd: 0.0,
            pricing_missing: false,
            pricing_estimated: false,
            cost_source: CostSource::Unknown,
            tools: Vec::new(),
            bash_commands: Vec::new(),
            mcp_servers: Vec::new(),
            has_plan_mode: false,
            has_agent_spawn: false,
        }
    }
}

/// 一个 "Turn" = 一条用户消息 + 后续的 N 次 assistant 调用。
/// classifier 按 turn 工作；aggregator 按 call 工作。
#[derive(Clone, Default, Debug)]
pub struct Turn {
    /// 这一轮用户发的纯文本（多个 text block 拼接，去掉 <command-*> 等包装）。
    pub user_message: String,
    /// 该 turn 所在项目的展示路径（cwd / displayPath）。
    pub project_path: String,
    /// 会话 id（用于 Top Sessions 排行）。
    pub session_id: String,
    /// 该 turn 包含的 assistant 调用。一次性 user→assistant 没有走多步时只有 1 个。
    pub calls: Vec<CallRecord>,
    /// 该 turn 第一条消息的时间（毫秒 unix 时间戳）。0 表示未知 —— aggregator 时
    /// 会退到 session 文件的 mtime。
    pub timestamp_ms: i64,
}

impl Turn {
    /// 整个 turn 的 token 合计（多个 call 时累加）。
    pub fn total_usage(&self) -> UsageSummary {
        let mut u = UsageSummary::default();
        for c in &self.calls {
            u.add_assign(&c.usage);
        }
        u.finalize()
    }

    /// 整个 turn 的美元成本（多个 call 时累加）。
    pub fn total_cost(&self) -> f64 {
        self.calls.iter().map(|c| c.cost_usd).sum()
    }
}
