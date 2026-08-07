// 跨 agent / 跨模块共享的工具函数。
// 这里只放"agent 无关"的逻辑——目录定位、时间戳、JSONL 文件写入、标题清洗等。
// agent-specific 的解析逻辑请放到对应的 `agents/<name>.rs` 文件里。

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{Block, DiffHunk, DiffLine, Msg, ProjectFileEntry};

/// `@` 文件浮层永远跳过的重目录（构建产物 / 依赖 / VCS）。点文件（`.codex` 等）保留 ——
/// 用户就是要 @ 它们。
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    "dist",
    "build",
    "out",
    "coverage",
    ".next",
    ".nuxt",
    ".svelte-kit",
    ".turbo",
    ".cache",
    "vendor",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    "Pods",
    "DerivedData",
    ".dart_tool",
    ".gradle",
    ".fvm",
];

/// 目录是否含**可见子项**（任一非 .DS_Store、非 SKIP_DIRS 的条目）。空目录 / 只含被跳过
/// 目录 → false，前端据此隐藏「进入」chevron（钻进去也是空的）。只读到第一个命中即返回。
fn dir_has_visible_child(dir: &Path) -> bool {
    let rd = match fs::read_dir(dir) {
        Ok(rd) => rd,
        Err(_) => return false,
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        return true;
    }
    false
}

fn fuzzy_text_score(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() || haystack == needle {
        return Some(0);
    }
    if haystack.starts_with(needle) {
        return Some(100 + haystack.len().saturating_sub(needle.len()));
    }
    if let Some(pos) = haystack.find(needle) {
        return Some(200 + pos);
    }

    let mut search_from = 0;
    let mut first = None;
    let mut previous = None;
    let mut gaps = 0;
    for wanted in needle.chars() {
        let (offset, matched) = haystack[search_from..]
            .char_indices()
            .find(|(_, ch)| *ch == wanted)?;
        let index = search_from + offset;
        if let Some(prev) = previous {
            gaps += index.saturating_sub(prev + 1);
        } else {
            first = Some(index);
        }
        previous = Some(index);
        search_from = index + matched.len_utf8();
    }
    Some(300 + first.unwrap_or(0) + gaps)
}

/// 裸查询的匹配优先级：文件名命中优先于相对路径命中；各自再按精确、前缀、子串、
/// 非连续字符的顺序排序。
fn project_match_score(name: &str, rel_path: &str, query: &str) -> Option<(u8, usize)> {
    let name_lc = name.to_lowercase();
    if let Some(score) = fuzzy_text_score(&name_lc, query) {
        return Some((0, score));
    }
    fuzzy_text_score(&rel_path.to_lowercase(), query).map(|score| (1, score))
}

fn fill_directory_children(base: &Path, entries: &mut [ProjectFileEntry]) {
    for entry in entries.iter_mut() {
        if entry.is_dir {
            entry.has_children = dir_has_visible_child(&base.join(&entry.rel_path));
        }
    }
}

fn search_project_files(base: &Path, query: &str, limit: usize) -> Vec<ProjectFileEntry> {
    let mut pending = vec![(base.to_path_buf(), String::new())];
    let mut matches: Vec<(u8, usize, ProjectFileEntry)> = Vec::new();

    while let Some((dir, prefix)) = pending.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == ".DS_Store" {
                continue;
            }
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir && SKIP_DIRS.contains(&name.as_str()) {
                continue;
            }
            let rel_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if let Some((source, score)) = project_match_score(&name, &rel_path, query) {
                matches.push((
                    source,
                    score,
                    ProjectFileEntry {
                        rel_path: rel_path.clone(),
                        name,
                        is_dir,
                        has_children: false,
                    },
                ));
            }
            if is_dir {
                pending.push((entry.path(), rel_path));
            }
        }
    }

    matches.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.2.is_dir.cmp(&b.2.is_dir))
            .then_with(|| a.1.cmp(&b.1))
            .then_with(|| a.2.rel_path.len().cmp(&b.2.rel_path.len()))
            .then_with(|| {
                a.2.rel_path
                    .to_lowercase()
                    .cmp(&b.2.rel_path.to_lowercase())
            })
    });
    let mut out: Vec<ProjectFileEntry> = matches
        .into_iter()
        .take(limit)
        .map(|(_, _, entry)| entry)
        .collect();
    fill_directory_children(base, &mut out);
    out
}

/// GUI chat `@` 浮层的项目文件列举。空 query → cwd 顶层；不含 `/` 的非空 query →
/// 全工作区递归模糊搜索；含 `/` 的 query → 保留目录逐级浏览，末段在直接子项中模糊过滤。
pub fn list_project_files(cwd: &str, query: &str, limit: usize) -> Vec<ProjectFileEntry> {
    let base = Path::new(cwd);
    if cwd.is_empty() || !base.is_dir() || limit == 0 {
        return Vec::new();
    }
    let normalized_query = query.replace('\\', "/");
    let query_lc = normalized_query.to_lowercase();
    if !query_lc.is_empty() && !query_lc.contains('/') {
        return search_project_files(base, &query_lc, limit);
    }

    // 末段 `/` 切成「目录前缀（含尾斜杠）」+「过滤词」。
    let (dir_part, filter) = match normalized_query.rfind('/') {
        Some(i) => (&normalized_query[..=i], &normalized_query[i + 1..]),
        None => ("", normalized_query.as_str()),
    };
    let filter_lc = filter.to_lowercase();
    let target = if dir_part.is_empty() {
        base.to_path_buf()
    } else {
        base.join(dir_part)
    };

    let mut out: Vec<(usize, ProjectFileEntry)> = Vec::new();
    let rd = match fs::read_dir(&target) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(), // 目标不存在（如打错的前缀）→ 空结果
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if name == ".DS_Store" {
            continue;
        }
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir && SKIP_DIRS.contains(&name.as_str()) {
            continue;
        }
        let Some(score) = fuzzy_text_score(&name.to_lowercase(), &filter_lc) else {
            continue;
        };
        out.push((
            score,
            ProjectFileEntry {
                rel_path: format!("{dir_part}{name}"),
                name,
                is_dir,
                has_children: false,
            },
        ));
    }

    // 目录在前、文件在后；组内优先更好的模糊匹配，再按路径稳定排序。
    out.sort_by(|a, b| match (a.1.is_dir, b.1.is_dir) {
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        _ => a.0.cmp(&b.0).then_with(|| a.1.rel_path.cmp(&b.1.rel_path)),
    });
    out.truncate(limit);
    let mut out: Vec<ProjectFileEntry> = out.into_iter().map(|(_, entry)| entry).collect();
    fill_directory_children(base, &mut out);
    out
}

