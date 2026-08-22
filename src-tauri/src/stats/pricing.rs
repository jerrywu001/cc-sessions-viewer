// 模型 → $/token 价格表。首选 js-bridge 的 models.dev 镜像，models.dev 原站兜底：
// (`https://www.js-bridge.com/api/models` → `https://models.dev/api.json`，开源模型目录，
// sst 维护、opencode 同源)：
//
//   - 启动期 `init()` 后台线程拉一份，落盘到
//     `~/Library/Caches/cc-sessions-viewer/model-pricing-v3.json`（24h TTL）。
//   - `lookup()` 只查这一份内存表 —— 没拉到 / 没命中就返回 None（成本按 $0 计）。
//   - 前端通过 `pricing_status` Tauri 命令读 `status()`，决定显示
//     正常 / loading / error placeholder。
//
// 历史背景：之前有一份 hardcoded `PRICING` 兜底表，但每次有新模型上市都要发版改代码，
// 违背了「remote-driven 配置」的初衷。后来用 LiteLLM 上游，但它 PR 驱动、新模型
// 常滞后数天（Fable 5 发布当天 models.dev 已收录而 LiteLLM 没有），于是切到
// models.dev —— 按 provider 分组、裸模型 ID、价格直接是 $/MTok 且含 cache 两档。
// 现在这份模块只剩两件事：
//   1. 把 CLI 里的"花式"模型名归一成上游的 canonical key（去 @pin / 日期 / provider，
//      `claude-opus-4.7` → `claude-opus-4-7` 这种别名）
//   2. 把 canonical 模型名转成展示名（"Opus 4.7" / "GPT-5.3 Codex"），
//      用通用规则推导而非维护映射表 —— 新版本零改动也能渲染对。

use crate::types::UsageSummary;
use once_cell::sync::OnceCell;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelCosts {
    /// $/token —— 不是 $/Mtok（models.dev 原始是 $/MTok，入表时已 ÷1e6），
    /// 乘 token 数直接得 USD。
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    /// 模型上下文窗口大小（limit.context），单位：tokens。0 = 上游没列出。
    /// 计费完全不用这个字段；纯展示给 PricingView 的 CONTEXT 列。
    #[serde(default)]
    pub context: u32,
}

// ---------- 价格表查询 ----------

/// 找 model 在动态表里的价格。归一后再前缀匹配，未知返回 None。
pub fn lookup(model: &str) -> Option<ModelCosts> {
    if model.is_empty() {
        return None;
    }
    let cell = REMOTE_PRICING.get()?;
    let table = cell.read().ok()?;
    if table.is_empty() {
        return None;
    }

    // 1) 候选键：原名 / 剥 @+date 后的整名 / 去掉任意层 provider 前缀后的模型 ID。
    let mut with_prefix = model.to_string();
    if let Some(pos) = with_prefix.find('@') {
        with_prefix.truncate(pos);
    }
    if let Some(s) = strip_trailing_yyyymmdd(&with_prefix) {
        with_prefix = s;
    }
    // Provider prefixes are transport metadata for known providers. Keep
    // user-owned/local endpoints isolated unless the table has an exact row.
    let canon = if has_isolated_provider_prefix(&with_prefix) {
        None
    } else {
        Some(resolve_alias(model_id_without_provider(&with_prefix)))
    };

    for key in [model, with_prefix.as_str()] {
        if let Some(v) = table.get(key) {
            return Some(*v);
        }
    }
    let canon = canon?;

    // 2) 前缀匹配：以 canonical 为目标，找表中最长且形如 `<canon>` 或 `<canon>-...` 的 key。
    //    `gpt-5-mini` 命中表里的 `gpt-5-mini`，不会塌成 `gpt-5`；
    //    未知的 `claude-opus-4-9` 也能塌到 `claude-opus-4` 的同系列上。
    let mut best: Option<(usize, ModelCosts)> = None;
    for (k, v) in table.iter() {
        if (canon == *k || canon.starts_with(&format!("{k}-")))
            && best.is_none_or(|(blen, _)| k.len() > blen)
        {
            best = Some((k.len(), *v));
        }
    }
    best.map(|(_, v)| v)
}

/// 严格价格查询：只接受精确 ID 及可证明安全的规范化形式，不做系列前缀推断。
///
/// Grok 会允许用户配置自定义 endpoint/model。普通 provider 前缀（包括多级网关）
/// 只是传输元数据，可以按末段模型 ID 规范化；`custom/`、`ollama/`、`local/`
/// 及其多级前缀不能因为末段看起来像官方模型就套用官方价格。精确查询失败后，
/// 额外允许 `*-free` 去后缀查其基础模型。
pub fn lookup_strict(model: &str) -> Option<ModelCosts> {
    if model.is_empty() {
        return None;
    }
    let cell = REMOTE_PRICING.get()?;
    let table = cell.read().ok()?;
    if table.is_empty() {
        return None;
    }

    let mut candidates = Vec::with_capacity(4);
    candidates.push(model.to_string());

    let mut unpinned = model.to_string();
    if let Some(pos) = unpinned.find('@') {
        unpinned.truncate(pos);
    }
    if let Some(stripped) = strip_trailing_yyyymmdd(&unpinned) {
        unpinned = stripped;
    }
    if !candidates.contains(&unpinned) {
        candidates.push(unpinned.clone());
    }

    // Provider prefixes are transport metadata, not part of the model ID.
    // Match `deepseek/deepseek-v4-flash`, `gateway/foo/model`, etc. against
    // the same source/opencode price row. Keep custom endpoints isolated so a
    // user-owned `custom/grok-*` model cannot inherit an official price.
    let custom_endpoint = has_isolated_provider_prefix(&unpinned);
    if !custom_endpoint {
        let canon = resolve_alias(model_id_without_provider(&unpinned));
        if !candidates.contains(&canon) {
            candidates.push(canon);
        }
    }

    for key in &candidates {
        if let Some(value) = table.get(key) {
            return Some(*value);
        }
    }
    for key in &candidates {
        if let Some(base) = key.strip_suffix("-free") {
            if let Some(value) = table.get(base) {
                return Some(*value);
            }
        }
    }
    None
}

/// 按 usage 算这次调用的美元成本。找不到模型时用 Claude 4.6-4.8 均价兜底。
///
/// `reasoning_output_tokens` 按 output 单价计费。OpenAI 的 o-系列 / GPT-5
/// 把推理 token 单独列出来（hidden chain-of-thought），但仍按 output rate 收钱；
/// codeburn 同样做法。Anthropic 的 extended thinking 把推理 token 直接计进
/// `output_tokens`、`reasoning_output_tokens` 始终为 0，所以这里加一项不会双扣。
///
/// `cache_creation_1h_input_tokens` 是 `cache_creation_input_tokens` 的子集，按 Anthropic
/// 价目额外再算一遍（5min 价目 → 1h tier = 1.6× 5min，跟 codeburn 同步）。
/// `5m × cw + 1h × cw × 1.6` = `(total - 1h) × cw + 1h × cw × 1.6`
/// = `total × cw + 1h × cw × 0.6`，所以这里 total 加一份 0.6× 的 1h 子集即可。
const ONE_HOUR_CACHE_WRITE_MULTIPLIER_OVER_5MIN: f64 = 1.6;
pub fn cost_usd(model: &str, usage: &UsageSummary) -> f64 {
    let Some(c) = lookup(model).or_else(fallback_costs) else {
        return 0.0;
    };
    calculate_cost(c, usage)
}

/// 严格计算成本。`None` 表示价格缺失；`Some(0.0)` 表示价格已知且为免费，调用方
/// 必须保留这个区别。Grok 统计只使用这个入口，永远不会落入 Claude 均价 fallback。
pub fn cost_usd_strict(model: &str, usage: &UsageSummary) -> Option<f64> {
    lookup_strict(model).map(|costs| calculate_cost(costs, usage))
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GrokPricedCost {
    pub cost_usd: f64,
    /// true = 模型本身未命中，使用 models.dev 中最新的纯官方旗舰 Grok 价格估算。
    pub estimated: bool,
}

/// Grok 专用计价：精确、安全规范化和 `-free` alias 优先；仍未命中时，以价格表中
/// 版本最高的纯官方旗舰型号（例如当前的 `grok-4.6`）作为动态兜底。
///
/// 该兜底只估算第三方 endpoint 的 token 成本，调用方必须把 `estimated=true` 传到 UI；
/// 它不代表代理商实际账单，也永远不会使用 Claude 或其它 provider 的价格。
pub fn cost_usd_grok(model: &str, usage: &UsageSummary) -> Option<GrokPricedCost> {
    if let Some(costs) = lookup_strict(model) {
        return Some(GrokPricedCost {
            cost_usd: calculate_cost(costs, usage),
            estimated: false,
        });
    }
    latest_plain_grok_costs().map(|costs| GrokPricedCost {
        cost_usd: calculate_cost(costs, usage),
        estimated: true,
    })
}

fn latest_plain_grok_costs() -> Option<ModelCosts> {
    let cell = REMOTE_PRICING.get()?;
    let table = cell.read().ok()?;
    table
        .iter()
        .filter(|(name, _)| is_plain_grok_model(name))
        .max_by(|(a, _), (b, _)| {
            version_tuple(a)
                .cmp(&version_tuple(b))
                .then_with(|| a.cmp(b))
        })
        .map(|(_, costs)| *costs)
}

fn is_plain_grok_model(name: &str) -> bool {
    name.strip_prefix("grok-").is_some_and(|version| {
        !version.is_empty()
            && version
                .split('.')
                .all(|segment| !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()))
    })
}

