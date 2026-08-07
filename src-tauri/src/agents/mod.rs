// 各 agent 的会话源抽象。
//
// 接入新 agent 的步骤：
//   1. 新建 `agents/<name>.rs`，定义一个 unit struct（如 `<Name>Source`），
//      为它实现下面的 `SessionSource` trait（每个方法各自调用 agent 自己的解析逻辑）。
//   2. 在文件末尾 `pub mod <name>;` 声明 module，并在 `source()` 里加一个 match 分支。
//   3. 前端 `types.ts` 的 `Agent` 联合类型里加上 `"<name>"`，sidebar / 切换 UI 自然支持。
//   4. 所有 Tauri 命令（list_projects / list_sessions / read_session / rename /
//      resume / 回收站）会自动通过 trait 分派下去，调用方零改动。
//
// 不要把 agent-specific 的解析细节漏到 lib.rs 或 trash.rs —— 加 agent 应该是
// 一个文件加一个 match 分支，超出这个范围就说明 trait 的抽象出了问题，需要重新设计。

use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use once_cell::sync::Lazy;

static SEARCH_POOL: Lazy<rayon::ThreadPool> = Lazy::new(|| {
    rayon::ThreadPoolBuilder::new()
        .num_threads(4)
        .thread_name(|i| format!("search-{i}"))
        .build()
        .expect("failed to build search thread pool")
});

use crate::agent_command::AgentCommand;
use crate::stats::types::Turn;
use crate::types::{
    AgentStats, DailyActivity, Msg, ProjectInfo, ProjectStats, SearchHit, SessionMeta, SessionPage,
    UsageSummary,
};
use crate::util::yyyymmdd_local;

/// 「会话 → 用户消息纯文本」缓存：搜索时跳过 JSONL 重新解析。
/// key 是文件绝对路径；value 是 (mtime, Vec<(msg_index, msg_uuid, text)>)。
/// mtime 用来失效检测：文件被改写后下一次搜索会自然重建。
///
/// 这一层只在「全文兜底」分支里读 / 写 —— 命中 title 不会触碰它。
/// 用 Mutex 即可：rayon 把 lock 切片得很小，竞争忽略不计；
/// 真正贵的事在 JSONL 解析 + 字节扫描，不在拿锁。
struct UserTextEntry {
    mtime: u64,
    /// (消息下标, 消息 uuid, 用户消息正文) —— 每条一行。
    msgs: Vec<(usize, Option<String>, String)>,
}
static USER_TEXT_CACHE: Mutex<Option<HashMap<String, UserTextEntry>>> = Mutex::new(None);

fn mtime_of(path: &str) -> u64 {
    use std::time::UNIX_EPOCH;
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 从缓存里拿用户消息正文；命中即返回，否则 None（调用方再去 read_session 重建）。
fn cached_user_text(path: &str, mtime: u64) -> Option<Vec<(usize, Option<String>, String)>> {
    let guard = USER_TEXT_CACHE.lock().ok()?;
    let map = guard.as_ref()?;
    let entry = map.get(path)?;
    if entry.mtime != mtime {
        return None;
    }
    Some(entry.msgs.clone())
}

/// 把刚解析好的用户消息正文写回缓存。
fn store_user_text(path: String, mtime: u64, msgs: Vec<(usize, Option<String>, String)>) {
    if let Ok(mut guard) = USER_TEXT_CACHE.lock() {
        let map = guard.get_or_insert_with(HashMap::new);
        map.insert(path, UserTextEntry { mtime, msgs });
    }
}

/// 搜索取消令牌：每次 `search_sessions` 调用都把自己的 `request_id` 写入
/// `gen`；循环里读到不一样就主动 bail。新搜索 / 显式 `cancel_search` 都会
/// 更新 `gen`，让旧的在跑的搜索立刻让位。
#[derive(Clone, Copy)]
pub struct Cancel<'a> {
    pub request_id: u64,
    pub gen: &'a AtomicU64,
}
impl<'a> Cancel<'a> {
    pub fn cancelled(&self) -> bool {
        self.gen.load(Ordering::Relaxed) != self.request_id
    }
}

/// 全局搜索单次返回上限 —— 防止前端在极端项目下一次性收到上万条命中。
/// UI 用户其实只看头几条，更多结果让用户 narrow query 即可。
const SEARCH_MAX_HITS: usize = 200;

/// 命中片段窗口（字符数）。`text` 字段的匹配返回的小段长度大致 = SNIPPET_WIN * 2。
const SNIPPET_WIN: usize = 60;

pub mod agy;
pub mod claude;
pub mod codex;
pub mod opencode;