pub fn home() -> PathBuf {
    dirs::home_dir().expect("无法定位用户主目录")
}

/// 构造不弹控制台窗口的子进程命令。打包后的 Windows GUI 进程没有控制台，
/// 直接 spawn 会给每个子进程新开一个 conhost 黑框（dev 模式继承终端所以看不出来），
/// 所有「后台静默执行」的子进程都必须走这里，别直接 `Command::new`。
pub fn silent_command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn mtime_millis(p: &Path) -> u64 {
    fs::metadata(p)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub fn is_jsonl(p: &Path) -> bool {
    p.extension().map(|x| x == "jsonl").unwrap_or(false)
}

/// 把首条用户消息清洗成简短标题：去掉 <...> 标记块、折叠空白、截断。
pub fn clean_title(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with("Caveat:") {
        return String::new();
    }
    let mut out = String::new();
    let mut depth = 0i32;
    for c in trimmed.chars() {
        match c {
            '<' => depth += 1,
            '>' if depth > 0 => depth -= 1,
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    let collapsed: String = out.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed.chars().take(100).collect()
}

/// 从用户消息提取副标题：取第一行非空文本，去掉 @file 引用、markdown 语法，截断到 120 字符。
pub fn truncate_subtitle(raw: &str) -> String {
    use once_cell::sync::Lazy;
    static RE_STRIP: Lazy<regex_lite::Regex> = Lazy::new(|| {
        regex_lite::Regex::new(r"@\[?[A-Za-z0-9_./-]+\]?|\[Image[^\]]*\]|\!\[[^\]]*\]").unwrap()
    });
    let line = raw
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('<') && !l.starts_with("Caveat:"))
        .find_map(|l| {
            let stripped = RE_STRIP.replace_all(l, "");
            let trimmed = stripped.trim().to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed)
            }
        })
        .unwrap_or_default();
    let cleaned = std::borrow::Cow::Borrowed(line.as_str());
    let collapsed: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= 120 {
        collapsed
    } else {
        let mut s: String = collapsed.chars().take(117).collect();
        s.push('…');
        s
    }
}

pub fn text_block(kind: &str, s: &str) -> Block {
    Block {
        kind: kind.to_string(),
        text: Some(s.to_string()),
        ..Default::default()
    }
}

/// GUI 的 Codex `turn/steer` 需要一段不可见的延后约束，防止原生 steer 抢占当前任务。
/// Codex 会把该输入原样写进 rollout；渲染时只应露出 `<follow-up>` 内的用户原文。
/// 只接受完整精确前后缀，普通用户消息不会被误处理。
pub fn visible_deferred_follow_up(text: &str) -> &str {
    const PREFIX: &str = "[Deferred follow-up from the user]\n\
Continue and fully complete the original task already in progress first.\n\
Do not interrupt it, change its direction, or begin the follow-up below yet.\n\
Only after the original task is complete, process this follow-up in the order received:\n\
<follow-up>\n";
    const SUFFIX: &str = "\n</follow-up>";
    text.strip_prefix(PREFIX)
        .and_then(|body| body.strip_suffix(SUFFIX))
        .unwrap_or(text)
}

pub fn simple_msg(role: &str, ts: Option<String>, block: Block) -> Msg {
    Msg {
        uuid: None,
        role: role.to_string(),
        timestamp: ts,
        model: None,
        sidechain: false,
        blocks: vec![block],
        meta_kind: None,
    }
}

/// 简易 ISO-8601 UTC 时间字符串：`YYYY-MM-DDTHH:MM:SS.mmmZ`。
/// 只用于写入 codex 的 thread_name_updated / session_index 行，精度够用。
pub fn format_iso8601_utc(secs: i64, ms: u32) -> String {
    let s = secs.rem_euclid(60) as u32;
    let m = (secs.div_euclid(60)).rem_euclid(60) as u32;
    let h = (secs.div_euclid(3600)).rem_euclid(24) as u32;
    let mut days = secs.div_euclid(86400);
    let mut year: i64 = 1970;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let yd = if leap { 366 } else { 365 };
        if days < yd {
            break;
        }
        days -= yd;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let mdays = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month: usize = 0;
    while month < 12 && days >= mdays[month] as i64 {
        days -= mdays[month] as i64;
        month += 1;
    }
    let day = days as u32 + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year,
        month + 1,
        day,
        h,
        m,
        s,
        ms
    )
}

/// 毫秒时间戳 → `YYYY-MM-DD`（UTC）。给统计 dashboard 的活跃度热图按日分桶用。
/// 与 `format_iso8601_utc` 共享同一套手写历法（不引 chrono）—— 这里只截前 10 位日期部分。
pub fn yyyymmdd_utc(ms: u64) -> String {
    let s = format_iso8601_utc((ms / 1000) as i64, (ms % 1000) as u32);
    s.chars().take(10).collect()
}

/// 毫秒时间戳 → `YYYY-MM-DD`（用户所在时区）。统计窗口按本地日历日切的话，
/// daily 热图也必须按本地日切，否则 "Today" 总额对得上 codeburn，但热图上
/// 同一笔花费会被画到错误的格子里。
pub fn yyyymmdd_local(ms: u64) -> String {
    use chrono::{Local, TimeZone};
    let secs = (ms / 1000) as i64;
    let nsecs = ((ms % 1000) as u32) * 1_000_000;
    match Local.timestamp_opt(secs, nsecs).single() {
        Some(dt) => dt.format("%Y-%m-%d").to_string(),
        None => yyyymmdd_utc(ms),
    }
}