fn calculate_cost(c: ModelCosts, usage: &UsageSummary) -> f64 {
    let safe = |n: u64| n as f64;
    let one_h_extra = ONE_HOUR_CACHE_WRITE_MULTIPLIER_OVER_5MIN - 1.0; // 0.6
    safe(usage.input_tokens) * c.input
        + safe(usage.output_tokens) * c.output
        + safe(usage.reasoning_output_tokens) * c.output
        + safe(usage.cache_creation_input_tokens) * c.cache_write
        + safe(usage.cache_creation_1h_input_tokens) * c.cache_write * one_h_extra
        + safe(usage.cache_read_input_tokens) * c.cache_read
}

/// Unknown models fall back to the average price of Claude Sonnet 4.6, Opus 4.7,
/// and Opus 4.8 from the remote table. Returns None only if the table is empty or
/// none of the three anchors are present.
fn fallback_costs() -> Option<ModelCosts> {
    let cell = REMOTE_PRICING.get()?;
    let table = cell.read().ok()?;
    const ANCHORS: &[&str] = &["claude-sonnet-4-6", "claude-opus-4-7", "claude-opus-4-8"];
    let mut sum_in = 0.0_f64;
    let mut sum_out = 0.0_f64;
    let mut sum_cw = 0.0_f64;
    let mut sum_cr = 0.0_f64;
    let mut count = 0u32;
    for name in ANCHORS {
        if let Some(c) = table.get(*name) {
            sum_in += c.input;
            sum_out += c.output;
            sum_cw += c.cache_write;
            sum_cr += c.cache_read;
            count += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let n = count as f64;
    Some(ModelCosts {
        input: sum_in / n,
        output: sum_out / n,
        cache_write: sum_cw / n,
        cache_read: sum_cr / n,
        context: 0,
    })
}

// ---------- 名称归一 ----------

/// 别名表 —— 把 CLI 里多写的"花式"名映射到上游 canonical key。
/// 上游都是 dash 形式 (`claude-opus-4-7`)，CLI 偶尔写 dot 形式或加 mode 后缀。
const ALIASES: &[(&str, &str)] = &[
    ("claude-opus-4.8", "claude-opus-4-8"),
    ("claude-opus-4.7", "claude-opus-4-7"),
    ("claude-opus-4.6", "claude-opus-4-6"),
    ("claude-opus-4.5", "claude-opus-4-5"),
    ("claude-sonnet-4.6", "claude-sonnet-4-6"),
    ("claude-sonnet-4.5", "claude-sonnet-4-5"),
    ("claude-haiku-4.5", "claude-haiku-4-5"),
    ("gpt-5-fast", "gpt-5"),
    ("gpt-5.2-low", "gpt-5"),
];

/// 去掉 `@xxx` pin、`-YYYYMMDD` 日期段、provider/ 前缀。
fn canonical(model: &str) -> String {
    let mut s = model.to_string();
    if let Some(pos) = s.find('@') {
        s.truncate(pos);
    }
    if let Some(stripped) = strip_trailing_yyyymmdd(&s) {
        s = stripped;
    }
    if let Some(pos) = s.find('/') {
        s = s[pos + 1..].to_string();
    }
    s
}

fn strip_trailing_yyyymmdd(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    if bytes.len() < 9 {
        return None;
    }
    let tail = &bytes[bytes.len() - 8..];
    if tail.iter().all(|b| b.is_ascii_digit()) && bytes[bytes.len() - 9] == b'-' {
        return Some(s[..bytes.len() - 9].to_string());
    }
    None
}

fn resolve_alias(name: &str) -> String {
    for (k, v) in ALIASES {
        if *k == name {
            return (*v).to_string();
        }
    }
    name.to_string()
}

fn model_id_without_provider(model: &str) -> &str {
    model.rsplit('/').next().unwrap_or(model)
}

/// `custom`, `ollama`, and `local` identify user-owned endpoints. They may
/// appear after a gateway prefix (`gateway/custom/model`) and must not inherit
/// an official provider's price by suffix matching.
fn has_isolated_provider_prefix(model: &str) -> bool {
    model.split('/').rev().skip(1).any(|provider| {
        provider.eq_ignore_ascii_case("custom")
            || provider.eq_ignore_ascii_case("ollama")
            || provider.eq_ignore_ascii_case("local")
    })
}

// ---------- 动态层（models.dev 数据镜像 + 原站兜底） ----------

// js-bridge 是国内更稳定的 models.dev 镜像；响应格式和原站保持一致。请求、HTTP
// 状态、响应正文或结构解析任一环节异常时，立刻回退到 models.dev 原站。
const JS_BRIDGE_MODELS_URL: &str = "https://www.js-bridge.com/api/models";
const MODELS_DEV_URL: &str = "https://models.dev/api.json";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;
// 注意：换源时连文件名一起换（旧 litellm-pricing.json 直接弃用），
// 避免新解析逻辑去读旧格式缓存。
// v3 changes the primary upstream to js-bridge. Reusing a fresh v2 cache would delay the source
// change for up to 24 hours, so deliberately refresh once after upgrade.
const CACHE_FILE_NAME: &str = "model-pricing-v3.json";

static REMOTE_PRICING: OnceCell<RwLock<HashMap<String, ModelCosts>>> = OnceCell::new();
static IS_FETCHING: AtomicBool = AtomicBool::new(false);
/// 最近一次拉取失败的描述。Some = 上次拉失败；None = 上次成功 / 还没拉过。
static LAST_FETCH_ERROR: OnceCell<Mutex<Option<String>>> = OnceCell::new();

/// 前端 Settings / Stats 用：当前价格表加载情况，决定渲染 loading / error / 正常。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PricingStatus {
    /// 内存表有数据（可能是已过期的旧缓存，但至少能查）
    pub loaded: bool,
    /// 当前正在跑网络拉取
    pub fetching: bool,
    /// 上次拉取失败时的错误描述；成功 / 还没拉过都是 None
    pub last_error: Option<String>,
    /// 表里有多少条
    pub model_count: usize,
}

/// 前端「模型实时价格」窗口要展示的单条记录。`family` 用来在 UI 上分 tab
/// （Claude / Codex / Grok / ...），名字按上游原始 key（用户已熟悉的标识符）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PricingEntry {
    pub name: String,
    pub family: &'static str, // "claude" | "codex" | "grok" | "agy" | "opencode"
    pub input: f64,
    pub output: f64,
    pub cache_write: f64,
    pub cache_read: f64,
    /// 上下文窗口 tokens（limit.context）；0 表示上游没列出。
    pub context: u32,
}

/// 抽取 model name 里所有"版本号样的小数字"，按出现顺序组成 tuple 用于比较。
/// 8 位数字识别为日期 pin（`-20241022`），不进版本元组 —— 否则
/// `claude-3-haiku-20240307` 的版本会变成 (3, 20240307) 比 `claude-3-5-haiku` 的
/// (3, 5) 还"大"，旧 haiku 就跑前面了。
///
/// 这套规则的关键诉求："4.8 > 4.7 > 4 > 3.7 > 3.5"，**不受 tier 名（opus/sonnet/
/// haiku/flash/pro）影响** —— 如果按字典序，`claude-sonnet-4-5` 会跑到
/// `claude-opus-4-8` 前面（"sonnet" > "opus"），但 opus-4-8 才是最新版本。
fn version_tuple(name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let flush = |buf: &mut String, out: &mut Vec<u32>| {
        if buf.is_empty() {
            return;
        }
        if buf.len() < 8 {
            if let Ok(n) = buf.parse::<u32>() {
                if n < 1000 {
                    out.push(n);
                }
            }
        }
        buf.clear();
    };
    for c in name.chars() {
        if c.is_ascii_digit() {
            buf.push(c);
        } else {
            flush(&mut buf, &mut out);
        }
    }
    flush(&mut buf, &mut out);
    out
}

/// 抽取 `-YYYYMMDD` / `@YYYYMMDD` 形态的日期 pin（8 位连续数字）。同 version_tuple
/// 同源 —— 都是把数字段挑出来，但这里只挑 8 位的当日期看。多个日期段时取最后一个。
fn pin_date(name: &str) -> Option<u32> {
    let mut buf = String::new();
    let mut latest: Option<u32> = None;
    let try_flush = |buf: &mut String, latest: &mut Option<u32>| {
        if buf.len() == 8 {
            if let Ok(n) = buf.parse::<u32>() {
                *latest = Some(n);
            }
        }
        buf.clear();
    };
    for c in name.chars() {
        if c.is_ascii_digit() {
            buf.push(c);
        } else {
            try_flush(&mut buf, &mut latest);
        }
    }
    try_flush(&mut buf, &mut latest);
    latest
}