/// 程序化聊天（GUI chat）里，agent 子进程 stdout 的一行被归一成的事件。
/// 各 agent 的 [`SessionSource::parse_chat_line`] 把自家 stream-json / JSON 行翻成这套
/// 统一形状，`agent_chat.rs` 只认这一个 enum，完全不感知具体协议。
#[allow(dead_code)] // 部分变体字段按 agent / cfg 可能未读取，保留以保持契约完整。
pub enum ChatEvent {
    /// 一条解析好的消息（assistant 回答 / 工具结果 user 记录）。
    Message(Msg),
    /// 子进程报告的 session id（如 Claude 的 `system`/init 事件）—— 前端据此定位
    /// 落盘的 JSONL、后续 `--resume` 续聊。`api_key_source` 来自 Claude init 的
    /// `apiKeySource`：`"none"` = 订阅/OAuth 登录（受 5 小时 / 周限额约束）；其它值
    /// （`ANTHROPIC_API_KEY` / `apiKeyHelper` / …）= API key 计费，不受 5h/周窗口约束，
    /// 前端据此隐藏限额角标。非 Claude / 拿不到时为 None。
    Init {
        session_id: Option<String>,
        api_key_source: Option<String>,
    },
    /// 一轮回答结束（如 Claude 的 `result` 事件）。`ok=false` 表示该轮出错。
    Result {
        ok: bool,
        usage: Option<UsageSummary>,
    },
    /// token 级流式增量（`--include-partial-messages` 的 `stream_event`）。
    /// 仅长驻 stdin 模型产出；权威 `Message` 仍会随后到达定稿。
    Delta(crate::types::ChatDelta),
    /// 交互式工具权限请求（Claude 控制协议 `can_use_tool`，`--permission-prompt-tool stdio`）。
    /// 前端据此弹「允许 Claude 运行 X？」对话框；用户选择经回写命令送回同一 stdin。
    /// 仅长驻 stdin 模型（Claude）产出。
    Permission(crate::types::ChatPermissionRequest),
    /// 模型向用户提的结构化选择题（Claude 的 `AskUserQuestion` 工具，同走 `can_use_tool`
    /// 控制协议）。前端据此弹「选择题」卡片；用户的选择经回写 `control_response` 送回同一
    /// stdin。仅长驻 stdin 模型（Claude）产出。
    Question(crate::types::ChatQuestionRequest),
    /// 与 UI 无关的行（诊断 / 未知类型）—— 直接丢弃。
    Ignore,
}

/// GUI chat 的「进程模型」—— 不同 agent 的 headless CLI 工作方式不同，`agent_chat.rs`
/// 据此分两条驱动路径（trait 驱动，不在驱动里按 agent 名分支）。
///
/// - `LongLivedStdin`（Claude）：起**一个长驻进程**，多轮用户消息持续写进 stdin
///   （`--input-format stream-json`）。
/// - `OneShotResume`：**一轮一进程**，每条用户消息 spawn 一个
///   `<cli> [resume <id>] "<prompt>"`，跑完即退出；靠 session/thread id resume 续上下文。
/// - `CodexAppServer`：Codex rich-client app-server 长驻 thread；切设置需 restart/resume。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChatProcessModel {
    LongLivedStdin,
    OneShotResume,
    CodexAppServer,
}

impl ChatProcessModel {
    /// 给前端的稳定标识：前端据此决定「切设置」是要 restart-with-resume（长驻）还是
    /// 改下轮 flag 即可（one-shot）。
    pub fn as_str(&self) -> &'static str {
        match self {
            ChatProcessModel::LongLivedStdin => "longLivedStdin",
            ChatProcessModel::OneShotResume => "oneShotResume",
            ChatProcessModel::CodexAppServer => "codexAppServer",
        }
    }
}

#[allow(dead_code)] // `name` / `image_src` 暂时只在调试/未来扩展中使用，但保留在 trait 上让 agent 契约完整。
pub trait SessionSource: Send + Sync {
    /// agent 标识，跟前端 `Agent` 联合类型保持一致（"claude" / "codex" / ...）。
    fn name(&self) -> &'static str;

    /// 列出该 agent 下的所有项目（已折叠到磁盘 / cwd 的逻辑各自负责）。
    fn list_projects(
        &self,
        include_codex_internal: bool,
        include_codex_archived: bool,
    ) -> Result<Vec<ProjectInfo>, String>;

    /// 分页返回某项目下的会话元信息。`project_key` 的含义由 agent 自己决定：
    /// Claude 是项目目录名，Codex 是 cwd 路径。
    fn list_sessions(
        &self,
        project_key: &str,
        offset: usize,
        limit: usize,
        include_codex_internal: bool,
        include_codex_archived: bool,
    ) -> Result<SessionPage, String>;

    /// 解析一个 JSONL 文件并返回标准 `Msg[]`（前端只认这一个形状）。
    fn read_session(&self, path: &str) -> Result<Vec<Msg>, String>;

    /// 实施重命名：写入合适的元数据行 + 必要的旁路（如 codex 还要更新 session_index / sqlite）。
    /// path 已经被 lib.rs 预校验（存在且是 .jsonl），不必再重复检查。
    fn rename_session(&self, path: &Path, name: &str) -> Result<(), String>;

    /// `/fork`：把既有会话**克隆**成一个全新、独立的磁盘 transcript（新 session id、新消息
    /// uuid），并打上 `title`（写成 custom-title）。返回新 session id。`project_key` 含义同
    /// `list_sessions`（Claude = 项目目录名）。仅支持「派生」语义的 agent（Claude）实现；
    /// 默认不支持（其它 agent 调用即报错）。
    fn fork_session(
        &self,
        _project_key: &str,
        _source_id: &str,
        _title: &str,
    ) -> Result<String, String> {
        Err("此 agent 不支持 fork 会话".into())
    }

    /// 复制目标用户提问之前的 transcript；`keep_user_turns` 是要保留的真实用户输入数。
    fn fork_session_before_user_turn(
        &self,
        _project_key: &str,
        _source_id: &str,
        _title: &str,
        _keep_user_turns: usize,
    ) -> Result<String, String> {
        Err("此 agent 不支持按用户轮次 fork 会话".into())
    }

    /// 回收站标题：用 agent 自己的解析逻辑提取展示名。
    fn trash_title(&self, path: &Path) -> String;

    /// 终端里 resume 一个会话用的 CLI 命令。`session_id` 已经过 [A-Za-z0-9-]+ 校验。
    fn resume_command(&self, session_id: &str, path: &str) -> AgentCommand;

    /// 终端里开一个全新会话用的 CLI 命令（不带 --resume）。
    fn new_session_command(&self) -> AgentCommand;