/// ISO-8601 → unix 毫秒。只解析 `YYYY-MM-DDTHH:MM:SS[.fff]Z` 这一形态；
/// 其他形态退到 None（聚合器会用文件 mtime 兜底）。手写以免引 chrono。
/// 给统计聚合器从 JSONL 时间戳串还原 unix ms 用。
pub fn parse_iso8601_ms(s: &str) -> Option<i64> {
    let bytes = s.as_bytes();
    if bytes.len() < 19 || bytes[4] != b'-' || bytes[7] != b'-' || bytes[10] != b'T' {
        return None;
    }
    let year: i64 = std::str::from_utf8(&bytes[0..4]).ok()?.parse().ok()?;
    let mon: u32 = std::str::from_utf8(&bytes[5..7]).ok()?.parse().ok()?;
    let day: u32 = std::str::from_utf8(&bytes[8..10]).ok()?.parse().ok()?;
    let h: u32 = std::str::from_utf8(&bytes[11..13]).ok()?.parse().ok()?;
    let m: u32 = std::str::from_utf8(&bytes[14..16]).ok()?.parse().ok()?;
    let sec: u32 = std::str::from_utf8(&bytes[17..19]).ok()?.parse().ok()?;
    let mut ms: u32 = 0;
    if bytes.len() > 19 && bytes[19] == b'.' {
        let end = (20 + 3).min(bytes.len());
        let frac = std::str::from_utf8(&bytes[20..end]).ok()?;
        if let Ok(n) = frac.parse::<u32>() {
            ms = n * 10u32.pow(3 - frac.len() as u32);
        }
    }
    // 转 unix epoch 秒：简易历法
    let mut days: i64 = 0;
    for y in 1970..year {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        days += if leap { 366 } else { 365 };
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let mdays: [i64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    for &md in mdays.iter().take((mon - 1) as usize) {
        days += md;
    }
    days += (day - 1) as i64;
    let secs = days * 86400 + (h as i64) * 3600 + (m as i64) * 60 + sec as i64;
    Some(secs * 1000 + ms as i64)
}

/// 解析 unified diff 文本为结构化 DiffHunk（agent 无关的通用文本格式解析，
/// agy 的 CODE_ACTION 与 opencode 的 edit metadata.diff 共用）。
pub fn parse_unified_diff(text: &str) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut current: Option<DiffHunk> = None;
    let mut old_line: u32 = 0;
    let mut new_line: u32 = 0;

    for raw_line in text.lines() {
        if let Some((os, ns)) = parse_hunk_header(raw_line) {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            old_line = os;
            new_line = ns;
            current = Some(DiffHunk {
                old_start: os,
                new_start: ns,
                lines: Vec::new(),
            });
        } else if let Some(ref mut hunk) = current {
            if let Some(rest) = raw_line.strip_prefix('+') {
                hunk.lines.push(DiffLine {
                    kind: "add".into(),
                    old_no: None,
                    new_no: Some(new_line),
                    text: rest.to_string(),
                });
                new_line += 1;
            } else if let Some(rest) = raw_line.strip_prefix('-') {
                hunk.lines.push(DiffLine {
                    kind: "del".into(),
                    old_no: Some(old_line),
                    new_no: None,
                    text: rest.to_string(),
                });
                old_line += 1;
            } else {
                let text = raw_line.strip_prefix(' ').unwrap_or(raw_line);
                hunk.lines.push(DiffLine {
                    kind: "ctx".into(),
                    old_no: Some(old_line),
                    new_no: Some(new_line),
                    text: text.to_string(),
                });
                old_line += 1;
                new_line += 1;
            }
        }
    }
    if let Some(h) = current {
        hunks.push(h);
    }
    hunks
}

pub fn parse_hunk_header(line: &str) -> Option<(u32, u32)> {
    // @@ -old_start[,old_count] +new_start[,new_count] @@
    let line = line.strip_prefix("@@ ")?;
    let end = line.find(" @@")?;
    let range_part = &line[..end];
    let mut parts = range_part.split(' ');
    let old_part = parts.next()?.strip_prefix('-')?;
    let new_part = parts.next()?.strip_prefix('+')?;
    let old_start: u32 = old_part.split(',').next()?.parse().ok()?;
    let new_start: u32 = new_part.split(',').next()?.parse().ok()?;
    Some((old_start, new_start))
}

/// 校验 rename 名称：去空白后非空且不过长。返回 trimmed 切片。
pub fn validate_rename_name(name: &str) -> Result<&str, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Name cannot be empty".to_string());
    }
    if trimmed.chars().count() > 200 {
        return Err("Name too long".to_string());
    }
    Ok(trimmed)
}

/// 安全地把一行追加到 JSONL：若文件末尾不是换行，先补一个，再写 `line + "\n"`。
pub fn append_jsonl_line(path: &Path, line: &str) -> Result<(), String> {
    let needs_nl = fs::metadata(path)
        .map(|m| m.len())
        .ok()
        .and_then(|len| {
            if len == 0 {
                Some(false)
            } else {
                use std::io::{Read, Seek, SeekFrom};
                let mut g = fs::File::open(path).ok()?;
                g.seek(SeekFrom::End(-1)).ok()?;
                let mut buf = [0u8; 1];
                g.read_exact(&mut buf).ok()?;
                Some(buf[0] != b'\n')
            }
        })
        .unwrap_or(false);
    let mut f = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|e| format!("Failed to open session file: {e}"))?;
    if needs_nl {
        f.write_all(b"\n")
            .map_err(|e| format!("Failed to append newline: {e}"))?;
    }
    f.write_all(line.as_bytes())
        .map_err(|e| format!("Failed to write rename entry: {e}"))?;
    f.write_all(b"\n")
        .map_err(|e| format!("Failed to write newline: {e}"))?;
    Ok(())
}

/// 当前 git 分支名 —— 给 chat 头部展示「分支图标 + 名称」用。
///
/// 直接读 `.git/HEAD` 而不 shell out 到 `git`：GUI app 从 Finder 启动时 PATH 往往很瘦，
/// 未必能找到 git；读文件没有这个依赖，也更快。从 `cwd` 逐级向上找 `.git`：
/// - `.git` 是目录 → 普通仓库，读它下面的 `HEAD`；
/// - `.git` 是文件（`gitdir: <path>`）→ worktree / submodule，跟到真正的 gitdir 再读 `HEAD`。
///
/// `HEAD` 内容两种形态：`ref: refs/heads/<branch>`（返回 `<branch>`）或裸 commit sha
/// （detached，返回短 sha）。非仓库 / 读不到时返回 `None`，前端据此不渲染分支块。
pub fn git_current_branch(cwd: &str) -> Option<String> {
    if cwd.is_empty() {
        return None;
    }
    let mut dir: &Path = Path::new(cwd);
    loop {
        let dot_git = dir.join(".git");
        if dot_git.is_dir() {
            return read_head_ref(&dot_git);
        }
        if dot_git.is_file() {
            // ".git" 文件：内容形如 "gitdir: /abs/or/rel/path"
            let content = fs::read_to_string(&dot_git).ok()?;
            let rest = content.lines().next()?.strip_prefix("gitdir:")?.trim();
            let gitdir = Path::new(rest);
            let gitdir = if gitdir.is_absolute() {
                gitdir.to_path_buf()
            } else {
                dir.join(gitdir)
            };
            return read_head_ref(&gitdir);
        }
        dir = dir.parent()?;
    }
}

/// 读 gitdir 下的 `HEAD`，解析出分支名或短 commit sha。
fn read_head_ref(gitdir: &Path) -> Option<String> {
    let head = fs::read_to_string(gitdir.join("HEAD")).ok()?;
    let head = head.trim();
    if let Some(r) = head.strip_prefix("ref:") {
        let r = r.trim();
        return Some(r.strip_prefix("refs/heads/").unwrap_or(r).to_string());
    }
    if head.is_empty() {
        return None;
    }
    // detached HEAD：取短 sha
    Some(head.chars().take(7).collect())
}

