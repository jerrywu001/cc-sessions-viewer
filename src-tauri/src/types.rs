// 前端 & 各 agent 模块共享的可序列化类型。
// 这里只放数据形状定义，所有字段都 `pub`，方便各 agent 实现直接构造。
// 字段命名规则：Rust snake_case → JS camelCase（serde 全局 rename_all）。
//
// `#[allow(dead_code)]`：流式统计模块（stats/stream.rs）尚未接入，
// `StatsProgress` / `StatsDone` / `StatsError` / `TimeRange` 等只在
// 下一批改动里被消费。允许暂时未使用，避免 clippy 报错阻塞构建。

#![allow(dead_code)]

use serde::{Deserialize, Serialize};

/// 前端发送一条 GUI chat 用户消息时附带的图片附件（粘贴 / 拖拽 / 选择）。
/// `data` 是去掉 `data:` 前缀的纯 base64；`media_type` 如 `image/png`。
/// 放在 types.rs（而非 agent_chat.rs）：`SessionSource::chat_encode_input` 要用到它，
/// 类型住在共享层，避免 trait 反向依赖驱动模块。
/// GUI chat `/` 浮层里的一条可用项 —— 命令（自定义 / 插件）或技能。`name` 是不含前导 `/`
/// 的调用 token（命令命名空间名 `git:commit` / 技能名 `animejs`），选中后按 `/<name>` 透传给
/// CLI（CLI 自己展开）。**不含 TUI 内置指令**（headless 不展开、会报「not available」）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    /// 调用 token（无前导 `/`）。
    pub name: String,
    /// 浮层展示名：命令 = `/name`；技能 = 由 name 美化的 Title Case（如 `animejs`→`Animejs`）。
    pub title: String,
    pub description: String,
    /// 分组 + 图标依据：`"command"` | `"skill"`。
    pub kind: String,
    /// 来源类别：`"user"`（→ UI 显示「Personal」）/ `"project"` / `"plugin"`。
    pub origin: String,
    /// 来源名：项目名 / 插件名（`user` 来源省略，前端回落到本地化「Personal」）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin_name: Option<String>,
    /// 命令 frontmatter 的 `argument-hint`（如 `[--wait] [--base <ref>]`）：选中命令后在输入框里
    /// 作为暗色 ghost 占位提示参数格式（对齐 Claude TUI）。技能 / 无此字段的命令省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
}

/// `agent_chat_start` 的返回：内部 chat id + 该 agent 的进程模型标识。前端据
/// `process_model`（"longLivedStdin" / "oneShotResume" / "codexAppServer"）决定切模型 /
/// effort / 权限时是要 restart-with-resume（长驻/app-server）还是改下轮 flag 即可（one-shot）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatStartInfo {
    pub chat_id: u64,
    pub process_model: String,
}

/// Claude GUI chat 需要知道当前本机配置是否挂了自定义 Anthropic 兼容端点。
/// 一旦 `has_custom_base_url=true`，前端就不应显示订阅专属的 5h/周限额，也不应暴露
/// effort 这种可能被第三方端点拒绝的参数。
#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeRuntimeInfo {
    pub has_custom_base_url: bool,
    pub alias_targets: ClaudeAliasTargets,
    /// 进会话前对 Claude 鉴权方式的**预判**（init 事件回来前的种子；init 一到以其为准）：
    /// `"none"` = 订阅/OAuth 登录（5h/周限额 + effort 生效）；`"ANTHROPIC_API_KEY"` /
    /// `"apiKeyHelper"` = API key 计费；`None` = 判不出（UI 保持保守，等 init）。
    pub api_key_source: Option<String>,
    /// settings.json 里的 `effortLevel`（用户在 CLI 选的全局 reasoning effort 默认档）。
    /// transcript 不记录 effort，CLI 在不带 `--effort` 时即用这个值 —— 故它是 GUI chat
    /// effort 选择器在用户未显式改档前应当展示的「真实生效默认」（而非假的 levels[0]）。
    pub effort_level: Option<String>,
}

#[derive(Serialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeAliasTargets {
    pub opus: Option<String>,
    pub sonnet: Option<String>,
    pub haiku: Option<String>,
    pub fable: Option<String>,
}