    /// GUI chat 输入框 `/` 浮层的动态指令列表 —— 扫磁盘上该 agent 的自定义命令 /
    /// user-invocable skills（headless 下能展开的那些），**不含 TUI 内置命令**。
    /// `cwd` 用于扫项目级 `.claude/commands/`。默认空：未适配的 agent 不提供。
    fn chat_slash_commands(&self, _cwd: &str) -> Vec<crate::types::SlashCommand> {
        Vec::new()
    }

    /// 程序化聊天（GUI chat）的子进程命令 —— 跑该 agent 的 headless stream-json 模式
    /// （纯管道，不走 PTY）。`session_id` 给出时续聊该会话；`permission_mode` 决定工具
    /// 审批策略（MVP 走 `acceptEdits`）。返回 `None` 表示该 agent 暂无可用的 headless
    /// chat 模式 —— 调用方据此禁用 GUI 入口 / 退回方案 A（TUI）。
    ///
    /// 默认 `None`：尚未适配的 agent（codex）不必改动即可编译，Phase 3 再实现。
    ///
    /// `model` / `effort` 为 `None` 时走 CLI 自身默认（不下发对应 flag）。长驻进程模型下
    /// 这两者在 start 时定型；切换需 restart-with-resume（前端据 `chat_process_model` 决策）。
    ///
    /// `fork` = true 且带 `session_id` 时：从该会话**派生**一个新 session id（不续写原文件）。
    /// 供「btw 侧聊」继承主聊上下文却不污染其 transcript。仅支持派生语义的 agent（Claude
    /// `--fork-session`）会用它；其它 agent 忽略。
    fn chat_command(
        &self,
        _session_id: Option<&str>,
        _permission_mode: &str,
        _model: Option<&str>,
        _effort: Option<&str>,
        _fork: bool,
    ) -> Option<AgentCommand> {
        None
    }

    /// 把子进程 stdout 的一行归一成 [`ChatEvent`]。事件归一逻辑放在各 agent 模块内
    /// （不污染 trait 形状、不在 `lib.rs`/`agent_chat.rs` 加 agent 分支）。
    ///
    /// 默认 `Ignore`：未实现 headless 的 agent 不会被 `agent_chat.rs` 起进程，所以这条
    /// 默认实现实际不会被调用，只为让 trait 契约完整、编译通过。
    fn parse_chat_line(&self, _line: &str) -> ChatEvent {
        ChatEvent::Ignore
    }

    /// 该 agent 的 GUI chat 进程模型。默认 `LongLivedStdin`（Claude）；
    /// Codex 覆写为 `CodexAppServer`。`agent_chat.rs` 据此选驱动路径。
    fn chat_process_model(&self) -> ChatProcessModel {
        ChatProcessModel::LongLivedStdin
    }

    /// 【LongLivedStdin 用】把一条用户消息编码成写进子进程 stdin 的**一行**（不含换行）。
    /// 默认 = Anthropic stream-json 用户消息形状（content 数组 = [image…, text]）——
    /// Claude 直接用默认；其它 stdin 形状不同的 LongLivedStdin agent 可覆写。
    ///
    /// 把编码从 `agent_chat.rs` 收进 trait，是为了让驱动彻底 agent-agnostic（不再写死
    /// 某家的 stdin 形状）。
    fn chat_encode_input(&self, text: &str, images: &[crate::types::ChatImageInput]) -> String {
        let mut content: Vec<Value> = Vec::new();
        for img in images {
            content.push(serde_json::json!({
                "type": "image",
                "source": {
                    "type": "base64",
                    "media_type": img.media_type,
                    "data": img.data,
                }
            }));
        }
        if !text.is_empty() {
            content.push(serde_json::json!({ "type": "text", "text": text }));
        }
        serde_json::json!({
            "type": "user",
            "message": { "role": "user", "content": content }
        })
        .to_string()
    }

    /// 【OneShotResume 用】构造「这一轮」的子进程命令：把 `prompt` 直接编进命令
    /// （如 `codex exec [resume <id>] --json "<prompt>"`）。`session_id` 给出时 resume
    /// 续上一轮的上下文。返回 `None` 表示该 agent 暂无可用 one-shot headless chat。
    ///
    /// 默认 `None`：LongLivedStdin agent（Claude）和尚未适配的 agent 都不必实现。
    ///
    /// `model` / `effort` 为 `None` 时走 CLI / 配置默认。one-shot 模型下三者（含
    /// `permission_mode`）每轮重新下发，故切换**免费即时生效**（下一轮带新 flag）。
    fn chat_turn_command(
        &self,
        _session_id: Option<&str>,
        _prompt: &str,
        _permission_mode: &str,
        _model: Option<&str>,
        _effort: Option<&str>,
    ) -> Option<AgentCommand> {
        None
    }

    /// 从单个 content 块中尝试提取图片 src（data:URL 或外链）。
    /// 主要供该 agent 自己的 `read_session` 内部使用，放在 trait 上也方便外部预览图片块。
    fn image_src(&self, block: &Value) -> Option<String>;

    /// 单个会话的 token 用量汇总。空数据 / agent 不记 token 时返回
    /// `UsageSummary::default()` 占位 —— 前端可以照画零值角标，不需要特判 None。
    /// 调用方应该自己负责缓存（`session_usage` 命令走 `USAGE_CACHE`）。
    fn usage_summary(&self, path: &str) -> Result<UsageSummary, String>;