/// 展示各已接入 CLI 用户实际会跑的聊天模型。入表时已限定原生 provider 和
/// opencode 白名单，这里的前缀过滤是第二道防线（顺带踢掉 embedding 等非 chat 条目）。
///
/// 排序：先 family（claude → codex），再按"版本号自然顺序倒序"
/// —— 最新型号在前。`claude-opus-4-8` > `claude-opus-4-7` > `claude-opus-4` > `claude-3-7-sonnet`，
/// 同名带日期后缀（`-20241022`）的具体版本 > 不带的"latest alias"。
pub fn list_for_ui() -> Vec<PricingEntry> {
    let Some(cell) = REMOTE_PRICING.get() else {
        return Vec::new();
    };
    let Ok(table) = cell.read() else {
        return Vec::new();
    };
    let mut out: Vec<PricingEntry> = Vec::with_capacity(64);
    for (raw_name, costs) in table.iter() {
        // 防御：带 provider 前缀 / `@default` deployment alias 的 key 不展示。
        // models.dev 的三家 provider 都是裸 ID，正常拉取不会出现这两种；留着
        // 是防上游格式漂移（测试 with_remote 塞的 key 也可能带）。
        if raw_name.contains('/') || raw_name.ends_with("@default") {
            continue;
        }
        let lower = raw_name.to_ascii_lowercase();
        // Codex CLI / Claude Code 用户实际只跑 chat completion 那条线 —— 把跟它不相干的
        // GPT 变体过滤掉。否则像 `gpt-oss-120b`（开源权重，名字里 "120" 是参数量不是版本）
        // 会被 version_tuple 解析成 [120]，排到 gpt-5 [5] 前面 —— 用户反馈过这个。
        // 一并干掉 image / audio / realtime / transcribe / search-preview，那都是
        // 独立 API 不是 chat 模型；以及 `gpt-35-*` Azure 命名重复（`gpt-3.5-*` 已在表里）。
        let is_noise = lower.contains("gpt-oss")
            || lower.contains("oss:")
            || lower.contains("-image")
            || lower.contains("-audio")
            || lower.contains("-realtime")
            || lower.contains("-transcribe")
            || lower.contains("-search-preview")
            || lower.contains("-search-api")
            || lower.contains("-video")
            || lower.contains("imagine-")
            || lower.starts_with("gpt-35-")
            || lower == "gpt-35";
        if is_noise {
            continue;
        }
        let family: &'static str = if lower.starts_with("claude-") {
            "claude"
        } else if lower.starts_with("codex-")
            || lower.contains("-codex")
            || lower.starts_with("gpt-")
            || lower.starts_with("o1-")
            || lower.starts_with("o3-")
            || lower.starts_with("o4-")
            || lower == "o1"
            || lower == "o3"
            || lower == "o4"
        {
            "codex"
        } else if lower.starts_with("grok-") {
            "grok"
        } else if lower.starts_with("gemini-") {
            "agy"
        } else if lower.starts_with("text-embedding-")
            || lower.starts_with("tts-")
            || lower.starts_with("whisper-")
            || lower.starts_with("dall-e-")
        {
            continue; // 非聊天模型，不进价格表
        } else {
            "opencode"
        };
        out.push(PricingEntry {
            name: raw_name.clone(),
            family,
            input: costs.input,
            output: costs.output,
            cache_write: costs.cache_write,
            cache_read: costs.cache_read,
            context: costs.context,
        });
    }
    let family_rank = |f: &str| match f {
        "claude" => 0,
        "codex" => 1,
        "grok" => 2,
        "agy" => 3,
        "opencode" => 4,
        _ => 9,
    };
    // 同 family 内："版本号倒序" 主键 + "naked > 日期 pin 倒序" tiebreak。
    // None pin（没有日期后缀，多半是"latest alias"）转 u32::MAX，保证它排在所有
    // 日期版本之前。
    let pin_key = |name: &str| pin_date(name).unwrap_or(u32::MAX);
    out.sort_by(|a, b| {
        family_rank(a.family)
            .cmp(&family_rank(b.family))
            .then_with(|| version_tuple(&b.name).cmp(&version_tuple(&a.name)))
            .then_with(|| pin_key(&b.name).cmp(&pin_key(&a.name)))
            // 极端 tie（同版本同 pin）：lex 字典正序保证 deterministic。
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

pub fn status() -> PricingStatus {
    let (loaded, model_count) = REMOTE_PRICING
        .get()
        .and_then(|c| c.read().ok())
        .map(|t| (!t.is_empty(), t.len()))
        .unwrap_or((false, 0));
    let last_error = LAST_FETCH_ERROR
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| g.clone());
    PricingStatus {
        loaded,
        fetching: IS_FETCHING.load(Ordering::Relaxed),
        last_error,
        model_count,
    }
}

#[derive(Serialize, Deserialize)]
struct CacheFile {
    timestamp_secs: u64,
    data: HashMap<String, ModelCosts>,
}

/// 应用启动时调一次：
///   - 命中本地 cache 且新鲜（<24h）：装进内存就 return。
///   - 缺失 / 过期：先把旧 cache（如有）灌进内存兜着，再后台线程拉远端覆写。
///   - 拉失败：内存表保持空（或保留旧 cache 的内容），错误记进 `LAST_FETCH_ERROR`，
///     前端可读 `pricing_status` 显示 error placeholder。
///
/// 永不阻塞、永不 panic。
pub fn init() {
    REMOTE_PRICING.get_or_init(|| RwLock::new(HashMap::new()));
    LAST_FETCH_ERROR.get_or_init(|| Mutex::new(None));

    // 清理 LiteLLM 时代的旧缓存文件 —— 换源后文件名已换，旧文件永远不会再被读。
    if let Some(path) = cache_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_file(dir.join("litellm-pricing.json"));
        }
    }

    let mut cache_is_fresh = false;
    if let Some((fresh, table)) = load_from_cache() {
        cache_is_fresh = fresh;
        write_table(table);
    }

    if cache_is_fresh {
        return;
    }

    // 后台拉一次 —— 不阻塞 setup hook。
    std::thread::spawn(|| {
        run_fetch();
    });
}

/// 同步强制刷新（前端 Settings 「立即刷新模型价格」按钮用）。
/// 成功返回入表条数；失败返回错误字符串。
pub fn refresh_blocking() -> Result<usize, String> {
    REMOTE_PRICING.get_or_init(|| RwLock::new(HashMap::new()));
    LAST_FETCH_ERROR.get_or_init(|| Mutex::new(None));

    match fetch_and_store() {
        Ok(table) => {
            let n = table.len();
            set_last_error(None);
            write_table(table);
            Ok(n)
        }
        Err(e) => {
            set_last_error(Some(e.clone()));
            Err(e)
        }
    }
}

/// 清除本地价格缓存和当前内存表，然后在后台重新拉取最新价格。
/// 用于“恢复默认设置”，避免价格页继续显示重置前的 24h 缓存。
pub fn clear_cache_and_refresh() {
    REMOTE_PRICING.get_or_init(|| RwLock::new(HashMap::new()));
    LAST_FETCH_ERROR.get_or_init(|| Mutex::new(None));

    if let Some(path) = cache_path() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_file_name("models-dev-pricing-v2.json"));
        let _ = std::fs::remove_file(path.with_file_name("models-dev-pricing.json"));
        let _ = std::fs::remove_file(path.with_file_name("litellm-pricing.json"));
    }
    write_table(HashMap::new());
    set_last_error(None);

    std::thread::spawn(run_fetch);
}

fn run_fetch() {
    if IS_FETCHING.swap(true, Ordering::SeqCst) {
        // 已经有一次在跑，不重复发车
        return;
    }
    let result = fetch_and_store();
    match result {
        Ok(table) => {
            set_last_error(None);
            write_table(table);
        }
        Err(e) => set_last_error(Some(e)),
    }
    IS_FETCHING.store(false, Ordering::SeqCst);
}

fn write_table(table: HashMap<String, ModelCosts>) {
    if let Some(cell) = REMOTE_PRICING.get() {
        if let Ok(mut w) = cell.write() {
            *w = table;
        }
    }
}

fn set_last_error(err: Option<String>) {
    if let Some(m) = LAST_FETCH_ERROR.get() {
        if let Ok(mut g) = m.lock() {
            *g = err;
        }
    }
}

/// 磁盘缓存路径：`<cache_dir>/cc-sessions-viewer/model-pricing-v3.json`。
/// macOS: `~/Library/Caches/...`；Linux: `~/.cache/...`；Windows: `%LOCALAPPDATA%\...`。
fn cache_path() -> Option<std::path::PathBuf> {
    let base = dirs::cache_dir()?;
    Some(base.join("cc-sessions-viewer").join(CACHE_FILE_NAME))
}