/// Codex 运行时信息（对标 `ClaudeRuntimeInfo`）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CodexRuntimeInfo {
    /// `true` = 用户通过第三方 API key / 自定义端点使用 Codex（config.toml 里
    /// `model_provider` 不是官方默认值）。前端据此隐藏仅官方订阅可用的模型。
    pub uses_api_key: bool,
    /// ~/.codex/config.toml 顶层 `model`。自定义 provider 下仅用于前端显示/勾选，
    /// 不代表 GUI chat 会强制下发该模型。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// ~/.codex/config.toml 顶层 `model_reasoning_effort`。保留给 UI 展示/后续扩展。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
}

/// `reclaude_info` 返回：本地 reclaude 守护进程的运行状态，供前端判断能否启用代理。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ReclaudeInfo {
    pub installed: bool,
    pub daemon_running: bool,
    pub daemon_port: Option<u16>,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatImageInput {
    pub media_type: String,
    pub data: String,
    /// Claude stdin content blocks 中图片对应的正文占位符；普通附件为空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

/// GUI chat 输入框 `@` 文件浮层的一条目录/文件项。`rel_path` 相对会话 `cwd`（统一用
/// `/` 分隔），`name` 是末段名字，`is_dir` 决定图标 + 钻取行为。`has_children` 仅对目录
/// 有意义：是否含可见子项（空目录 = false → 前端隐藏「进入」chevron / 禁用下钻）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFileEntry {
    pub rel_path: String,
    pub name: String,
    pub is_dir: bool,
    pub has_children: bool,
}

/// 流式增量（`--include-partial-messages` → `stream_event`）归一后的一帧。
/// Claude stream_event / Codex app-server delta 会产出；前端据此驱动「正在生成」气泡的打字机效果。
/// `phase`：`start`(块开始) | `delta`(追加) | `stop`(块结束)。
/// `kind`：块类型 `text` | `thinking` | `tool_use`（start 必有；delta 带上便于前端兜底建块）。
/// `text`：仅 delta —— 本次追加的文本片段。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatDelta {
    pub index: u64,
    pub phase: String,
    pub kind: Option<String>,
    pub text: Option<String>,
}

/// GUI chat 交互式工具权限请求 —— Claude headless 控制协议里 `can_use_tool` 的归一形状
/// （`--permission-prompt-tool stdio`）。CLI 在工具被门控时把它从 stdout 发来，前端弹
/// 「允许 Claude 运行 X？」对话框，用户的选择经 `agent_chat_respond_permission` 回写。
///
/// `input` / `permission_suggestions` 是任意 JSON：前者原样回传给 `updatedInput`（允许），
/// 后者是 CLI 给出的「永久允许」规则建议（`addRules`，含 `destination` 如 localSettings =
/// 截图里的「Project (local)」），勾「始终允许」时原样回传 `updatedPermissions`。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatPermissionRequest {
    /// 控制协议的关联 id —— 回写 `control_response` 时必须原样带回。
    pub request_id: String,
    /// 工具名（如 "Bash" / "Write" / "Edit"）。
    pub tool_name: String,
    /// 工具参数原文（Bash 的 `command`、文件工具的 `file_path` 等都在里面）。
    pub input: serde_json::Value,
    /// CLI 给的人类可读说明（可能为空）。
    pub description: Option<String>,
    /// 「始终允许」的规则建议（`addRules` 数组）；None / 空 = 不提供「始终允许」。
    pub permission_suggestions: Option<serde_json::Value>,
}