    /// 「当前上下文」估算 —— 取文件里**最后一条**带非零 usage 的记录
    /// （≈ 会话末尾喂给模型的总输入 = 当前上下文规模），区别于 `usage_summary` 的
    /// 全程累加。用于 resume 后立刻把上下文进度角标填成真实值，而不必空等下一轮
    /// result 才有数（否则刚续聊时显示 0% 与 TUI 不符）。默认返回 default
    /// （agent 不记 token 时为 0）；记 token 的 agent 重写。
    fn context_usage(&self, _path: &str) -> Result<UsageSummary, String> {
        Ok(UsageSummary::default())
    }

    /// 取会话中最后一条用户消息的纯文本（截断到首行 ≤120 字符），用于列表副标题。
    /// 默认返回 None；各 agent 按自己的存储格式从尾部快速读取。
    fn last_prompt(&self, _path: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// 把一个 JSONL 解析成 `Turn` 列表，给统计聚合器（stats）使用。
    /// 一个 Turn = 一条用户消息 + 紧随其后的 N 个 assistant API call；
    /// 每个 call 记录该次调用用了哪个模型、产生了多少 token、调用了哪些工具
    /// （含 Bash 命令首词 / MCP server 名）。
    ///
    /// Agent 没记某些字段（如 Codex 把 token 算在 session
    /// 级而非 call 级）时按 0 / 空列表处理，不要返回错误 —— 一个坏文件不要拖垮
    /// 整个全局统计。失败仅在文件完全无法打开时返回 Err，调用方会跳过这个文件。
    fn read_turns(&self, path: &str) -> Result<Vec<Turn>, String>;

    /// 统计扫描时使用的会话发现接口 —— 默认实现 = list_sessions(0, usize::MAX)。
    /// Claude 重写它以同时纳入 `<projects>/<dir>/<sessionId>/subagents/*.jsonl`，
    /// 否则统计会缺一大块（sub-agent 是实打实的 API 调用且独立计费）。
    /// list_sessions 仍只返回顶层文件 —— 别把 sub-agent 塞进聊天列表，否则
    /// 用户的会话清单会被自动生成的小段污染。
    fn discover_stats_sessions(&self, project_key: &str) -> Result<Vec<SessionMeta>, String> {
        Ok(self
            .list_sessions(project_key, 0, usize::MAX, false, false)?
            .sessions)
    }

    /// 单会话统计时的同伴文件 —— 默认返回空，Claude 重写以返回
    /// `<parent>/subagents/*.jsonl`。`run_session_scope` 把它们和 parent 一起喂给
    /// 同一个 Aggregator，让单会话 cost / call 跟全局 by-session 那一行对得上。
    /// 共用一个 aggregator，`seen_message_ids` 会自动去重跨文件复制的 message-id。
    fn discover_session_companions(&self, _path: &str) -> Vec<SessionMeta> {
        Vec::new()
    }

    /// 会话「数据源」的 mtime —— usage / 搜索文本缓存的失效锚点。
    /// 文件型 agent = 会话文件自身的 mtime（默认实现）；库型 agent（opencode 的
    /// `opencode://` 虚拟路径没有对应文件，fs mtime 恒 0 会让缓存永不失效）重写成
    /// 库文件的 mtime。
    fn source_mtime(&self, path: &str) -> u64 {
        mtime_of(path)
    }

    /// 全文搜索预筛：会话原始数据里是否包含 `q_lower`（ASCII 大小写不敏感）。
    /// 只是粗筛 —— 命中后仍由 `find_text_hit` 在「用户消息 text 块」里精确匹配。
    /// 默认 = 字节层扫会话文件；库型 agent 重写成 SQL。
    fn contains_text(&self, path: &str, q_lower: &str) -> bool {
        file_contains_ci(path, q_lower)
    }

    /// 实时 tail（watch.rs）需要盯的真实磁盘文件。默认 = 会话文件自身；
    /// agy 重写成 transcript_full 优先，opencode 重写成库的 -wal 文件。
    /// 返回 None 表示该会话没有可盯的文件（前端静默降级为一次性读取）。
    fn watch_target(&self, path: &str) -> Option<std::path::PathBuf> {
        Some(std::path::PathBuf::from(path))
    }

    /// rename 等写操作前的路径合法性检查（lib.rs 统一调用，不再自带 exists/.jsonl
    /// 硬编码）。文件型 agent 用默认实现；虚拟路径 agent（opencode）重写。
    fn validate_session_path(&self, path: &Path) -> Result<(), String> {
        if !path.exists() {
            return Err("Session file does not exist".to_string());
        }
        if !crate::util::is_jsonl(path) {
            return Err("Not a JSONL file".to_string());
        }
        Ok(())
    }
}

// ============================ 用量缓存（按文件 mtime 失效） ============================
// 跟 USER_TEXT_CACHE 同模式：把每个 JSONL 的解析结果用 (path, mtime) 锁住，
// 后端命令 `session_usage` 命中直接返回，miss 才让 agent 走一次全文件扫描。
// 单个 entry ~ 48 B，放心存。
static USAGE_CACHE: Mutex<Option<HashMap<String, (u64, UsageSummary)>>> = Mutex::new(None);

fn cached_usage(path: &str, mtime: u64) -> Option<UsageSummary> {
    let g = USAGE_CACHE.lock().ok()?;
    let m = g.as_ref()?;
    let (saved, u) = m.get(path)?;
    if *saved != mtime {
        return None;
    }
    Some(*u)
}

fn store_usage(path: String, mtime: u64, u: UsageSummary) {
    if let Ok(mut g) = USAGE_CACHE.lock() {
        let m = g.get_or_insert_with(HashMap::new);
        m.insert(path, (mtime, u));
    }
}

/// 命令层调用入口：先查缓存、miss 才让 agent 走 `usage_summary`。
/// 这一层不在 trait 上是为了让具体 agent 不必感知缓存策略 —— 各 agent 只关心
/// 「读一个文件、算出 UsageSummary」即可。
pub fn session_usage(src: &(dyn SessionSource + Sync), path: &str) -> Result<UsageSummary, String> {
    let mt = src.source_mtime(path);
    if let Some(u) = cached_usage(path, mt) {
        return Ok(u);
    }
    let u = src.usage_summary(path)?;
    store_usage(path.to_string(), mt, u);
    Ok(u)
}

// ============================ 统计 dashboard ============================
// 一次性把当前 agent 下的所有项目 + 会话扫一遍，得出聚合数字 / 项目排行 /
// 日活轴。本身不缓存 —— 上游 `session_usage` 已经有 (path, mtime) 缓存，
// 二次调用走的是 cache 命中路径，整体开销是常数级。
//
// 实现：
//   1) `list_projects` 拿所有项目
//   2) 对每个项目 `list_sessions(.., 0, usize::MAX)` 拉全量 SessionMeta
//   3) 把所有 (project_idx, SessionMeta) 拍平，再 par_iter 拉 usage
//   4) 单线程聚合：按 project_idx 累加 / 按 yyyymmdd_utc 分桶
//   5) projects 按 usage.total 降序、daily 按日期升序输出
pub fn agent_stats(
    src: &(dyn SessionSource + Sync),
    agent_name: &str,
) -> Result<AgentStats, String> {
    let projects = src.list_projects(false, false)?;

    // Pull every session per project. List_sessions is cheap (just mtime + deep-parse window).
    // 用 usize::MAX 让 agent 把所有都返回（pagination 在这层不需要）。
    let mut items: Vec<(usize, SessionMeta)> = Vec::new();
    for (i, p) in projects.iter().enumerate() {
        match src.list_sessions(&p.dir_name, 0, usize::MAX, false, false) {
            Ok(page) => {
                for s in page.sessions {
                    items.push((i, s));
                }
            }
            // 单个项目坏了不让整盘挂；统计页上当作 0 处理。
            Err(_) => continue,
        }
    }

    // 并行拉 usage。session_usage 内部走 (path, mtime) 缓存，重复调用基本零成本。
    let usages: Vec<UsageSummary> = items
        .par_iter()
        .map(|(_, s)| session_usage(src, &s.path).unwrap_or_default())
        .collect();

    // 项目级聚合槽
    let mut project_stats: Vec<ProjectStats> = projects
        .iter()
        .map(|p| ProjectStats {
            dir_name: p.dir_name.clone(),
            display_path: p.display_path.clone(),
            ..Default::default()
        })
        .collect();

    // 日活分桶
    let mut daily: HashMap<String, DailyActivity> = HashMap::new();
    // 顶层标量
    let mut total = AgentStats {
        scope: agent_name.to_string(),
        ..Default::default()
    };

    for ((proj_idx, s), u) in items.iter().zip(usages.iter()) {
        // 项目槽
        let p = &mut project_stats[*proj_idx];
        p.session_count += 1;
        p.message_count += s.message_count;
        p.usage.add_assign(u);
        p.last_modified = p.last_modified.max(s.modified);

        // 日活槽
        let date = yyyymmdd_local(s.modified);
        let d = daily.entry(date.clone()).or_default();
        if d.date.is_empty() {
            d.date = date;
        }
        d.session_count += 1;
        d.message_count += s.message_count;
        d.tokens += u.total;

        // 顶层标量
        total.session_count += 1;
        total.message_count += s.message_count;
        total.usage.add_assign(u);
    }

    // 项目按 token 总量降序；零 token 的项目沉底
    project_stats.sort_by_key(|p| std::cmp::Reverse(p.usage.total));
    // 日活按日期升序，便于前端直接绘图
    let mut daily_vec: Vec<DailyActivity> = daily.into_values().collect();
    daily_vec.sort_by(|a, b| a.date.cmp(&b.date));

    total.days_active = daily_vec.len();
    total.projects = project_stats;
    total.daily_activity = daily_vec;
    Ok(total)
}

/// 全局搜索的具体实现 —— 拎到 trait 外的自由函数里，参数收 `&dyn SessionSource`，
/// 这样可以在闭包 / rayon 里随意复制 `&dyn` 引用，绕开 trait 默认方法
/// 对 `Self: ?Sized` 的限制。
///
/// 性能要点：
///
///   1. 元数据（title / id / cwd / 项目路径）匹配先做，命中即返回，不读文件；
///   2. 元数据未中再走全文 —— 但先用 `file_contains_ci` 在字节层快速过滤，
///      只有可能命中的会话才会触发 `read_session` 的完整 JSON 解析；
///   3. 项目内所有会话用 rayon 并行扫描，CPU 多核场景下接近线性加速；
///   4. **可取消**：循环里多处检查 `Cancel::cancelled()`，被新搜索 / 显式 cancel
///      让位时立即 bail；返回 `Ok(Vec::new())`，前端的 reqSeq 守卫负责丢掉结果。
///
/// 命中按「项目 last_modified → 会话 modified」降序输出（与侧栏 / 会话列表一致）。
pub fn search(
    src: &(dyn SessionSource + Sync),
    query: &str,
    project_filter: Option<&str>,
    cancel: Cancel<'_>,
) -> Result<Vec<SearchHit>, String> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    // 没指定项目就扫全部；指定时只搜该项目，跳过其它项目的 list_sessions 调用。
    let projects = src.list_projects(false, false)?;
    let projects: Vec<ProjectInfo> = match project_filter {
        Some(key) => projects.into_iter().filter(|p| p.dir_name == key).collect(),
        None => projects,
    };
    // 先收集所有项目的会话列表（顺序，但很快——只是目录 stat）
    let mut all_sessions: Vec<(String, String, Vec<SessionMeta>)> = Vec::new();
    for proj in projects {
        if cancel.cancelled() {
            return Ok(Vec::new());
        }
        let page = match src.list_sessions(&proj.dir_name, 0, usize::MAX, false, false) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if !page.sessions.is_empty() {
            all_sessions.push((
                proj.dir_name.clone(),
                proj.display_path.clone(),
                page.sessions,
            ));
        }
    }
    // 把所有项目的会话展平，跨项目并行扫描
    let flat: Vec<(String, String, SessionMeta)> = all_sessions
        .into_iter()
        .flat_map(|(key, display, sessions)| {
            sessions
                .into_iter()
                .map(move |s| (key.clone(), display.clone(), s))
        })
        .collect();
    let hits: Vec<SearchHit> = SEARCH_POOL.install(|| {
        flat.into_par_iter()
            .filter_map(|(project_key, project_display, session)| {
                if cancel.cancelled() {
                    return None;
                }
                classify_hit(src, &project_key, &project_display, session, &q, cancel)
            })
            .collect()
    });
    if cancel.cancelled() {
        return Ok(Vec::new());
    }
    let mut hits = hits;
    hits.sort_by_key(|h| std::cmp::Reverse(h.session.modified));
    hits.truncate(SEARCH_MAX_HITS);
    Ok(hits)
}