/// 把聊天里点击的「文件引用」解析成磁盘上真实存在的文件路径。
///
/// `rel` 可能是完整相对路径，也可能是**部分路径**——聊天正文里常写成 `bank/refund_detail.dart`，
/// 真身其实在 `lib/pages/wallet/bank/refund_detail.dart`。解析两步：
///   1. 先按 `cwd` 直接拼接，存在即用（完整相对路径 / 绝对路径走这条）。
///   2. 否则在 `cwd` 下广度优先搜索，找「目录段尾部与 `rel` 完全对齐」的文件。BFS 保证先
///      命中**最靠近根**的那个；段级对齐避免 `xbank/f.dart` 误命中 `bank/f.dart`。
///
/// 跳过 `SKIP_DIRS`（依赖 / 构建产物），并对访问条目数设上限，避免在超大仓库里走太久。
/// 找不到返回 `None`（调用方据此报「文件不存在」）。
pub fn resolve_file_ref(cwd: &str, rel: &str) -> Option<PathBuf> {
    if cwd.is_empty() || rel.is_empty() {
        return None;
    }
    let base = Path::new(cwd);
    let direct = base.join(rel);
    if direct.is_file() {
        return Some(direct);
    }
    let want: Vec<String> = rel
        .split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != ".")
        .map(|s| s.to_string())
        .collect();
    if want.is_empty() {
        return None;
    }
    let file_name = want.last().unwrap().as_str();

    let mut budget: usize = 50_000; // 访问条目上限（一次点击的可接受成本）
    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(base.to_path_buf());
    while let Some(dir) = queue.pop_front() {
        let rd = match fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        let mut subdirs: Vec<PathBuf> = Vec::new();
        for entry in rd.flatten() {
            if budget == 0 {
                return None;
            }
            budget -= 1;
            let name = entry.file_name().to_string_lossy().to_string();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                if name == ".DS_Store" || SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                subdirs.push(entry.path());
            } else if name == file_name {
                let path = entry.path();
                if path_ends_with_segments(&path, &want) {
                    return Some(path); // BFS：先命中即最浅
                }
            }
        }
        // 同层文件全看完再下钻 —— 维持「最靠近根优先」。
        queue.extend(subdirs);
    }
    None
}