/// 从磁盘读 cache。返回 `(is_fresh, table)`；过期但能解出的 table 也返回（启动期兜底）。
fn load_from_cache() -> Option<(bool, HashMap<String, ModelCosts>)> {
    let path = cache_path()?;
    if let Some(cached) = read_cache_file(&path) {
        return Some(cached);
    }
    // 旧缓存只作为离线启动兜底：强制后台按新优先级刷新，成功后只写 v3。
    for old_name in ["models-dev-pricing-v2.json", "models-dev-pricing.json"] {
        if let Some((_, table)) = read_cache_file(&path.with_file_name(old_name)) {
            return Some((false, table));
        }
    }
    None
}

fn read_cache_file(path: &Path) -> Option<(bool, HashMap<String, ModelCosts>)> {
    let raw = std::fs::read_to_string(path).ok()?;
    let parsed: CacheFile = serde_json::from_str(&raw).ok()?;
    let age = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs().saturating_sub(parsed.timestamp_secs))
        .unwrap_or(u64::MAX);
    Some((age < CACHE_TTL_SECS, parsed.data))
}

/// 先 GET js-bridge 镜像；任一步失败即 GET models.dev 原站兜底。成功后才写盘，
/// 因而临时网络问题不会覆盖仍可用的旧缓存。
fn fetch_and_store() -> Result<HashMap<String, ModelCosts>, String> {
    let sources = pricing_sources();
    let mut failures = Vec::with_capacity(sources.len());
    for (name, url) in sources {
        match fetch_source(url) {
            Ok(body) => match parse_models_dev_json(&body).filter(|table| !table.is_empty()) {
                Some(table) => {
                    save_to_cache(&table);
                    return Ok(table);
                }
                None => failures.push(format!("{name}: parse: empty or malformed")),
            },
            Err(error) => failures.push(format!("{name}: {error}")),
        }
    }
    Err(format!(
        "all pricing sources failed ({})",
        failures.join("; ")
    ))
}

fn pricing_sources() -> [(&'static str, &'static str); 2] {
    [
        ("js-bridge", JS_BRIDGE_MODELS_URL),
        ("models.dev", MODELS_DEV_URL),
    ]
}

fn fetch_source(url: &str) -> Result<String, String> {
    let body = ureq::get(url)
        .timeout(Duration::from_secs(20))
        .call()
        .map_err(|e| format!("network: {e}"))?
        .into_string()
        .map_err(|e| format!("read body: {e}"))?;
    Ok(body)
}

fn save_to_cache(table: &HashMap<String, ModelCosts>) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let cache = CacheFile {
        timestamp_secs: now,
        data: table.clone(),
    };
    if let Ok(serialized) = serde_json::to_string(&cache) {
        let _ = std::fs::write(path, serialized);
    }
}