/// 对单个会话做完整的「命中分类」—— 元数据先 / 文本兜底。返回 None 表示该会话
/// 没有任何命中字段（这条会话不进结果）。提到 trait 外是为了能拿 `&dyn SessionSource`
/// 在闭包里随便用，避免对 `Self` 的 sized 限制。
fn classify_hit(
    src: &(dyn SessionSource + Sync),
    project_key: &str,
    project_display: &str,
    session: SessionMeta,
    q: &str,
    cancel: Cancel<'_>,
) -> Option<SearchHit> {
    // 全局搜索范围：只看「会话标题」+「用户发的消息」—— 助手回复 / thinking /
    // 工具调用 / 工具结果 / 项目路径 / 会话 ID 都不再参与匹配。
    let title_l = session.title.to_lowercase();
    let mut match_msg_index: Option<usize> = None;
    let mut match_msg_uuid: Option<String> = None;
    let (field, snippet) = if title_l.contains(q) {
        ("title", session.title.clone())
    } else {
        if cancel.cancelled() {
            return None;
        }
        // 缓存热时直接内存扫描，跳过磁盘 I/O
        let mtime = src.source_mtime(&session.path);
        let cached = cached_user_text(&session.path, mtime);
        if let Some(ref texts) = cached {
            {
                let hit = scan_user_text(texts, q)?;
                match_msg_index = Some(hit.msg_index);
                match_msg_uuid = hit.msg_uuid;
                ("text", hit.snippet)
            }
        } else {
            // 冷路径：粗筛（文件型 = 字节扫描；库型 = SQL）→ JSON 解析
            if !src.contains_text(&session.path, q) {
                return None;
            }
            if cancel.cancelled() {
                return None;
            }
            {
                let hit = find_text_hit(|p| src.read_session(p), &session.path, mtime, q)?;
                match_msg_index = Some(hit.msg_index);
                match_msg_uuid = hit.msg_uuid;
                ("text", hit.snippet)
            }
        }
    };
    Some(SearchHit {
        project_key: project_key.to_string(),
        project_display: project_display.to_string(),
        session,
        matched_field: field.to_string(),
        snippet,
        match_msg_index,
        match_msg_uuid,
    })
}