/// AskUserQuestion 工具的结构化提问 —— 与工具权限同走控制协议 `can_use_tool`，只是
/// `tool_name == "AskUserQuestion"`，参数里带的是 `questions` 而非工具入参。前端据此弹
/// 「选择题」卡片（单选 / 多选 / Other 自填 / 并排预览），用户的选择经
/// `agent_chat_respond_question` 回写 `control_response` 送回同一 stdin。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ChatQuestionRequest {
    /// 控制协议的关联 id —— 回写 `control_response` 时必须原样带回。
    pub request_id: String,
    /// 提问数组原文：每项 `{question, header?, multiSelect?, options:[{label, description?, preview?}]}`。
    /// 原样透传给前端（回写 decision 的 `updatedInput.questions` 要把它带回去）。
    pub questions: serde_json::Value,
    /// Codex app-server 的 requestUserInput 可能在 turn completed 后仍有效，需要前端保留弹框。
    #[serde(default)]
    pub keep_after_turn: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectInfo {
    /// 项目标识：Claude 为目录名，Codex 为 cwd 路径。
    pub dir_name: String,
    pub display_path: String,
    pub session_count: usize,
    pub last_modified: u64,
    /// 项目目录当前是否仍存在于磁盘上。
    pub exists: bool,
    /// 是否为用户手动添加的书签目录。
    #[serde(default)]
    pub bookmarked: bool,
    /// worktree 所属父项目的 dir_name（非 worktree 则为 None）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_dir_name: Option<String>,
    /// worktree 分支名（如 "test-aaa"）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_name: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub file_name: String,
    pub path: String,
    pub title: String,
    pub cwd: Option<String>,
    pub created: Option<String>,
    pub modified: u64,
    pub size: u64,
    pub message_count: usize,
    pub codex_app_list_rank: Option<usize>,
    pub codex_app_list_scanned: usize,
    pub codex_app_first_page_size: usize,
    pub codex_app_first_page_position: usize,
    pub codex_internal: bool,
    pub codex_archived: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionPage {
    /// 该项目会话总数（用于前端判断是否还有下一页）。
    pub total: usize,
    pub sessions: Vec<SessionMeta>,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub kind: String, // ctx | add | del
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    pub text: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DiffHunk {
    pub old_start: u32,
    pub new_start: u32,
    pub lines: Vec<DiffLine>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GitCommit {
    pub hash: String,
    pub author: String,
    pub date: String,
    pub message: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GitFileStatus {
    pub path: String,
    pub status: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GitRepositoryState {
    pub branch: Option<String>,
    pub branches: Vec<String>,
    pub change_count: usize,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct GitDiffFile {
    pub path: String,
    pub additions: u32,
    pub deletions: u32,
    pub status: String,
}

#[derive(Serialize, Deserialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub kind: String, // text | thinking | tool_use | tool_result | image
    pub text: Option<String>,
    pub tool_name: Option<String>,
    pub tool_input: Option<String>,
    pub tool_id: Option<String>,
    #[serde(default)]
    pub is_error: bool,
    /// 文件改动类工具结果携带的目标文件路径。
    pub file_path: Option<String>,
    /// 文件改动类型：add / update / delete。Codex GUI 的 fileChange 用于渲染文件头状态。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_change_type: Option<String>,
    /// file 块：该 `@path` 引用是目录而非文件。前端据此用文件夹图标 +「打开文件夹」。
    /// 仅在确为目录时才置 `Some(true)`（普通文件留 None），让历史会话的文件夹 chip 与
    /// 实时回显一致，而不至于全都显示成文件图标。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_dir: Option<bool>,
    /// 文件改动的结构化 diff（如 Claude 的 structuredPatch）。
    pub diff: Option<Vec<DiffHunk>>,
    /// 图片源：通常为 data:<mime>;base64,<...> 的内联 URL 或 http(s) URL。
    pub image_src: Option<String>,
    /// Chat 粘贴图片在用户正文中的可见占位符。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inline_placeholder: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(rename_all = "camelCase")]
pub struct Msg {
    pub uuid: Option<String>,
    pub role: String,
    pub timestamp: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub sidechain: bool,
    pub blocks: Vec<Block>,
    /// 系统注入的 `type:"user"` 记录的归类：压缩摘要 / skill 注入 / 任务通知 /
    /// 命令输出等。这些记录在 JSONL 里是 `role:"user"`，但不是用户手敲的 prose，
    /// 前端据此把它们渲染成低调的「系统」块而非「Me」气泡。`None` = 真正的用户消息。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta_kind: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashItem {
    pub trash_file: String,
    pub agent: String,
    pub project_label: String,
    pub original_path: String,
    /// 回收站里 JSONL 的绝对路径，供「在回收站里直接查看会话详情」读取。
    pub trash_path: String,
    pub deleted_at: u64,
    pub title: String,
    pub size: u64,
}

/// 全局搜索的命中条目 —— 包含足以「打开这条会话 + 滚到那条消息」的所有上下文。
/// `matched_field` 是字符串而非枚举，方便前端按 i18n key 直接拼一行说明。
/// `snippet` 是命中文本周围一小段（约 120 字符）；前端再按关键词高亮。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchHit {
    /// 命中所属项目，给前端「先 selectProject 再 openSession」的跳转用。
    pub project_key: String,
    pub project_display: String,
    pub session: SessionMeta,
    /// "title" | "id" | "path" | "text"
    pub matched_field: String,
    /// 命中片段；title/id/path 上等于原值，text 上是带前后文的一小段。
    pub snippet: String,
    /// 文本命中所在消息的索引（在 read_session 返回的 Msg 数组里）。
    /// metadata 命中（title/id/path）时为 None —— 这种情况只需打开会话，不需要滚动。
    pub match_msg_index: Option<usize>,
    /// 文本命中所在消息的 uuid（若该 agent 写了 uuid）。和 index 同源；前端优先用 uuid，
    /// 万一从打开会话到滚动之间消息数组发生重排，uuid 能比 index 更稳。
    pub match_msg_uuid: Option<String>,
}

/// 一个会话的 token 用量汇总。三个 agent 用的字段名各不相同，这里统一抽象：
///   - `input_tokens` / `output_tokens` —— 新鲜进 / 出的 token
///   - `cache_creation_input_tokens` —— 写入缓存（仅 Claude 有这个概念，含 5min + 1h 两档）
///   - `cache_creation_1h_input_tokens` —— 上面那个之中属于 1-hour tier 的子集。
///     Anthropic 1h cache write 单价 = 5min 的 2×，所以 cost 公式要单独再加一遍；
///     这个字段是 `cache_creation_input_tokens` 的子集，不要双计 token 数（只在 cost 上加）。
///   - `cache_read_input_tokens` —— 从缓存读（Claude / Codex 都用，字段名不同）
///   - `reasoning_output_tokens` —— 推理 token（仅 Codex / 部分模型）
///   - `total` —— 五项之和；前端通常只展示这一项，hover 展开看细分。
///     `cache_creation_1h_input_tokens` **不** 进 total，因为它已经被 `cache_creation_input_tokens` 包含。
///
/// 任一字段缺失（agent 没记 / 该轮没产生）记 0，结构永远完整，不出 Optional。
#[derive(Serialize, Default, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UsageSummary {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_input_tokens: u64,
    pub cache_creation_1h_input_tokens: u64,
    pub cache_read_input_tokens: u64,
    pub reasoning_output_tokens: u64,
    pub total: u64,
}

impl UsageSummary {
    /// 把 total 字段算上五项之和；构造完直接 `.finalize()` 一下即可，避免调用方各自累加。
    /// `cache_creation_1h_input_tokens` 是 `cache_creation_input_tokens` 的子集，不进 total。
    pub fn finalize(mut self) -> Self {
        self.total = self.input_tokens
            + self.output_tokens
            + self.cache_creation_input_tokens
            + self.cache_read_input_tokens
            + self.reasoning_output_tokens;
        self
    }

    /// 累加另一个 UsageSummary 进来；total 自动重算。聚合统计用。
    pub fn add_assign(&mut self, other: &UsageSummary) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_input_tokens += other.cache_creation_input_tokens;
        self.cache_creation_1h_input_tokens += other.cache_creation_1h_input_tokens;
        self.cache_read_input_tokens += other.cache_read_input_tokens;
        self.reasoning_output_tokens += other.reasoning_output_tokens;
        self.total += other.total;
    }
}

// ============================ 统计 dashboard 用的聚合类型 ============================
// `agent_stats` 命令一次性算齐当前 agent 的所有项目 + 会话，返回这一坨。
// 数据量不大（一个用户最多大概几千个会话），一次性 IPC 传过去比前端逐项 fetch 划算。

/// 某个项目（dirName 级别）的统计聚合：会话数、消息数、token 用量、cost、最后活跃时间。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ProjectStats {
    pub dir_name: String,
    pub display_path: String,
    pub session_count: usize,
    pub message_count: usize,
    pub call_count: u64,
    pub usage: UsageSummary,
    pub cost_usd: f64,
    pub last_modified: u64,
}

/// 某一天（UTC YYYY-MM-DD）的活动量。前端按这串数据画热图 + 时间线图。
/// 用 UTC 是为了不引 chrono 维护本地时区；对国内用户最多差 8h，可接受。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DailyActivity {
    pub date: String,
    pub session_count: usize,
    pub message_count: usize,
    pub call_count: u64,
    pub tokens: u64,
    pub cost_usd: f64,
}

/// Top Sessions 排行里的一条 —— 一次"贵会话"。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct SessionStat {
    /// 该会话所属 agent（"claude" / "codex"），跨 agent 聚合时区分用。
    pub agent: String,
    pub session_id: String,
    pub path: String,
    pub project_display: String,
    pub title: String,
    pub last_modified: u64,
    pub call_count: u64,
    pub usage: UsageSummary,
    pub cost_usd: f64,
}