/// `path` 的尾部目录段是否与 `want` 逐段相等（段级对齐，非子串匹配）。
fn path_ends_with_segments(path: &Path, want: &[String]) -> bool {
    let segs: Vec<String> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(s) => Some(s.to_string_lossy().to_string()),
            _ => None,
        })
        .collect();
    if want.len() > segs.len() {
        return false;
    }
    segs[segs.len() - want.len()..] == *want
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn yyyymmdd_at_unix_epoch_is_1970_01_01() {
        assert_eq!(yyyymmdd_utc(0), "1970-01-01");
    }

    #[test]
    fn yyyymmdd_handles_leap_day() {
        // 2024-02-29T00:00:00Z = 1709164800 s
        assert_eq!(yyyymmdd_utc(1_709_164_800_000), "2024-02-29");
    }

    #[test]
    fn yyyymmdd_strips_to_date_only() {
        // 2026-05-23T12:34:56Z = 1779539696 s
        assert_eq!(yyyymmdd_utc(1_779_539_696_000), "2026-05-23");
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cssv_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".codex/skills/git-push")).unwrap();
        fs::write(dir.join(".codex/config.toml"), "x").unwrap();
        fs::write(dir.join(".codex/skills/git-push/SKILL.md"), "x").unwrap();
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::write(dir.join("node_modules/pkg/index.js"), "x").unwrap();
        fs::create_dir(dir.join(".empty")).unwrap(); // 空目录 → has_children=false
        fs::write(dir.join("README.md"), "x").unwrap();
        dir
    }

    #[test]
    fn list_project_files_empty_query_lists_top_level_only() {
        let dir = scratch("toplevel");
        let out = list_project_files(dir.to_str().unwrap(), "", 200);
        let rels: Vec<&str> = out.iter().map(|e| e.rel_path.as_str()).collect();
        // 只列直接子项，目录在前；node_modules 被跳过、不出现。
        assert_eq!(rels, vec![".codex", ".empty", "README.md"]);
        // 含可见子项的目录 has_children=true；空目录 false；文件恒 false。
        assert!(out[0].is_dir && out[0].has_children); // .codex
        assert!(out[1].is_dir && !out[1].has_children); // .empty
        assert!(!out[2].is_dir && !out[2].has_children); // README.md
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_project_files_lists_direct_children_one_level() {
        let dir = scratch("children");
        // 进入 .codex/：只列其直接子项（不递归进 skills/git-push）。目录在前。
        let out = list_project_files(dir.to_str().unwrap(), ".codex/", 200);
        let rels: Vec<&str> = out.iter().map(|e| e.rel_path.as_str()).collect();
        assert_eq!(rels, vec![".codex/skills", ".codex/config.toml"]);
        assert!(out[0].is_dir && out[0].has_children); // skills 含 git-push
        assert!(!out[1].is_dir); // config.toml
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_project_files_fuzzy_filters_by_trailing_segment() {
        let dir = scratch("filter");
        // 路径式 `<dir>/<filter>` 只过滤目标目录的直接子项，并支持非连续字符匹配。
        let nested = list_project_files(dir.to_str().unwrap(), ".codex/sks", 200);
        assert_eq!(
            nested
                .iter()
                .map(|e| e.rel_path.as_str())
                .collect::<Vec<_>>(),
            vec![".codex/skills"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_project_files_bare_query_recursively_fuzzy_matches_nested_files() {
        let dir = scratch("recursive_fuzzy");
        fs::create_dir_all(dir.join("src/components")).unwrap();
        fs::write(dir.join("src/components/ChatComposer.vue"), "x").unwrap();

        for query in ["ChatCom", "Com", "Cpsr"] {
            let out = list_project_files(dir.to_str().unwrap(), query, 200);
            assert_eq!(out[0].rel_path, "src/components/ChatComposer.vue");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_project_files_skips_ignored_dirs() {
        let dir = scratch("skip");
        // node_modules 在顶层即被跳过，绝不递归进去 → 顶层用 "index" 过滤命中不到它里面的文件。
        let out = list_project_files(dir.to_str().unwrap(), "index", 200);
        assert!(out.is_empty());
        // 即便显式进入 node_modules/，也按「跳过」处理（不列内容）。
        let inside = list_project_files(dir.to_str().unwrap(), "node_modules/", 200);
        assert_eq!(
            inside
                .iter()
                .map(|e| e.rel_path.as_str())
                .collect::<Vec<_>>(),
            vec!["node_modules/pkg"]
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_project_files_invalid_cwd_is_empty() {
        assert!(list_project_files("", "x", 10).is_empty());
        assert!(list_project_files("/no/such/dir/xyz", "x", 10).is_empty());
    }

    fn git_scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cssv_git_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join(".git")).unwrap();
        dir
    }

    #[test]
    fn git_branch_reads_ref_head() {
        let dir = git_scratch("ref");
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/feature/chat\n").unwrap();
        assert_eq!(
            git_current_branch(dir.to_str().unwrap()).as_deref(),
            Some("feature/chat")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_branch_detached_head_is_short_sha() {
        let dir = git_scratch("detached");
        fs::write(
            dir.join(".git/HEAD"),
            "0123456789abcdef0123456789abcdef01234567\n",
        )
        .unwrap();
        assert_eq!(
            git_current_branch(dir.to_str().unwrap()).as_deref(),
            Some("0123456")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_branch_walks_up_from_subdir() {
        let dir = git_scratch("nested");
        fs::write(dir.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let sub = dir.join("a/b/c");
        fs::create_dir_all(&sub).unwrap();
        assert_eq!(
            git_current_branch(sub.to_str().unwrap()).as_deref(),
            Some("main")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_branch_follows_worktree_gitdir_file() {
        // worktree：cwd 下的 ".git" 是文件，内容 "gitdir: <真正的 gitdir>"。
        let dir = git_scratch("worktree");
        let real_gitdir = dir.join("realgit");
        fs::create_dir_all(&real_gitdir).unwrap();
        fs::write(real_gitdir.join("HEAD"), "ref: refs/heads/wt-branch\n").unwrap();
        let wt = dir.join("wt");
        fs::create_dir_all(&wt).unwrap();
        fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", real_gitdir.display()),
        )
        .unwrap();
        assert_eq!(
            git_current_branch(wt.to_str().unwrap()).as_deref(),
            Some("wt-branch")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_branch_none_when_not_a_repo() {
        assert_eq!(git_current_branch(""), None);
        assert_eq!(git_current_branch("/no/such/dir/xyz"), None);
        let dir = std::env::temp_dir().join(format!("cssv_nogit_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(git_current_branch(dir.to_str().unwrap()), None);
        let _ = fs::remove_dir_all(&dir);
    }

    // 仿真一个 Flutter 工程：多个同名 refund_detail.dart 散在不同父目录下，外加一个被跳过的
    // node_modules 副本，用来验证「部分路径 → 段级后缀解析」。
    fn ref_scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cssv_ref_{}_{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&dir);
        for sub in ["bank", "cash", "alipay"] {
            let d = dir.join(format!("lib/pages/wallet/{sub}"));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("refund_detail.dart"), "x").unwrap();
        }
        fs::create_dir_all(dir.join("lib/main")).unwrap();
        fs::write(dir.join("lib/main/app.dart"), "x").unwrap();
        // 被 SKIP_DIRS 跳过的目录里也放一个同名文件，确认搜索不会命中它。
        fs::create_dir_all(dir.join("node_modules/pkg/bank")).unwrap();
        fs::write(dir.join("node_modules/pkg/bank/refund_detail.dart"), "x").unwrap();
        dir
    }

    #[test]
    fn resolve_file_ref_direct_join_wins() {
        let dir = ref_scratch("direct");
        let got = resolve_file_ref(dir.to_str().unwrap(), "lib/main/app.dart");
        assert_eq!(got, Some(dir.join("lib/main/app.dart")));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_file_ref_partial_path_is_segment_aligned() {
        let dir = ref_scratch("partial");
        // `bank/refund_detail.dart` 只对齐 wallet/bank 那一份，不会错到 cash/alipay。
        assert_eq!(
            resolve_file_ref(dir.to_str().unwrap(), "bank/refund_detail.dart"),
            Some(dir.join("lib/pages/wallet/bank/refund_detail.dart")),
        );
        assert_eq!(
            resolve_file_ref(dir.to_str().unwrap(), "cash/refund_detail.dart"),
            Some(dir.join("lib/pages/wallet/cash/refund_detail.dart")),
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_file_ref_rejects_substring_segment() {
        let dir = ref_scratch("substr");
        // `ank/refund_detail.dart` 不是任何文件的「整段」后缀 → 不命中（不能子串匹配 bank）。
        assert_eq!(
            resolve_file_ref(dir.to_str().unwrap(), "ank/refund_detail.dart"),
            None,
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_file_ref_skips_ignored_dirs() {
        let dir = ref_scratch("skip");
        // node_modules 里那份同名文件不应被搜到（只有它带 pkg/bank 这层时才可能命中）。
        assert_eq!(
            resolve_file_ref(dir.to_str().unwrap(), "pkg/bank/refund_detail.dart"),
            None,
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_file_ref_none_when_missing_or_empty() {
        let dir = ref_scratch("missing");
        assert_eq!(
            resolve_file_ref(dir.to_str().unwrap(), "nope/ghost.dart"),
            None
        );
        assert_eq!(resolve_file_ref("", "bank/refund_detail.dart"), None);
        assert_eq!(resolve_file_ref(dir.to_str().unwrap(), ""), None);
        let _ = fs::remove_dir_all(&dir);
    }
}

// ─── 消息后处理：提取文本块中的图片与文件物理路径并抬升为独立 Block ───

pub fn post_process_session_msgs(msgs: &mut [Msg]) {
    for msg in msgs {
        if msg.role != "user" {
            continue;
        }
        // Claude / Codex 的专属解析器有时已先生成文件 tag。后处理仍要保留原始
        // 提示词中的行内 @引用，但不能因此再渲染一份相同附件。
        let mut attachment_keys: HashSet<String> =
            msg.blocks.iter().filter_map(attachment_key).collect();
        let mut new_blocks = Vec::new();
        for block in std::mem::take(&mut msg.blocks) {
            if block.kind == "text" {
                if let Some(text) = &block.text {
                    let (lifted, remaining_text) = lift_paths_from_text(text);
                    for lifted_block in lifted {
                        if let Some(key) = attachment_key(&lifted_block) {
                            if !attachment_keys.insert(key) {
                                continue;
                            }
                        }
                        new_blocks.push(lifted_block);
                    }
                    if !remaining_text.trim().is_empty() {
                        new_blocks.push(Block {
                            kind: "text".to_string(),
                            text: Some(remaining_text),
                            ..Default::default()
                        });
                    }
                    continue;
                }
            }
            new_blocks.push(block);
        }
        msg.blocks = new_blocks;
    }
}

fn attachment_key(block: &Block) -> Option<String> {
    match block.kind.as_str() {
        "file" => block.file_path.as_ref().map(|path| format!("file:{path}")),
        "image" => block.image_src.as_ref().map(|src| format!("image:{src}")),
        _ => None,
    }
}

/// 移除提示词**开头或末尾连续**的文件引用，保留中间引用的完整原文。`refs` 必须按文本
/// 顺序且互不重叠；它们通常来自同一条正则扫描。这样 `@a.md 正文 @b.md @c.md` 会变为
/// `正文`，而 `正文 @a.md 的差异` 则完全不动。
pub fn strip_edge_references(text: &str, refs: &[(usize, usize)]) -> String {
    if refs.is_empty() {
        return text.to_string();
    }
    let mut remove = vec![false; refs.len()];

    let mut cursor = 0;
    for (idx, (start, end)) in refs.iter().copied().enumerate() {
        if text[cursor..start].trim().is_empty() {
            remove[idx] = true;
            cursor = end;
        } else {
            break;
        }
    }

    let mut cursor = text.len();
    for idx in (0..refs.len()).rev() {
        let (start, end) = refs[idx];
        if text[end..cursor].trim().is_empty() {
            remove[idx] = true;
            cursor = start;
        } else {
            break;
        }
    }

    let mut cleaned = String::new();
    let mut last = 0;
    for ((start, end), should_remove) in refs.iter().copied().zip(remove) {
        if should_remove {
            cleaned.push_str(&text[last..start]);
            last = end;
        }
    }
    cleaned.push_str(&text[last..]);
    cleaned
}

fn lift_paths_from_text(text: &str) -> (Vec<Block>, String) {
    let mut lifted = Vec::new();
    let mut cleaned_text = text.to_string();

    // 1. `@[path]` 无论在提示词何处都生成附件 tag；首尾连续的附件从正文移除，
    //    中间引用需完整保留，例如「分析@[src/a.sh]的执行过程」。
    let re_bracket = regex_lite::Regex::new(r"@\[([^\]]+)\]").expect("valid regex");
    let mut refs = Vec::new();
    for caps in re_bracket.captures_iter(&cleaned_text) {
        let whole = caps.get(0).unwrap();
        let path = caps.get(1).unwrap().as_str().trim().to_string();
        lift_path_block(&path, &mut lifted);
        refs.push((whole.start(), whole.end()));
    }
    cleaned_text = strip_edge_references(&cleaned_text, &refs);

    // 2. `@path` / `@"path"` 同样始终生成 tag，但中间位置保留完整原文。
    let re_at = regex_lite::Regex::new(r#"@"([^"]+)"|@(\S+)"#).expect("valid regex");
    let mut refs = Vec::new();
    for caps in re_at.captures_iter(&cleaned_text) {
        let whole = caps.get(0).unwrap();
        let (path, reference_end) = match (caps.get(1), caps.get(2)) {
            (Some(q), _) => (Some(q.as_str().to_string()), whole.end()),
            (None, Some(u)) => {
                let path = inline_at_file_path(u.as_str(), None);
                let end = path
                    .as_ref()
                    .map(|path| whole.start() + 1 + path.len())
                    .unwrap_or(whole.end());
                (path, end)
            }
            _ => (None, whole.end()),
        };
        if let Some(p) = path {
            lift_path_block(&p, &mut lifted);
            refs.push((whole.start(), reference_end));
        }
    }
    cleaned_text = strip_edge_references(&cleaned_text, &refs);

    // 2b. 提取 [name](path/) 形式的文件夹/文件 markdown 链接（Codex 文件夹引用格式）
    let re_mdlink = regex_lite::Regex::new(r"\[([^\]]+)\]\(([^)]+)\)").expect("valid regex");
    let mut temp = String::new();
    let mut last = 0;
    for caps in re_mdlink.captures_iter(&cleaned_text) {
        let whole = caps.get(0).unwrap();
        let _name = caps.get(1).unwrap().as_str();
        let path = caps.get(2).unwrap().as_str().trim();
        if looks_like_file_path(path) {
            temp.push_str(&cleaned_text[last..whole.start()]);
            last = whole.end();
            let is_dir =
                path.ends_with('/') || path.ends_with('\\') || std::path::Path::new(path).is_dir();
            lifted.push(Block {
                kind: "file".to_string(),
                file_path: Some(
                    path.trim_end_matches('/')
                        .trim_end_matches('\\')
                        .to_string(),
                ),
                is_dir: if is_dir { Some(true) } else { None },
                ..Default::default()
            });
        }
    }
    temp.push_str(&cleaned_text[last..]);
    cleaned_text = temp;

    // 用户粘贴的 Xcode 构建日志里会出现临时 result bundle、Pods.xcodeproj 等路径。
    // 它们是错误文案的一部分，不是用户附加的文件；若抬成 tag 会把整段诊断的阅读节奏打断。
    // 显式的 @文件引用已在前两步处理，仍不受这个保护影响。
    if is_xcode_build_log(&cleaned_text) {
        return (lifted, cleaned_text.trim().to_string());
    }

    // 3. 提取行内绝对路径 / 家目录路径（支持中文粘连，如 "前缀/var/path.png"）
    // 排除 URL 中的路径段（`://` 后面不是文件路径）
    let re_abs = regex_lite::Regex::new(
        r#"(?x)
        (?: ^ | \s | [^\x00-\x7F] | [^\w.@/:=-] )
        (
            (?: ~[/\\] | /[a-zA-Z0-9] | [a-zA-Z]:[/\\] )
            (?: [\w.@~-]+ [/\\] )*
            [\w.@-]+ \. [a-zA-Z0-9]+
            (?: :\d+(?::\d+)? )?
        )
        "#,
    )
    .expect("valid regex");

    let mut temp = String::new();
    let mut last = 0;
    for caps in re_abs.captures_iter(&cleaned_text) {
        let whole = caps.get(0).unwrap();
        let path = caps.get(1).unwrap().as_str().trim().to_string();

        let capture = caps.get(1).unwrap();
        let capture_start = capture.start();
        // 行内 `@/path` 已在步骤 2 识别；正文必须保留其原文，不能被这里再次剥走。
        if capture_start > 0 && cleaned_text.as_bytes()[capture_start - 1] == b'@' {
            continue;
        }
        if is_rust_diagnostic_location(&cleaned_text, capture_start) {
            continue;
        }

        temp.push_str(&cleaned_text[last..capture_start]);
        last = whole.end();
        lift_path_block(&path, &mut lifted);
    }
    temp.push_str(&cleaned_text[last..]);
    cleaned_text = temp;

    // 4. 对余下文本运行逐行文件提炼（处理相对路径等）
    let mut remaining_lines = Vec::new();
    for line in cleaned_text.lines() {
        let trimmed_line = line.trim();
        if trimmed_line.is_empty() {
            remaining_lines.push(line);
            continue;
        }

        if let Some(path) = parse_line_as_path(trimmed_line) {
            lift_path_block(&path, &mut lifted);
        } else {
            remaining_lines.push(line);
        }
    }

    let mut remaining_text = remaining_lines.join("\n");
    remaining_text = remaining_text.trim().to_string();

    (lifted, remaining_text)
}

fn is_xcode_build_log(text: &str) -> bool {
    text.contains("Error output from Xcode build:")
        || text.contains("Xcode's output:")
        || text.contains("** BUILD FAILED **")
}

fn is_rust_diagnostic_location(text: &str, path_start: usize) -> bool {
    let line_start = text[..path_start].rfind('\n').map(|i| i + 1).unwrap_or(0);
    text[line_start..path_start].trim_end().ends_with("-->")
}

fn lift_path_block(path: &str, lifted: &mut Vec<Block>) {
    if path.contains('{')
        || path.contains('}')
        || path.contains('$')
        || path.contains('|')
        || path.contains('^')
        || path.contains('*')
    {
        return;
    }
    let path_buf = expand_home(path);
    let exists = path_buf.exists();
    let is_dir = exists && path_buf.is_dir();

    if exists && !is_dir && is_image_file(&path_buf) {
        lifted.push(Block {
            kind: "image".to_string(),
            image_src: Some(path_buf.to_string_lossy().to_string()),
            ..Default::default()
        });
    } else {
        lifted.push(Block {
            kind: "file".to_string(),
            file_path: Some(path.to_string()),
            is_dir: if is_dir { Some(true) } else { None },
            ..Default::default()
        });
    }
}

fn parse_line_as_path(line: &str) -> Option<String> {
    let mut s = line;
    if s.starts_with('@') {
        s = &s[1..];
    }
    if ((s.starts_with('"') && s.ends_with('"')) || (s.starts_with('\'') && s.ends_with('\'')))
        && s.len() >= 2
    {
        s = &s[1..s.len() - 1];
    }
    s = s.trim();
    if s.is_empty() {
        return None;
    }

    let is_abs = s.starts_with('/')
        || s.starts_with('\\')
        || s.starts_with("~/")
        || s.starts_with("~\\")
        || (s.len() >= 2 && s.as_bytes()[1] == b':');

    if is_abs {
        if s.contains(char::is_whitespace) {
            return None;
        }
        let after_root = if s.starts_with("~/") || s.starts_with("~\\") {
            &s[2..]
        } else if s.starts_with('/') || s.starts_with('\\') {
            &s[1..]
        } else {
            &s[2..] // drive letter C:
        };
        let has_sep = after_root.contains('/') || after_root.contains('\\');
        let has_ext = after_root
            .rsplit_once('.')
            .map(|(_, ext)| {
                !ext.is_empty() && ext.len() <= 10 && ext.chars().all(|c| c.is_ascii_alphanumeric())
            })
            .unwrap_or(false);
        if has_sep || has_ext || std::path::Path::new(s).exists() {
            return Some(s.to_string());
        }
        return None;
    }

    if s.contains(char::is_whitespace) {
        return None;
    }

    let has_sep = s.contains('/') || s.contains('\\');
    let has_ext = s
        .rsplit_once('.')
        .map(|(_, ext)| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or(false);

    if (has_sep && has_ext) || std::path::Path::new(s).exists() {
        return Some(s.to_string());
    }

    None
}

fn looks_like_file_path(s: &str) -> bool {
    if s.contains("://") {
        return false;
    }
    s.starts_with('/')
        || s.starts_with('~')
        || s.starts_with("./")
        || s.starts_with("../")
        || s.contains('/')
        || (s.len() >= 2 && s.as_bytes()[1] == b':')
        || has_file_extension(s)
}

/// 解析不带引号的 `@path` token。Claude Code 允许把文件引用贴在中文文案中，例如
/// `@scripts/release/appstore-release.sh的执行过程`。先借助会话 cwd 找最长真实路径；
/// 找不到时，仍会在「ASCII 扩展名 + 中文/标点后缀」处分界，避免后缀被吞进文件名。
pub fn inline_at_file_path(token: &str, cwd: Option<&Path>) -> Option<String> {
    if token.is_empty() || token.contains("://") {
        return None;
    }

    if let Some(prefix) = longest_existing_path_prefix(token, cwd) {
        return Some(prefix.to_string());
    }

    let candidate = trim_inline_path_suffix(token);
    looks_like_file_path(candidate).then(|| candidate.to_string())
}

fn longest_existing_path_prefix<'a>(token: &'a str, cwd: Option<&Path>) -> Option<&'a str> {
    let mut boundaries: Vec<usize> = token.char_indices().map(|(idx, _)| idx).collect();
    boundaries.push(token.len());
    for end in boundaries.into_iter().rev() {
        if end == 0 {
            continue;
        }
        let candidate = &token[..end];
        if !looks_like_file_path(candidate) {
            continue;
        }
        let path = expand_home(candidate);
        let exists = if path.is_absolute() {
            path.exists()
        } else if let Some(cwd) = cwd {
            cwd.join(&path).exists()
        } else {
            path.exists()
        };
        // 仅用「真实文件」来裁掉粘在路径后的文案。若允许任意已存在父目录，
        // `@/a/one.txt` 这类不存在路径会被错误缩成根目录 `/`。目录引用只在
        // token 本身恰好就是该目录时接受。
        let is_file = if path.is_absolute() {
            path.is_file()
        } else if let Some(cwd) = cwd {
            cwd.join(&path).is_file()
        } else {
            path.is_file()
        };
        if exists && (is_file || candidate == token) {
            return Some(candidate);
        }
    }
    None
}

fn trim_inline_path_suffix(token: &str) -> &str {
    let bytes = token.as_bytes();
    for (dot, byte) in bytes.iter().enumerate().rev() {
        if *byte != b'.' {
            continue;
        }
        let mut end = dot + 1;
        while end < bytes.len() && bytes[end].is_ascii_alphanumeric() {
            end += 1;
        }
        let ext_len = end - (dot + 1);
        if !(1..=8).contains(&ext_len) || end == bytes.len() {
            continue;
        }
        let next = token[end..].chars().next().expect("suffix is non-empty");
        if !next.is_ascii_alphanumeric() && !matches!(next, '/' | '\\' | '.' | '_' | '-') {
            return &token[..end];
        }
    }
    token
}

fn has_file_extension(s: &str) -> bool {
    match s.rsplit_once('.') {
        Some((stem, ext)) => {
            !stem.is_empty()
                && (1..=8).contains(&ext.len())
                && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }
        None => false,
    }
}

fn expand_home(path: &str) -> PathBuf {
    if path.starts_with("~/") || path.starts_with("~\\") {
        if let Some(home) = dirs::home_dir() {
            return home.join(&path[2..]);
        }
    }
    PathBuf::from(path)
}

pub fn is_image_file(path: &Path) -> bool {
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        is_image_ext_str(ext)
    } else {
        false
    }
}

fn is_image_ext_str(ext: &str) -> bool {
    let ext_lower = ext.to_lowercase();
    matches!(
        ext_lower.as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "tiff" | "ico"
    )
}

#[cfg(test)]
mod path_lifting_tests {
    use super::*;

    #[test]
    fn visible_deferred_follow_up_strips_only_the_exact_control_wrapper() {
        let wrapped = "[Deferred follow-up from the user]\n\
Continue and fully complete the original task already in progress first.\n\
Do not interrupt it, change its direction, or begin the follow-up below yet.\n\
Only after the original task is complete, process this follow-up in the order received:\n\
<follow-up>\n当前时间\n</follow-up>";
        assert_eq!(visible_deferred_follow_up(wrapped), "当前时间");
        assert_eq!(
            visible_deferred_follow_up("<follow-up>普通文本</follow-up>"),
            "<follow-up>普通文本</follow-up>"
        );
    }

    #[test]
    fn test_parse_line_as_path() {
        assert_eq!(
            parse_line_as_path("/var/folders/pic.png").unwrap(),
            "/var/folders/pic.png"
        );
        assert_eq!(
            parse_line_as_path("~/.config/config.json").unwrap(),
            "~/.config/config.json"
        );
        assert_eq!(parse_line_as_path("@\"/abs/path\"").unwrap(), "/abs/path");
        assert_eq!(parse_line_as_path("not_a_path"), None);
    }

    #[test]
    fn test_lift_paths_from_text() {
        let text =
            "/var/folders/some_image.png\n\nSome normal message\n/Users/jerry/document.pdf\nDone.";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 2);
        // 文件不存在时一律 file chip（不渲染破碎图片）
        assert_eq!(blocks[0].kind, "file");
        assert_eq!(
            blocks[0].file_path.as_deref().unwrap(),
            "/var/folders/some_image.png"
        );
        assert_eq!(blocks[1].kind, "file");
        assert_eq!(
            blocks[1].file_path.as_deref().unwrap(),
            "/Users/jerry/document.pdf"
        );
        assert_eq!(remaining, "Some normal message\n\nDone.");
    }

    #[test]
    fn test_lift_bracket_paths() {
        let text = "@[src/App.vue] 这个文件很大，有什么规划？";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "file");
        assert_eq!(blocks[0].file_path.as_deref().unwrap(), "src/App.vue");
        assert_eq!(remaining, "这个文件很大，有什么规划？");
    }

    #[test]
    fn test_lift_mid_prompt_at_file_reference_keeps_text_and_adds_file() {
        let text = "分析@scripts/release/appstore-release.sh的执行过程和调用命令";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].file_path.as_deref(),
            Some("scripts/release/appstore-release.sh")
        );
        assert_eq!(remaining, text);
    }

    #[test]
    fn test_lift_keeps_xcode_build_log_paths_in_text() {
        let text = "运行报错：Error output from Xcode build:\n↳\n    ** BUILD FAILED **\n\nXcode's output:\n↳\n    Writing result bundle at path:\n        /var/folders/a/T/flutter_tools.3xroe7/flutter_ios_build_temp_dir/temporary_xcresult_bundle\n    /Users/wuchao/develop/flutter/sales-api-app-test/ios/Pods/Pods.xcodeproj: warning";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert!(blocks.is_empty());
        assert_eq!(remaining, text);
    }

    #[test]
    fn test_post_process_deduplicates_existing_mid_prompt_file_tag() {
        let text = "分析@scripts/release/appstore-release.sh的执行过程和调用命令";
        let mut msgs = vec![Msg {
            role: "user".to_string(),
            blocks: vec![
                Block {
                    kind: "file".to_string(),
                    file_path: Some("scripts/release/appstore-release.sh".to_string()),
                    ..Default::default()
                },
                text_block("text", text),
            ],
            ..Default::default()
        }];
        post_process_session_msgs(&mut msgs);
        assert_eq!(msgs[0].blocks.len(), 2);
        assert_eq!(msgs[0].blocks[1].text.as_deref(), Some(text));
    }

    #[test]
    fn test_lift_leading_at_file_reference_but_keeps_prompt_body() {
        let text = "@scripts/release/appstore-release.sh 分析执行过程和调用命令";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "file");
        assert_eq!(
            blocks[0].file_path.as_deref(),
            Some("scripts/release/appstore-release.sh")
        );
        assert_eq!(remaining, "分析执行过程和调用命令");
    }

    #[test]
    fn test_lift_trailing_at_file_references_are_not_repeated_in_prompt() {
        let text = "我加了几个附件，回答我即可 @\"/tmp/one.xlsx\" @\"docs/two.md\" @\".vscode/launch.json\"";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 3);
        assert_eq!(remaining, "我加了几个附件，回答我即可");
    }

    #[test]
    fn test_lift_multiple_leading_at_file_references() {
        let text = "@\"src/a.ts\" @\"src/b.ts\" 比较两个文件";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].file_path.as_deref(), Some("src/a.ts"));
        assert_eq!(blocks[1].file_path.as_deref(), Some("src/b.ts"));
        assert_eq!(remaining, "比较两个文件");
    }

    #[test]
    fn test_lift_glued_paths() {
        let text = "这个应该像Claude一样/var/folders/8h/ddvbjjrn74q1v55wywphwkdc0000gn/T/clipboard-2026-07-05-122944-350E5680.png";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].kind, "file");
        assert_eq!(blocks[0].file_path.as_deref().unwrap(), "/var/folders/8h/ddvbjjrn74q1v55wywphwkdc0000gn/T/clipboard-2026-07-05-122944-350E5680.png");
        assert_eq!(remaining, "这个应该像Claude一样");
    }

    #[test]
    fn test_lift_mix_bracket_and_raw_path() {
        let text = "@[.env.local] /var/folders/8h/ddvbjjrn74q1v55wywphwkdc0000gn/T/clipboard-2026-07-05-123507-F13776EE.png\n\nhello";
        let (blocks, remaining) = lift_paths_from_text(text);
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].kind, "file");
        assert_eq!(blocks[0].file_path.as_deref().unwrap(), ".env.local");
        assert_eq!(blocks[1].kind, "file");
        assert_eq!(blocks[1].file_path.as_deref().unwrap(), "/var/folders/8h/ddvbjjrn74q1v55wywphwkdc0000gn/T/clipboard-2026-07-05-123507-F13776EE.png");
        assert_eq!(remaining, "hello");
    }
}