/// 命中一条文本时返回的元信息：片段 + 消息在数组里的索引 + 消息 uuid（可选）。
/// 前端用 (uuid 或 index) 在加载完会话后定位到具体消息并触发闪烁动画。
struct TextHit {
    snippet: String,
    msg_index: usize,
    msg_uuid: Option<String>,
}

/// 读一个会话，找第一条命中。仅匹配「用户消息的 text 块」 ——
/// 助手回复 / thinking / tool_use / tool_result / 图片全部跳过。
/// 「我之前问过什么」是用户最常想检索的轴，这条策略让结果直接得多。
/// `q` 必须已经小写化。失败 / 无命中返回 None。
///
/// 走 `USER_TEXT_CACHE`：相同 (path, mtime) 第二次搜索直接拿纯文本，跳过
/// JSONL 反序列化。冷启动仍然走 `read_session`（FnOnce 闭包提供），但解析完
/// 立刻把「用户消息正文」抽出来缓存，下一次搜任何关键词都是 in-memory 操作。
/// `mtime` 由调用方经 `SessionSource::source_mtime` 提供（虚拟路径的 fs mtime 恒 0，
/// 不能在这里自己 stat）。
fn find_text_hit<F>(read: F, path: &str, mtime: u64, q: &str) -> Option<TextHit>
where
    F: FnOnce(&str) -> Result<Vec<Msg>, String>,
{
    if let Some(cached) = cached_user_text(path, mtime) {
        return scan_user_text(&cached, q);
    }
    // 冷路径：解析 + 抽取 + 缓存
    let msgs = read(path).ok()?;
    let mut user_texts: Vec<(usize, Option<String>, String)> = Vec::new();
    for (i, msg) in msgs.into_iter().enumerate() {
        if msg.role != "user" {
            continue;
        }
        let uuid = msg.uuid.clone();
        // 用户消息可能有多个 text 块（图片附件 + 文字、连续 prompt 等）—— 拼成一段
        // 避免缓存太碎，搜索时一行一次 substring 比若干次小串更高效。
        let mut combined = String::new();
        for blk in msg.blocks {
            if blk.kind != "text" {
                continue;
            }
            if let Some(text) = blk.text {
                if !combined.is_empty() {
                    combined.push('\n');
                }
                combined.push_str(&text);
            }
        }
        if !combined.is_empty() {
            user_texts.push((i, uuid, combined));
        }
    }
    let hit = scan_user_text(&user_texts, q);
    store_user_text(path.to_string(), mtime, user_texts);
    hit
}