/// By Model 排行里的一条 —— 按模型聚合的 cost / 调用次数 / cache 命中率。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ModelStat {
    /// 模型原始名（前端用 short_name 做展示，也保留这个用于 tooltip）。
    pub model: String,
    pub label: String,
    pub call_count: u64,
    pub usage: UsageSummary,
    pub cost_usd: f64,
    /// cache_read / (input + cache_read + cache_creation)。0..=1。
    pub cache_hit_rate: f64,
}

/// By Tool / By Shell / By MCP 通用条目：name + calls。
#[derive(Serialize, Default, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub name: String,
    pub count: u64,
}

/// By Activity 一行：分类 + 调用次数 + 成本。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ActivityStat {
    /// 分类 key —— 跟 stats.activity.* 翻译对齐。
    pub key: String,
    pub turn_count: u64,
    pub call_count: u64,
    pub cost_usd: f64,
}

/// 时间范围筛选 —— 前端按按钮切，每次切都触发一次新扫描。
#[derive(Serialize, Default, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum TimeRange {
    Today,
    Days7,
    Days30,
    #[default]
    All,
}

/// 流式统计的完整结果。整个 agent 的统计概览：顶层标量 + 各排行 + 日活时间线。
///
/// `cost_usd` 用 USD 计；前端按需展示美元。`days_active` = UTC 日历日。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct AgentStats {
    /// "all" / "claude" / "codex"。前端按这值给小标题。
    pub scope: String,
    pub session_count: usize,
    pub message_count: usize,
    pub call_count: u64,
    /// 至少出现过一条会话的 UTC 天数。
    pub days_active: usize,
    pub usage: UsageSummary,
    pub cost_usd: f64,
    /// 顶层 cache 命中率（cache_read / (input + cache_read + cache_creation)）。
    pub cache_hit_rate: f64,
    /// 按 cost_usd 降序的项目列表。
    pub projects: Vec<ProjectStats>,
    /// 按日期升序的日活时间线；可能稀疏。
    pub daily_activity: Vec<DailyActivity>,
    /// 按 cost_usd 降序的 Top 10 会话。
    pub top_sessions: Vec<SessionStat>,
    /// 按 cost_usd 降序的模型排行。
    pub by_model: Vec<ModelStat>,
    /// 按调用次数降序的工具排行。
    pub by_tool: Vec<NamedCount>,
    /// 按调用次数降序的 shell 主命令排行（first-token of Bash input）。
    pub by_shell: Vec<NamedCount>,
    /// 按调用次数降序的 MCP server 排行。
    pub by_mcp: Vec<NamedCount>,
    /// 按 cost_usd 降序的活动分类排行。
    pub by_activity: Vec<ActivityStat>,
}