/// 解析 models.dev 的根 JSON：`{ <provider>: { models: { <id>: { cost, limit, … } } } }`。
///
/// 两步：
///   1. anthropic / openai / xai / google —— 原生 CLI 的模型，直接入表。
///   2. opencode 专属模型（白名单，来源 opencode 官方文档）：逐个在各厂商直连
///      provider 里查价格（官方价为准），查不到的在 `opencode` provider 里兜底。
pub(crate) fn parse_models_dev_json(body: &str) -> Option<HashMap<String, ModelCosts>> {
    const NATIVE_PROVIDERS: &[&str] = &["anthropic", "openai", "xai", "google"];
    const OPENCODE_DIRECT: &[&str] = &[
        "deepseek",
        "kimi-for-coding",
        "minimax",
        "opencode-go",
        "zhipuai",
        "xiaomi",
        "xai",
        "alibaba-cn",
    ];
    // opencode 文档列出的专属模型和别名（合并 go + zen 两份文档去重）。
    // 来源：https://opencode.ai/docs/zh-cn/go/#模型
    //       https://opencode.ai/docs/zh-cn/zen/#定价
    const OPENCODE_MODELS: &[&str] = &[
        "big-pickle",
        "deepseek-v4-flash",
        "deepseek-v4-flash-free",
        "deepseek-v4-pro",
        "gemini-3.1-pro",
        "glm-5",
        "glm-5.1",
        "glm-5.2",
        "glm-5.3",
        "glm-5-free",
        "gpt-5.2-codex",
        "grok-build-0.1",
        "hy3-free",
        "hy3-preview-free",
        "kimi-k2.5",
        "kimi-k2.5-free",
        "kimi-k2.6",
        "kimi-k2.7-code",
        "kimi-k3",
        "laguna-s-2.1-free",
        "ling-2.6-flash-free",
        "ling-3.0-flash-free",
        "ling-3.0-tiny-free",
        "longcat-2.0-free",
        "mimo-v2.5",
        "mimo-v2.5-free",
        "mimo-v2.5-pro",
        "mimo-v2-omni-free",
        "mimo-v2-pro-free",
        "minimax-m2.5",
        "minimax-m2.5-free",
        "minimax-m2.7",
        "minimax-m3",
        "minimax-m3-free",
        "muse-spark-1.2",
        "muse-spark-1.2-contributor-free",
        "nemotron-3.5-lightning-free",
        "nemotron-3-super-free",
        "nemotron-3-ultra-free",
        "north-mini-code-free",
        "qwen3.5-plus",
        "qwen3.6-plus",
        "qwen3.6-plus-free",
        "qwen3.7-max",
        "qwen3.7-plus",
        "qwen3.8-max",
        "ring-2.6-1t-free",
        "trinity-large-preview-free",
        "x-preview-f-free",
    ];

    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let root = value.as_object()?;
    let mut out: HashMap<String, ModelCosts> = HashMap::with_capacity(128);

    // Step 1: 原生 CLI provider
    for prov in NATIVE_PROVIDERS {
        let Some(models) = root
            .get(*prov)
            .and_then(|p| p.get("models"))
            .and_then(|m| m.as_object())
        else {
            continue;
        };
        for (name, entry) in models.iter() {
            let Some(costs) = parse_models_dev_entry(entry) else {
                continue;
            };
            out.insert(name.clone(), costs);
        }
    }

    // Step 2: opencode 专属模型——各厂商直连价优先，opencode provider 兜底
    let mut direct_all: HashMap<String, ModelCosts> = HashMap::new();
    for prov in OPENCODE_DIRECT {
        if let Some(models) = root
            .get(*prov)
            .and_then(|p| p.get("models"))
            .and_then(|m| m.as_object())
        {
            for (name, entry) in models.iter() {
                if let Some(costs) = parse_models_dev_entry(entry) {
                    direct_all.insert(name.to_ascii_lowercase(), costs);
                }
            }
        }
    }
    let oc_models = root
        .get("opencode")
        .and_then(|p| p.get("models"))
        .and_then(|m| m.as_object());

    for id in OPENCODE_MODELS {
        let lower = id.to_ascii_lowercase();
        // "-free" 是 opencode 的免费套餐标签，底层模型有厂商官方价——去掉后缀再查。
        let base = lower.strip_suffix("-free").unwrap_or(&lower);
        if let Some(costs) = direct_all.get(base).or_else(|| direct_all.get(&lower)) {
            out.insert(id.to_string(), *costs);
        } else if let Some(entry) = oc_models.and_then(|m| m.get(*id)) {
            if let Some(costs) = parse_models_dev_entry(entry) {
                out.insert(id.to_string(), costs);
            }
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
}

fn parse_models_dev_entry(v: &serde_json::Value) -> Option<ModelCosts> {
    let obj = v.as_object()?;
    // cost 单位是 $/MTok，内存表统一成 $/token（÷1e6）。没有 cost 的条目
    // （gpt-image / gemma 开源权重这类）直接跳过 —— CLI 也跑不到它们。
    const PER_MTOK: f64 = 1e-6;
    let cost = obj.get("cost").and_then(|c| c.as_object())?;
    let input = cost.get("input").and_then(|x| x.as_f64())? * PER_MTOK;
    let output = cost.get("output").and_then(|x| x.as_f64())? * PER_MTOK;
    // 缺 cache 字段沿用旧约定兜底：write = input×1.25，read = input×0.1。
    let cw = cost
        .get("cache_write")
        .and_then(|x| x.as_f64())
        .map(|x| x * PER_MTOK);
    let cr = cost
        .get("cache_read")
        .and_then(|x| x.as_f64())
        .map(|x| x * PER_MTOK);
    // limit.context = 上下文窗口 tokens。越界（> u32::MAX）钳到 0 当未知处理。
    let context = obj
        .get("limit")
        .and_then(|l| l.get("context"))
        .and_then(|x| x.as_u64())
        .map(|n| u32::try_from(n).unwrap_or(0))
        .unwrap_or(0);
    Some(ModelCosts {
        input,
        output,
        cache_write: cw.unwrap_or(input * 1.25),
        cache_read: cr.unwrap_or(input * 0.1),
        context,
    })
}

// ---------- 展示名 ----------

/// 不规则展示名覆盖表 —— 只放 `derive_name` 推不出的：旧式 claude-3.x（家族在尾）、
/// o 系小写、以及 codex 独立名。规则命名的现代模型一律走 `derive_name`，新版本免改表。
const SHORT_OVERRIDE: &[(&str, &str)] = &[
    ("claude-3-7-sonnet", "Sonnet 3.7"),
    ("claude-3-5-sonnet", "Sonnet 3.5"),
    ("claude-3-5-haiku", "Haiku 3.5"),
    ("claude-3-opus", "Opus 3"),
    ("codex-mini-latest", "Codex Mini"),
    ("o4-mini", "o4-mini"),
    ("o3-mini", "o3-mini"),
    ("o3-pro", "o3-pro"),
    ("o3", "o3"),
    ("o1-mini", "o1-mini"),
    ("o1-pro", "o1-pro"),
    ("o1", "o1"),
];

/// 模型友好展示名 —— "Opus 4.7" / "Sonnet 4.6" / "GPT-5.3 Codex" 等。
/// 顺序：通用推导 → 不规则覆盖表 → 原样 canonical。
pub fn short_name(model: &str) -> String {
    let canon = resolve_alias(&canonical(model));
    if let Some(name) = derive_name(&canon) {
        return name;
    }
    let mut sorted: Vec<&(&str, &str)> = SHORT_OVERRIDE.iter().collect();
    sorted.sort_by_key(|(k, _)| std::cmp::Reverse(k.len()));
    for (k, label) in sorted {
        if canon.starts_with(*k) {
            return (*label).to_string();
        }
    }
    canon
}

/// 从结构化模型 ID 推导展示名。只在能自信解析时返回 Some；不规则名返回 None 交给覆盖表。
///   claude-<family>-<major>[-<minor>...]  -> "Opus 4.8" / "Sonnet 4"
///   gpt-<ver>[-suffix...]                 -> "GPT-5.3 Codex" / "GPT-4o Mini"
fn derive_name(canon: &str) -> Option<String> {
    if let Some(rest) = canon.strip_prefix("claude-") {
        let segs: Vec<&str> = rest.split('-').collect();
        let family = match *segs.first()? {
            "opus" => "Opus",
            "sonnet" => "Sonnet",
            "haiku" => "Haiku",
            _ => return None,
        };
        let ver = &segs[1..];
        if ver.is_empty()
            || !ver
                .iter()
                .all(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        {
            return None;
        }
        return Some(format!("{family} {}", ver.join(".")));
    }
    if let Some(rest) = canon.strip_prefix("gpt-") {
        let segs: Vec<&str> = rest.split('-').collect();
        let ver = segs.first()?;
        if ver.is_empty() {
            return None;
        }
        let mut out = format!("GPT-{ver}");
        for s in &segs[1..] {
            out.push(' ');
            out.push_str(&title_case(s));
        }
        return Some(out);
    }
    if let Some(rest) = canon.strip_prefix("grok-") {
        let mut out = String::from("Grok Build");
        for segment in rest.split('-') {
            out.push(' ');
            out.push_str(&title_case(segment));
        }
        return Some(out);
    }
    None
}

fn title_case(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// 测试帮助：往内存表里塞一组常见模型价格，幂等。
/// 给其它模块（aggregate / agents/codex）的单元测试用 ——
/// 它们走 `cost_usd` 链路，没有这一步 hardcoded 表被砍后就全部塌成 $0。
/// 用 `Once` 保证多次调用只灌一次；其它测试若用 `with_remote` 临时 override
/// 同名 key 也安全（HashMap 直接覆盖）。
#[cfg(test)]
pub fn seed_test_prices() {
    use std::sync::Once;
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let cell = REMOTE_PRICING.get_or_init(|| RwLock::new(HashMap::new()));
        let mut w = cell.write().unwrap();
        for (name, c) in TEST_DEFAULT_PRICES {
            w.entry((*name).to_string()).or_insert(*c);
        }
    });
}

#[cfg(test)]
const TEST_DEFAULT_PRICES: &[(&str, ModelCosts)] = &[
    (
        "claude-opus-4-7",
        ModelCosts {
            input: 0.000005,
            output: 0.000025,
            cache_write: 0.00000625,
            cache_read: 0.0000005,
            context: 0,
        },
    ),
    (
        "claude-sonnet-4-6",
        ModelCosts {
            input: 0.000003,
            output: 0.000015,
            cache_write: 0.00000375,
            cache_read: 0.0000003,
            context: 0,
        },
    ),
    (
        "gpt-5",
        ModelCosts {
            input: 0.00000125,
            output: 0.00001,
            cache_write: 0.0000015625,
            cache_read: 0.000000125,
            context: 0,
        },
    ),
];

#[cfg(test)]
mod tests {
    use super::*;

    fn u(input: u64, output: u64, cw: u64, cr: u64) -> UsageSummary {
        UsageSummary {
            input_tokens: input,
            output_tokens: output,
            cache_creation_input_tokens: cw,
            cache_creation_1h_input_tokens: 0,
            cache_read_input_tokens: cr,
            reasoning_output_tokens: 0,
            total: input + output + cw + cr,
        }
    }

    #[test]
    fn pricing_sources_prefer_js_bridge_and_keep_models_dev_fallback() {
        assert_eq!(
            pricing_sources(),
            [
                ("js-bridge", "https://www.js-bridge.com/api/models"),
                ("models.dev", "https://models.dev/api.json"),
            ]
        );
    }

    /// 测试辅助：往内存 REMOTE_PRICING 塞一份"模拟拉来的"价格表，并在闭包结束后
    /// **恢复原值**（而不是一删了之）。cargo test 默认多线程，每条测试应使用
    /// 各自专属的 model key 避免互相串扰；恢复语义保证即便和 seed_test_prices
    /// 共享了 key（如 claude-opus-4-7），清理也不会把别的测试正在用的条目删掉。
    fn with_remote<F: FnOnce()>(rows: &[(&str, ModelCosts)], f: F) {
        let cell = REMOTE_PRICING.get_or_init(|| RwLock::new(HashMap::new()));
        let saved: Vec<(String, Option<ModelCosts>)>;
        {
            let mut w = cell.write().unwrap();
            saved = rows
                .iter()
                .map(|(k, v)| (k.to_string(), w.insert(k.to_string(), *v)))
                .collect();
        }
        f();
        let mut w = cell.write().unwrap();
        for (k, old) in saved {
            match old {
                Some(v) => {
                    w.insert(k, v);
                }
                None => {
                    w.remove(&k);
                }
            }
        }
    }

    fn opus_4_7_costs() -> ModelCosts {
        ModelCosts {
            input: 0.000005,
            output: 0.000025,
            cache_write: 0.00000625,
            cache_read: 0.0000005,
            context: 0,
        }
    }

    #[test]
    fn canonical_strips_pin_date_and_provider_prefix() {
        assert_eq!(
            canonical("anthropic/claude-opus-4-6@20250929"),
            "claude-opus-4-6"
        );
        assert_eq!(canonical("claude-sonnet-4-20250514"), "claude-sonnet-4");
        assert_eq!(
            canonical("openrouter/anthropic/claude-opus-4-6"),
            "anthropic/claude-opus-4-6"
        );
        // 注意：canonical 只剥第一段 provider；lookup 会按末段模型 ID 处理多级前缀。
    }

    #[test]
    fn lookup_returns_none_for_local_or_unknown() {
        // 空内存表 / 未匹配键 → None。
        assert!(lookup("llama3:8b-instruct").is_none());
        assert!(lookup("totally-made-up-model").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn cost_usd_uses_fallback_for_unknown_model() {
        seed_test_prices();
        let big = u(1_000_000, 1_000_000, 1_000_000, 1_000_000);
        let c = cost_usd("ollama/llama-3", &big);
        assert!(
            c > 0.0,
            "unknown model should use Claude 4.6-4.8 average as fallback"
        );
    }

    #[test]
    fn strict_cost_never_uses_claude_fallback_for_unknown_grok_model() {
        seed_test_prices();
        let usage = u(1_000_000, 1_000_000, 0, 0);
        assert_eq!(lookup_strict("custom/grok-private-model"), None);
        assert_eq!(cost_usd_strict("custom/grok-private-model", &usage), None);
        assert!(
            cost_usd("custom/grok-private-model", &usage) > 0.0,
            "the legacy non-strict path still demonstrates why Grok must not use it"
        );
    }

    #[test]
    fn strict_lookup_supports_xai_prefix_pin_and_free_alias() {
        let rate = ModelCosts {
            input: 2e-6,
            output: 10e-6,
            cache_write: 2.5e-6,
            cache_read: 0.2e-6,
            context: 131_072,
        };
        with_remote(&[("grok-strict-alias-test", rate)], || {
            assert_eq!(lookup_strict("grok-strict-alias-test"), Some(rate));
            assert_eq!(
                lookup_strict("xai/grok-strict-alias-test@20260818"),
                Some(rate)
            );
            assert_eq!(lookup_strict("grok-strict-alias-test-free"), Some(rate));
            assert_eq!(
                lookup_strict("custom/grok-strict-alias-test"),
                None,
                "custom endpoints must not inherit official xAI pricing"
            );
        });
    }

    #[test]
    fn strict_lookup_supports_known_direct_provider_prefixes() {
        let rate = opus_4_7_costs();
        with_remote(&[("deepseek-v4-flash", rate)], || {
            assert_eq!(lookup_strict("deepseek/deepseek-v4-flash"), Some(rate));
            assert_eq!(
                lookup_strict("gateway/deepseek/deepseek-v4-flash"),
                Some(rate)
            );
            assert_eq!(lookup_strict("custom/deepseek-v4-flash"), None);
        });
    }

    #[test]
    fn lookup_supports_multi_level_prefixes_without_leaking_local_prices() {
        let rate = opus_4_7_costs();
        with_remote(&[("deepseek-v4-flash", rate)], || {
            assert_eq!(
                lookup("gateway/deepseek/deepseek-v4-flash@20260818"),
                Some(rate)
            );
            assert_eq!(
                lookup("gateway/custom/deepseek-v4-flash"),
                None,
                "custom endpoints must not inherit official pricing"
            );
            assert_eq!(lookup("gateway/ollama/deepseek-v4-flash"), None);
            assert_eq!(lookup("gateway/local/deepseek-v4-flash"), None);
        });
    }

    #[test]
    fn strict_cost_distinguishes_known_free_from_missing_price() {
        let free = ModelCosts {
            input: 0.0,
            output: 0.0,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 32_000,
        };
        with_remote(&[("grok-known-free-test-free", free)], || {
            let usage = u(10_000, 2_000, 0, 0);
            assert_eq!(
                cost_usd_strict("grok-known-free-test-free", &usage),
                Some(0.0)
            );
            assert_eq!(cost_usd_strict("grok-missing-price-test", &usage), None);
        });
    }

    #[test]
    fn grok_cost_uses_latest_plain_official_model_as_marked_fallback() {
        let older = ModelCosts {
            input: 1e-6,
            output: 2e-6,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 131_072,
        };
        let latest = ModelCosts {
            input: 7e-6,
            output: 11e-6,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 262_144,
        };
        let special = ModelCosts {
            input: 99e-6,
            output: 99e-6,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 0,
        };
        with_remote(
            &[
                ("grok-9998", older),
                ("grok-9999", latest),
                ("grok-10000-multi-agent", special),
            ],
            || {
                let usage = u(1_000_000, 1_000_000, 0, 0);
                let exact = cost_usd_grok("grok-9998", &usage).expect("exact price");
                assert!(!exact.estimated);
                assert!((exact.cost_usd - 3.0).abs() < 1e-9);

                let fallback = cost_usd_grok("third-party/custom-model", &usage)
                    .expect("official Grok estimate");
                assert!(fallback.estimated);
                assert!((fallback.cost_usd - 18.0).abs() < 1e-9);
            },
        );
    }

    #[test]
    fn plain_grok_fallback_excludes_specialized_variants() {
        assert!(is_plain_grok_model("grok-4"));
        assert!(is_plain_grok_model("grok-4.6"));
        assert!(is_plain_grok_model("grok-4.20.1"));
        assert!(!is_plain_grok_model("grok-build-0.1"));
        assert!(!is_plain_grok_model("grok-4.20-reasoning"));
        assert!(!is_plain_grok_model("grok-4-free"));
    }

    #[test]
    fn cost_usd_bills_1h_cache_creation_at_double_5min_rate() {
        // Anthropic 的 1-hour cache write = 5-minute cache write 的 2×；codeburn
        // 也是这么算的（`safeOneHourCacheCreation × cacheWriteCostPerToken × ONE_HOUR_MULTIPLIER`）。
        // 我们之前一刀切 5min 价位，碰到长会话全 1h cache 写入会少扣一半，
        // Today 总成本被压低 ~8%。
        let rate = ModelCosts {
            input: 0.0,
            output: 0.0,
            cache_write: 0.000010,
            cache_read: 0.0,
            context: 0,
        };
        with_remote(&[("cw-tier-test", rate)], || {
            // 全 5min：1M × $10/MTok = $10
            let pure_5m = UsageSummary {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 1_000_000,
                cache_creation_1h_input_tokens: 0,
                cache_read_input_tokens: 0,
                reasoning_output_tokens: 0,
                total: 1_000_000,
            };
            // 全 1h：1M × $10/MTok × 1.6 = $16
            let pure_1h = UsageSummary {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 1_000_000,
                cache_creation_1h_input_tokens: 1_000_000,
                cache_read_input_tokens: 0,
                reasoning_output_tokens: 0,
                total: 1_000_000,
            };
            let c_5m = cost_usd("cw-tier-test", &pure_5m);
            let c_1h = cost_usd("cw-tier-test", &pure_1h);
            assert!((c_5m - 10.0).abs() < 1e-9);
            assert!(
                (c_1h - 16.0).abs() < 1e-9,
                "1h tier must cost 1.6× the 5min tier, got ${c_1h}"
            );
        });
    }

    #[test]
    fn cost_usd_bills_reasoning_tokens_at_output_rate() {
        // 回归：GPT-5 / o-系列把 hidden chain-of-thought 单独算成 reasoning_output_tokens，
        // 但仍按 output rate 收钱。旧公式漏算这一项 —— 在 reasoning ≈ 25% of output 的
        // 典型 codex 会话上，每次 call 都少扣 ~25% × output rate，整轮 Today 总成本约
        // 比 codeburn 低 4-5%。这里用纯推理负载锁住：reasoning 单独计费 = output 单独计费。
        let rate = ModelCosts {
            input: 0.0,
            output: 0.0001,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 0,
        };
        with_remote(&[("reasoning-billing-test", rate)], || {
            let pure_reasoning = UsageSummary {
                input_tokens: 0,
                output_tokens: 0,
                cache_creation_input_tokens: 0,
                cache_creation_1h_input_tokens: 0,
                cache_read_input_tokens: 0,
                reasoning_output_tokens: 1_000_000,
                total: 1_000_000,
            };
            let pure_output = UsageSummary {
                input_tokens: 0,
                output_tokens: 1_000_000,
                cache_creation_input_tokens: 0,
                cache_creation_1h_input_tokens: 0,
                cache_read_input_tokens: 0,
                reasoning_output_tokens: 0,
                total: 1_000_000,
            };
            let c_reason = cost_usd("reasoning-billing-test", &pure_reasoning);
            let c_output = cost_usd("reasoning-billing-test", &pure_output);
            assert!(
                (c_reason - 100.0).abs() < 1e-9,
                "expected $100, got ${c_reason}"
            );
            assert!(
                (c_reason - c_output).abs() < 1e-9,
                "reasoning must cost the same as output"
            );
        });
    }

    #[test]
    fn lookup_finds_canonical_key_in_dynamic_table() {
        with_remote(&[("opus-4-7-test", opus_4_7_costs())], || {
            // 装载后能查到
            let c = lookup("opus-4-7-test").expect("direct");
            assert!((c.input - 0.000005).abs() < 1e-12);
        });
        // 退场后查不到（隔离）
        assert!(lookup("opus-4-7-test").is_none());
    }

    #[test]
    fn lookup_strips_pin_and_date_against_remote_table() {
        with_remote(&[("pin-target-7", opus_4_7_costs())], || {
            let with_pin = lookup("pin-target-7@20250101").expect("pin stripped");
            let with_date = lookup("pin-target-7-20250101").expect("date stripped");
            let plain = lookup("pin-target-7").expect("direct");
            assert_eq!(with_pin, plain);
            assert_eq!(with_date, plain);
        });
    }

    // 注意：cargo test 多线程跑，REMOTE_PRICING 是全局共享表 —— 每条测试必须用
    // 没有其它测试（含 seed_test_prices / list_for_ui 排序测试）共享的专属 key，
    // 否则一边的 with_remote 清理会把另一边正在 lookup 的条目摘走，随机挂。
    // 别名测试必须用 ALIASES 里的真实 key，选了独占的 claude-opus-4-6。
    #[test]
    fn lookup_resolves_dot_alias_to_dash_form() {
        with_remote(&[("claude-opus-4-6", opus_4_7_costs())], || {
            let dot = lookup("claude-opus-4.6").expect("aliased");
            let dash = lookup("claude-opus-4-6").expect("direct");
            assert_eq!(dot, dash);
        });
    }

    #[test]
    fn lookup_strips_provider_prefix_when_canonicalizing() {
        with_remote(&[("prefix-strip-target", opus_4_7_costs())], || {
            let prefixed = lookup("anthropic/prefix-strip-target").expect("provider stripped");
            assert_eq!(prefixed, opus_4_7_costs());
        });
    }

    #[test]
    fn lookup_longest_prefix_wins_against_sibling_keys() {
        // 表里同时有 base 和子版本：未知子版本应套同系列最长前缀，不塌成 base。
        let base = ModelCosts {
            input: 100e-6,
            output: 100e-6,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 0,
        };
        let sub = ModelCosts {
            input: 1e-6,
            output: 1e-6,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 0,
        };
        with_remote(&[("xyz-base", base), ("xyz-base-special", sub)], || {
            let unknown = lookup("xyz-base-special-99").expect("longest prefix");
            assert!(
                (unknown.input - 1e-6).abs() < 1e-15,
                "got {}",
                unknown.input
            );
        });
    }

    #[test]
    fn cost_usd_uses_remote_table_when_loaded() {
        with_remote(&[("dyn-cost-test", opus_4_7_costs())], || {
            let one_million = u(1_000_000, 1_000_000, 0, 0);
            let c = cost_usd("dyn-cost-test", &one_million);
            // 1M × $5/MTok + 1M × $25/MTok = $30
            assert!((c - 30.0).abs() < 1e-6, "got {c}");
        });
    }

    #[test]
    fn parse_models_dev_json_extracts_costs_and_handles_fallbacks() {
        // 覆盖点：
        //   - $/MTok → $/token 换算（÷1e6）+ limit.context 抽取
        //   - 缺 cache_write：套兜底公式 input × 1.25
        //   - 缺 cache_read：套兜底公式 input × 0.1
        //   - 没有 cost 的条目（image / 开源权重）跳过
        //   - 非 anthropic/openai 的 provider（镜像网关）整组跳过
        let body = r#"{
            "anthropic": { "models": {
                "claude-magic-9": {
                    "cost": { "input": 10, "output": 50, "cache_read": 1, "cache_write": 12.5 },
                    "limit": { "context": 1000000, "output": 128000 }
                }
            }},
            "openai": { "models": {
                "gpt-no-cw": {
                    "cost": { "input": 2, "output": 10, "cache_read": 0.2 }
                },
                "gpt-no-cr": {
                    "cost": { "input": 4, "output": 20, "cache_write": 5 },
                    "limit": { "context": 400000 }
                },
                "gpt-image-x": { "limit": { "context": 32000 } }
            }},
            "openrouter": { "models": {
                "anthropic/claude-magic-9": { "cost": { "input": 99, "output": 99 } }
            }}
        }"#;
        let table = parse_models_dev_json(body).expect("parsed");

        let full = table.get("claude-magic-9").expect("full");
        assert!((full.input - 1e-5).abs() < 1e-15, "$10/MTok → $1e-5/token");
        assert!((full.output - 5e-5).abs() < 1e-15);
        assert!((full.cache_read - 1e-6).abs() < 1e-15);
        assert!((full.cache_write - 1.25e-5).abs() < 1e-15);
        assert_eq!(full.context, 1_000_000);

        let no_cw = table.get("gpt-no-cw").expect("no-cw");
        assert!(
            (no_cw.cache_write - 2.5e-6).abs() < 1e-15,
            "input×1.25 fallback"
        );
        assert_eq!(no_cw.context, 0, "缺 limit → context 0");

        let no_cr = table.get("gpt-no-cr").expect("no-cr");
        assert!(
            (no_cr.cache_read - 4e-7).abs() < 1e-15,
            "input×0.1 fallback"
        );
        assert_eq!(no_cr.context, 400_000);

        assert!(!table.contains_key("gpt-image-x"), "无 cost entry 跳过");
        assert!(
            !table.contains_key("anthropic/claude-magic-9"),
            "镜像 provider 整组跳过"
        );
        // openrouter 的镜像价 ($99) 不能覆盖 anthropic 官方价
        assert!((table.get("claude-magic-9").unwrap().input - 1e-5).abs() < 1e-15);
    }

    #[test]
    fn parse_models_dev_json_includes_opencode_kimi_k3() {
        let body = r#"{
            "opencode": { "models": {
                "kimi-k3": {
                    "cost": { "input": 3, "output": 15, "cache_read": 0.3 },
                    "limit": { "context": 1048576, "output": 131072 }
                }
            }}
        }"#;

        let table = parse_models_dev_json(body).expect("parsed");
        let kimi = table.get("kimi-k3").expect("opencode kimi-k3");
        assert!((kimi.input - 3e-6).abs() < 1e-15);
        assert!((kimi.output - 15e-6).abs() < 1e-15);
        assert!((kimi.cache_read - 0.3e-6).abs() < 1e-15);
        assert_eq!(kimi.context, 1_048_576);
    }

    #[test]
    fn parse_models_dev_json_includes_native_xai_models() {
        let body = r#"{
            "xai": { "models": {
                "grok-native-price-test": {
                    "cost": { "input": 2, "output": 10, "cache_read": 0.2 },
                    "limit": { "context": 131072 }
                }
            }}
        }"#;

        let table = parse_models_dev_json(body).expect("parsed");
        let grok = table
            .get("grok-native-price-test")
            .expect("native xAI model");
        assert!((grok.input - 2e-6).abs() < 1e-15);
        assert!((grok.output - 10e-6).abs() < 1e-15);
        assert_eq!(grok.context, 131_072);
    }

    #[test]
    fn parse_models_dev_json_includes_opencode_qwen_3_8_max() {
        let body = r#"{
            "alibaba-cn": { "models": {
                "qwen3.8-max": {
                    "cost": { "input": 1.77744, "output": 5.33231, "cache_read": 0.22218 },
                    "limit": { "context": 1000000, "output": 131072 }
                }
            }}
        }"#;

        let table = parse_models_dev_json(body).expect("parsed");
        let qwen = table.get("qwen3.8-max").expect("opencode qwen3.8-max");
        assert!((qwen.input - 1.77744e-6).abs() < 1e-15);
        assert!((qwen.output - 5.33231e-6).abs() < 1e-15);
        assert!((qwen.cache_read - 0.22218e-6).abs() < 1e-15);
        assert_eq!(qwen.context, 1_000_000);
    }

    #[test]
    fn parse_models_dev_json_includes_glm_5_3_and_native_claude_5_models() {
        let body = r#"{
            "anthropic": { "models": {
                "claude-fable-5": {
                    "cost": { "input": 10, "output": 50, "cache_read": 1, "cache_write": 12.5 },
                    "limit": { "context": 1000000 }
                },
                "claude-opus-5": {
                    "cost": { "input": 5, "output": 25, "cache_read": 0.5, "cache_write": 6.25 },
                    "limit": { "context": 1000000 }
                }
            }},
            "opencode-go": { "models": {
                "glm-5.3": {
                    "cost": { "input": 1.4, "output": 4.4, "cache_read": 0.26 },
                    "limit": { "context": 1000000 }
                }
            }}
        }"#;

        let table = parse_models_dev_json(body).expect("parsed");
        for name in ["claude-fable-5", "claude-opus-5", "glm-5.3"] {
            assert!(table.contains_key(name), "missing {name}");
        }
        let glm = table.get("glm-5.3").unwrap();
        assert!((glm.input - 1.4e-6).abs() < 1e-15);
        assert!((glm.output - 4.4e-6).abs() < 1e-15);
        assert!((glm.cache_read - 0.26e-6).abs() < 1e-15);
        assert_eq!(glm.context, 1_000_000);
    }

    #[test]
    fn parse_models_dev_json_includes_latest_opencode_models() {
        let body = r#"{
            "opencode": { "models": {
                "x-preview-f-free": {
                    "cost": { "input": 0, "output": 0 },
                    "limit": { "context": 1000000 }
                },
                "muse-spark-1.2": {
                    "cost": { "input": 1.25, "output": 4.25 },
                    "limit": { "context": 1048576 }
                },
                "gpt-5.2-codex": {
                    "cost": { "input": 1.75, "output": 14 },
                    "limit": { "context": 400000 }
                },
                "gemini-3.1-pro": {
                    "cost": { "input": 2, "output": 12 },
                    "limit": { "context": 1048576 }
                }
            }}
        }"#;

        let table = parse_models_dev_json(body).expect("parsed");
        let ox = table.get("x-preview-f-free").expect("latest free model");
        assert_eq!(ox.input, 0.0);
        assert_eq!(ox.output, 0.0);
        assert_eq!(ox.context, 1_000_000);

        let muse = table.get("muse-spark-1.2").expect("latest paid model");
        assert!((muse.input - 1.25e-6).abs() < 1e-15);
        assert!((muse.output - 4.25e-6).abs() < 1e-15);

        let codex = table.get("gpt-5.2-codex").expect("latest codex model");
        assert!((codex.input - 1.75e-6).abs() < 1e-15);
        assert!((codex.output - 14e-6).abs() < 1e-15);

        assert!(table.contains_key("gemini-3.1-pro"));
    }

    #[test]
    fn status_reports_loaded_count_when_remote_has_entries() {
        with_remote(
            &[
                ("status-test-1", opus_4_7_costs()),
                ("status-test-2", opus_4_7_costs()),
            ],
            || {
                let s = status();
                assert!(s.loaded);
                assert!(s.model_count >= 2);
            },
        );
    }

    #[test]
    fn short_name_picks_longest_prefix() {
        assert_eq!(short_name("claude-opus-4-8"), "Opus 4.8");
        assert_eq!(short_name("claude-opus-4-7"), "Opus 4.7");
        assert_eq!(short_name("gpt-5.3-codex"), "GPT-5.3 Codex");
        assert_eq!(short_name("gpt-5-fast"), "GPT-5"); // aliased
        assert_eq!(short_name("grok-4-fast"), "Grok Build 4 Fast");
    }

    #[test]
    fn list_for_ui_classifies_grok_family() {
        let rate = ModelCosts {
            input: 2e-6,
            output: 10e-6,
            cache_write: 2.5e-6,
            cache_read: 0.2e-6,
            context: 131_072,
        };
        with_remote(&[("grok-ui-family-test", rate)], || {
            let entry = list_for_ui()
                .into_iter()
                .find(|entry| entry.name == "grok-ui-family-test")
                .expect("Grok pricing row");
            assert_eq!(entry.family, "grok");
        });
    }

    #[test]
    fn short_name_falls_back_to_canonical_for_unknown() {
        assert_eq!(short_name("totally-new-model-9"), "totally-new-model-9");
    }

    #[test]
    fn derive_name_handles_future_versions_without_table_edits() {
        // Claude：家族在前的 4.x+ 一律自动推导
        assert_eq!(short_name("claude-opus-4-9"), "Opus 4.9");
        assert_eq!(short_name("claude-opus-5-0"), "Opus 5.0");
        assert_eq!(short_name("claude-sonnet-5"), "Sonnet 5");
        assert_eq!(short_name("claude-opus-4"), "Opus 4");
        // GPT：版本在中间、后缀任意层级
        assert_eq!(short_name("gpt-5.6-codex"), "GPT-5.6 Codex");
        assert_eq!(short_name("gpt-5.1-codex-max"), "GPT-5.1 Codex Max");
        assert_eq!(short_name("gpt-4.1-mini"), "GPT-4.1 Mini");
        assert_eq!(short_name("gpt-4o"), "GPT-4o");
    }

    #[test]
    fn short_name_override_keeps_irregular_names() {
        assert_eq!(short_name("claude-3-5-sonnet"), "Sonnet 3.5");
        assert_eq!(short_name("claude-3-7-sonnet"), "Sonnet 3.7");
        assert_eq!(short_name("o3"), "o3");
        assert_eq!(short_name("o4-mini"), "o4-mini");
        assert_eq!(short_name("codex-mini-latest"), "Codex Mini");
    }

    /// PricingView 一打开就期望「新型号在前」：在价格表里塞一组同 family
    /// 的不同代次模型，验证 list_for_ui() 按"版本号"自然倒序排（4-8 在 4-7 之前，
    /// 4 在 3-7-sonnet 之前），不是按 input 单价或字典序。
    ///
    /// 关键 bug 防回归点：之前版本号 tokenize 把 tier 名（"opus"/"sonnet"/"haiku"）
    /// 也参与比较了，导致 `claude-sonnet-4-5` 因为字典序 "sonnet" > "opus" 排到了
    /// `claude-opus-4-8` 前面 —— 但 4.8 > 4.5 才是用户在意的"新"。
    #[test]
    fn list_for_ui_sorts_newest_version_first() {
        let z = ModelCosts {
            input: 1e-6,
            output: 1e-6,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 0,
        };
        let rows = [
            ("claude-opus-4-8", z),
            ("claude-opus-4-7", z),
            ("claude-opus-4", z),
            ("claude-opus-4-1-20250805", z), // 带日期 pin 的旧版本
            ("claude-sonnet-4-5", z),        // 同代 sonnet，4.5 < 4.8 —— 必须排在 opus-4-8 之后
            ("claude-haiku-4-5", z),
            ("claude-3-7-sonnet", z),
            ("claude-3-5-haiku-20241022", z),
            ("claude-3-5-haiku", z),
            ("claude-3-haiku-20240307", z), // 日期 pin 容易被误当成超大版本号
            ("claude-3-haiku", z),
        ];
        with_remote(&rows, || {
            let list = list_for_ui();
            let names: Vec<String> = list.iter().map(|e| e.name.clone()).collect();
            // 仅校验相对顺序，避免和其它并行测试塞进表里的 entry 冲突。
            let idx = |n: &str| names.iter().position(|x| x == n).expect(n);
            // 主排序：版本号倒序
            assert!(
                idx("claude-opus-4-8") < idx("claude-opus-4-7"),
                "4-8 before 4-7: {names:?}"
            );
            assert!(
                idx("claude-opus-4-7") < idx("claude-opus-4"),
                "4-7 before naked 4: {names:?}"
            );
            // 关键回归：opus-4-8 必须在 sonnet-4-5 / haiku-4-5 之前（版本号 4.8 > 4.5），
            // 不能被 tier 名字典序影响。
            assert!(
                idx("claude-opus-4-8") < idx("claude-sonnet-4-5"),
                "opus-4-8 before sonnet-4-5: {names:?}"
            );
            assert!(
                idx("claude-opus-4-8") < idx("claude-haiku-4-5"),
                "opus-4-8 before haiku-4-5: {names:?}"
            );
            // 同 4.5 代次 tier 名同代：tier 名差异不算"新"，按字典升序 deterministic
            // —— 'h' < 's' 所以 haiku 在前。约定写死，便于回归。
            assert!(
                idx("claude-haiku-4-5") < idx("claude-sonnet-4-5"),
                "haiku-4-5 before sonnet-4-5 (lex asc): {names:?}"
            );
            // 跨代次：4.x 系列都在 3.x 之前
            assert!(
                idx("claude-haiku-4-5") < idx("claude-3-7-sonnet"),
                "haiku-4-5 before 3-7-sonnet: {names:?}"
            );
            // tiebreak：naked > 日期 pin（同版本元组，naked 是 latest alias）
            assert!(
                idx("claude-3-5-haiku") < idx("claude-3-5-haiku-20241022"),
                "naked before dated: {names:?}"
            );
            // 日期不能被当成超大版本号：claude-3-haiku-20240307 必须留在 3-haiku 后边，
            // 而不是因为 "20240307" 大就跑到所有 3.x 前。
            assert!(
                idx("claude-3-7-sonnet") < idx("claude-3-haiku"),
                "3-7 before 3-haiku: {names:?}"
            );
            assert!(
                idx("claude-3-haiku") < idx("claude-3-haiku-20240307"),
                "naked 3-haiku before dated: {names:?}"
            );
        });
    }

    /// 用户反馈：「gpt 模型的排序，5.x 系列应该在前面」。bug 来源 —— `gpt-oss-120b`
    /// 名字里 "120" 是参数量不是版本，但 version_tuple 解析后 [120] > [5]，把 oss
    /// 顶到了 gpt-5 前面。修复策略：直接把 gpt-oss / image / audio / realtime /
    /// transcribe / search-preview / gpt-35 这些非 chat completion 的变体过滤掉
    /// —— Codex CLI 用户不会跑这些。
    #[test]
    fn list_for_ui_drops_gpt_noise_and_keeps_5x_above_4x() {
        let z = ModelCosts {
            input: 1e-6,
            output: 1e-6,
            cache_write: 0.0,
            cache_read: 0.0,
            context: 0,
        };
        let rows = [
            ("gpt-5", z),
            ("gpt-5-mini", z),
            ("gpt-5.2-pro", z),
            ("gpt-4.1", z),
            ("gpt-4o-2024-11-20", z),
            ("gpt-oss-120b", z), // 噪声：120 不是版本号是参数量
            ("gpt-oss:20b-cloud", z),
            ("gpt-image-2", z),
            ("gpt-audio", z),
            ("gpt-realtime-2", z),
            ("gpt-4o-transcribe", z),
            ("gpt-4o-search-preview", z),
            ("gpt-35-turbo", z), // Azure 命名，跟 gpt-3.5 重复
            ("o1", z),
            ("codex-mini-latest", z),
        ];
        with_remote(&rows, || {
            let list = list_for_ui();
            let names: Vec<String> = list.iter().map(|e| e.name.clone()).collect();
            // 噪声项必须被过滤
            for noise in [
                "gpt-oss-120b",
                "gpt-oss:20b-cloud",
                "gpt-image-2",
                "gpt-audio",
                "gpt-realtime-2",
                "gpt-4o-transcribe",
                "gpt-4o-search-preview",
                "gpt-35-turbo",
            ] {
                assert!(
                    !names.iter().any(|n| n == noise),
                    "noise {noise} should be filtered: {names:?}"
                );
            }
            let idx = |n: &str| names.iter().position(|x| x == n).expect(n);
            // 用户的关键诉求：5.x 全部在 4.x 之前
            assert!(
                idx("gpt-5.2-pro") < idx("gpt-5"),
                "5.2-pro before 5: {names:?}"
            );
            assert!(idx("gpt-5") < idx("gpt-4.1"), "5 before 4.1: {names:?}");
            assert!(
                idx("gpt-5-mini") < idx("gpt-4o-2024-11-20"),
                "5-mini before 4o-2024: {names:?}"
            );
            assert!(idx("gpt-4.1") < idx("o1"), "4.1 before o1: {names:?}");
        });
    }
}