/// 在已抽取的「用户消息正文」列表里扫第一条命中。
fn scan_user_text(texts: &[(usize, Option<String>, String)], q: &str) -> Option<TextHit> {
    for (idx, uuid, text) in texts {
        if let Some(snip) = match_snippet(text, q) {
            return Some(TextHit {
                snippet: snip,
                msg_index: *idx,
                msg_uuid: uuid.clone(),
            });
        }
    }
    None
}

/// 廉价的「文件里有没有这个串」检查 —— 用来在跑 JSON 全量解析前先把一堆
/// 显然不命中的会话筛掉。`q_lower` 必须已经小写化。
///
/// ASCII 查询走快路径：`windows().eq_ignore_ascii_case`，不分配。
/// 含非 ASCII 字符的查询退到 `to_lowercase().contains` —— 多一次分配，
/// 但 CJK / 重音字母按 unicode 折叠的场景本来就少。
fn file_contains_ci(path: &str, q_lower: &str) -> bool {
    if q_lower.is_empty() {
        return false;
    }
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(_) => return false,
    };
    if q_lower.is_ascii() {
        let q = q_lower.as_bytes();
        if bytes.len() < q.len() {
            return false;
        }
        // windows().any() 在编译器优化下接近 memmem 性能；够用且零额外依赖。
        bytes.windows(q.len()).any(|w| w.eq_ignore_ascii_case(q))
    } else {
        match std::str::from_utf8(&bytes) {
            Ok(s) => {
                // CJK / 非拉丁字符没有大小写变体，跳过整文件 to_lowercase 的分配
                let has_ascii_letter = q_lower.bytes().any(|b| b.is_ascii_alphabetic());
                if has_ascii_letter {
                    s.to_lowercase().contains(q_lower)
                } else {
                    s.contains(q_lower)
                }
            }
            Err(_) => false,
        }
    }
}