/// 流式推送时的进度负载（事件名：`stats://progress`）。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatsProgress {
    /// 这次流的标识 —— 前端比对 requestId，过时的进度直接丢弃。
    pub request_id: u64,
    pub processed: usize,
    pub total: usize,
    /// 增量快照：到目前为止已处理文件聚合出的 AgentStats。前端可以直接替换 ref。
    pub partial: AgentStats,
}

/// 流式推送完成时的最终负载（事件名：`stats://done`）。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatsDone {
    pub request_id: u64,
    pub stats: AgentStats,
}

/// 流式推送出错时的负载（事件名：`stats://error`）。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StatsError {
    pub request_id: u64,
    pub error: String,
}

/// 托盘弹窗用的轻量统计：每个 agent 在 today / 7d / month 三个窗口的 token + cost。
#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrayAgentSummary {
    pub agent: String,
    pub today_tokens: u64,
    pub today_cost: f64,
    pub week_tokens: u64,
    pub week_cost: f64,
    pub month_tokens: u64,
    pub month_cost: f64,
    pub session_count: usize,
}

#[derive(Serialize, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TrayStats {
    pub agents: Vec<TrayAgentSummary>,
    pub total_today_tokens: u64,
    pub total_today_cost: f64,
    pub total_week_tokens: u64,
    pub total_week_cost: f64,
    pub total_month_tokens: u64,
    pub total_month_cost: f64,
}

// ---- CLI 环境检测 ----

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CliVersionInfo {
    pub cli: String,
    pub npm_package: String,
    pub current_version: Option<String>,
    pub latest_version: Option<String>,
    pub upgradable: bool,
    pub installed: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CliInstallation {
    pub path: String,
    pub version: Option<String>,
    pub is_default: bool,
    pub package_manager: String,
    pub resolved_path: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CliDiagnosisResult {
    pub cli: String,
    pub binary_name: String,
    pub installations: Vec<CliInstallation>,
    pub has_conflict: bool,
    pub error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct CliUpgradeResult {
    pub cli: String,
    pub success: bool,
    pub new_version: Option<String>,
    pub error: Option<String>,
}