/// 在 `hay` 中按小写匹配 `q`，命中时返回前后各 SNIPPET_WIN 字符的片段
/// （按字符切，不按字节，避免切到 utf-8 中间）。
fn match_snippet(hay: &str, q: &str) -> Option<String> {
    let hay_l = hay.to_lowercase();
    let byte_idx = hay_l.find(q)?;
    // 把 byte index 翻成 char index 才能安全切 utf-8。
    let char_idx = hay_l[..byte_idx].chars().count();
    let chars: Vec<char> = hay.chars().collect();
    let start = char_idx.saturating_sub(SNIPPET_WIN);
    let end = (char_idx + q.chars().count() + SNIPPET_WIN).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    // 长行（粘进来的代码 / json）里可能有大量 newline / 控制空白 —— 折叠成单空格
    // 便于在一行结果里渲染。
    let collapsed: String = out
        .chars()
        .map(|c| if c.is_whitespace() { ' ' } else { c })
        .collect();
    Some(collapsed.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// 按 agent 名拿到一个具体的会话源。Unknown agent 返回错误，调用方应直接透传给前端。
pub fn source(agent: &str) -> Result<Box<dyn SessionSource>, String> {
    match agent {
        "agy" => Ok(Box::new(agy::AgySource)),
        "claude" => Ok(Box::new(claude::ClaudeSource)),
        "codex" => Ok(Box::new(codex::CodexSource)),
        "opencode" => Ok(Box::new(opencode::OpencodeSource)),
        other => Err(format!("Unknown agent: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippet_returns_match_with_surrounding_context() {
        let hay = "the quick brown fox jumps over the lazy dog";
        let snip = match_snippet(hay, "fox").unwrap();
        // 命中片段保留命中前后；上下文够短就不会带省略号。
        assert!(snip.contains("fox"));
        assert!(snip.contains("brown"));
        assert!(snip.contains("jumps"));
    }

    #[test]
    fn snippet_collapses_whitespace_into_single_spaces() {
        let hay = "alpha\n\tbeta   gamma";
        let snip = match_snippet(hay, "beta").unwrap();
        assert!(!snip.contains('\n'));
        assert!(!snip.contains('\t'));
        assert!(snip.contains("alpha beta gamma"));
    }

    #[test]
    fn snippet_is_case_insensitive_but_preserves_original_case() {
        let snip = match_snippet("Hello World", "world").unwrap();
        assert!(snip.contains("World")); // 命中段原样保留大写
    }

    #[test]
    fn snippet_returns_none_when_query_absent() {
        assert!(match_snippet("nothing here", "missing").is_none());
    }

    #[test]
    fn snippet_handles_multibyte_characters_safely() {
        // 验证按 char 切而非按 byte 切——切到 CJK 中间会 panic。
        let hay = "我们今天搜索一段中文然后再来一点english tail";
        let snip = match_snippet(hay, "english").unwrap();
        assert!(snip.contains("english"));
    }

    #[test]
    fn snippet_marks_truncation_with_ellipsis() {
        let hay: String = "a".repeat(200) + "needle" + &"b".repeat(200);
        let snip = match_snippet(&hay, "needle").unwrap();
        assert!(snip.starts_with('…'));
        assert!(snip.ends_with('…'));
    }

    // ---- find_text_hit: 只匹配「用户消息的 text 块」 ----
    fn block(kind: &str, text: Option<&str>) -> crate::types::Block {
        crate::types::Block {
            kind: kind.to_string(),
            text: text.map(String::from),
            ..Default::default()
        }
    }
    fn msg_with_role(role: &str, blocks: Vec<crate::types::Block>) -> Msg {
        Msg {
            uuid: None,
            role: role.to_string(),
            timestamp: None,
            model: None,
            sidechain: false,
            blocks,
            meta_kind: None,
        }
    }
    fn msg(blocks: Vec<crate::types::Block>) -> Msg {
        msg_with_role("user", blocks)
    }

    // 用唯一 path 每个测试 —— USER_TEXT_CACHE 是进程级 Mutex，path 重名会让用例互相
    // 污染。tests 在不存在的文件路径上跑（read 闭包注入 msgs），所以 path 只是缓存 key。
    fn unique_path(tag: &str) -> String {
        format!("__test_find_text_hit_{tag}__")
    }

    #[test]
    fn find_text_hit_skips_tool_use_and_tool_result_blocks() {
        let mut tool_call = block("tool_use", None);
        tool_call.tool_name = Some("needle-runner".to_string());
        tool_call.tool_input = Some("{\"q\":\"needle\"}".to_string());
        let tool_result = block("tool_result", Some("needle was found in stack"));
        let msgs = vec![msg(vec![tool_call, tool_result])];
        let read = move |_p: &str| Ok(msgs);
        let p = unique_path("tool_blocks");
        assert!(find_text_hit(read, &p, 0, "needle").is_none());
    }

    #[test]
    fn find_text_hit_matches_only_in_user_text_blocks() {
        let msgs = vec![msg(vec![block("text", Some("hello world"))])];
        let read = move |_p: &str| Ok(msgs);
        let p = unique_path("user_text");
        let hit = find_text_hit(read, &p, 0, "world").expect("expected a hit");
        assert_eq!(hit.msg_index, 0);
    }

    #[test]
    fn find_text_hit_skips_assistant_messages() {
        let msgs = vec![msg_with_role(
            "assistant",
            vec![block("text", Some("I think the needle is in the haystack"))],
        )];
        let read = move |_p: &str| Ok(msgs);
        let p = unique_path("assistant");
        assert!(find_text_hit(read, &p, 0, "needle").is_none());
    }

    #[test]
    fn find_text_hit_skips_thinking_blocks() {
        let msgs = vec![msg(vec![block("thinking", Some("planning carefully"))])];
        let read = move |_p: &str| Ok(msgs);
        let p = unique_path("thinking");
        assert!(find_text_hit(read, &p, 0, "carefully").is_none());
    }

    #[test]
    fn find_text_hit_returns_the_index_of_the_first_matching_user_message() {
        let msgs = vec![
            msg_with_role("assistant", vec![block("text", Some("the needle ignored"))]),
            msg(vec![block("text", Some("the needle is here"))]),
        ];
        let read = move |_p: &str| Ok(msgs);
        let p = unique_path("first_user");
        let hit = find_text_hit(read, &p, 0, "needle").expect("expected a hit");
        assert_eq!(hit.msg_index, 1);
    }

    #[test]
    fn find_text_hit_warm_cache_skips_the_read_closure() {
        // 第一次：read 闭包被调用，缓存写入
        let msgs = vec![msg(vec![block("text", Some("cached message"))])];
        let read1 = move |_p: &str| Ok(msgs);
        let p = unique_path("warm_cache");
        find_text_hit(read1, &p, 0, "cached").expect("first call should hit");
        // 第二次：闭包应该完全不被调用（断言 panic 来证明）
        let read2 = |_p: &str| -> Result<Vec<Msg>, String> {
            panic!("read closure must not be called on warm cache")
        };
        let hit = find_text_hit(read2, &p, 0, "message").expect("second call should still hit");
        assert_eq!(hit.msg_index, 0);
    }

    // ---- file_contains_ci: 字节级 ASCII fast path + UTF-8 fallback ----
    use std::io::Write as _;
    fn tmp_file(name: &str, body: &[u8]) -> std::path::PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("csv-search-{}-{}", std::process::id(), name));
        let mut f = std::fs::File::create(&p).unwrap();
        f.write_all(body).unwrap();
        p
    }

    #[test]
    fn file_contains_ci_finds_ascii_case_insensitive() {
        let p = tmp_file("ascii", b"The Quick Brown Fox");
        let path = p.to_string_lossy().to_string();
        assert!(file_contains_ci(&path, "quick"));
        assert!(file_contains_ci(&path, "fox"));
        assert!(!file_contains_ci(&path, "missing"));
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn file_contains_ci_handles_utf8_query() {
        let p = tmp_file("utf8", "我们今天搜索一段中文".as_bytes());
        let path = p.to_string_lossy().to_string();
        assert!(file_contains_ci(&path, "中文"));
        assert!(!file_contains_ci(&path, "英文"));
        std::fs::remove_file(p).ok();
    }

    #[test]
    fn file_contains_ci_returns_false_for_missing_path() {
        assert!(!file_contains_ci(
            "/no/such/file/for/csv-test.txt",
            "anything"
        ));
    }

    #[test]
    fn cancel_token_reports_cancellation_when_gen_changes() {
        let gen = AtomicU64::new(7);
        let c = Cancel {
            request_id: 7,
            gen: &gen,
        };
        assert!(!c.cancelled(), "fresh token should not be cancelled");
        gen.store(8, Ordering::SeqCst); // newer search took over
        assert!(c.cancelled(), "old token should now be cancelled");
        gen.store(7, Ordering::SeqCst); // restore — back to live
        assert!(!c.cancelled());
        gen.fetch_add(1, Ordering::SeqCst); // explicit cancel_search bump
        assert!(c.cancelled());
    }
}
