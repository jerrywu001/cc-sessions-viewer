---
title: Agent 兼容范围、验证矩阵与合成会话样例
description: 核查七种会话格式、恢复及内置对话能力，区分源码核对与真实CLI验证，下载经过命令回归测试的合成JSONL和SQLite样例。
---

# 支持哪些 coding agent 格式

Sessions Viewer 支持下表布局。**源码核对和合成命令测试，不等于真实 CLI 端到端认证。** 除 opencode 行已补充本次 v2 存储适配外，此表描述 2026-10-05 核对的 0.6.0 实现；不承诺每个 CLI 版本都兼容。

## 能力与验证矩阵 {#support-matrix}

七种都有历史浏览、搜索、导出和终端恢复；内置对话只支持 Claude Code、Codex。恢复需要安装并配置对应 CLI，语法见[恢复命令对照](/zh/agents/#resume-commands)。

| Agent | 覆盖格式 | 内置对话 | 验证证据 | 本轮端到端验证的 CLI 版本 |
| --- | --- | --- | --- | --- |
| [Claude Code](/zh/agents/claude-code) | JSONL `message.content` 字符串／块 | 支持 | 合成抽取 + [Rust 解析投影](#offline-parser-tests) | 未验证 |
| [Codex](/zh/agents/codex) | `session_meta`、`event_msg`、`response_item` | 支持 | 合成 `cwd`／文本 + [Rust 解析投影](#offline-parser-tests) | 未验证 |
| [Grok Build](/zh/agents/grok-build) | `updates.jsonl` 中 ACP 更新 | 不支持 | 合成对象／数组片段 | 未验证 |
| [Kimi Code](/zh/agents/kimi-code) | 主 wire 事件；适配器另有旧格式回退 | 不支持 | 仅合成主事件 | 未验证 |
| [Pi](/zh/agents/pi) | 会话头及消息／树条目 | 不支持 | 合成全部存储分支的文本 | 未验证 |
| [Antigravity CLI](/zh/agents/antigravity-cli) | `transcript*.jsonl` step 记录 | 不支持 | 合成多行 XML 提取 | 未验证 |
| [opencode](/zh/agents/opencode) | 旧版 SQLite 及 v2 `session_v2` / `session_message` / 可选 `session_part` | 不支持 | 合成只读 SQL + Rust 解析回归 | 未验证 |

提供 macOS、Windows、Linux 安装包；Shell 示例使用 macOS/Linux 语法，不代表逐系统 GUI 认证。支持的 Antigravity 格式没有用量字段。旧布局、自定义目录、图片、工具、分支和服务商行为需结合对应参考页及[排障指南](/zh/guide/troubleshooting)判断。

## 下载安全、可复现的样例 {#synthetic-samples}

这些是最小**合成文本提取样例**，不是实际会话或完整 CLI 可恢复文件。可选图片／工具字段不完整，不要当作真实历史导入或恢复。路径与提问均为虚构。JSONL／SQL 及<a href="/examples/expected.json" download>预期输出清单</a>就是 `npm run docs:test` 检查的输入。

下载后将对应 agent 参考页的文本提取命令文件参数改为样本名，预期行顺序如下：

| Agent | 下载 | 预期文本 |
| --- | --- | --- |
| Claude Code | <a href="/examples/claude-code.jsonl" download>claude-code.jsonl</a> | `Question`、`Answer`、`Legacy string` |
| Codex | <a href="/examples/codex.jsonl" download>codex.jsonl</a> | `Question`、`Answer`；元数据 `cwd` 为 `/synthetic/project` |
| Grok Build | <a href="/examples/grok-build.jsonl" download>grok-build.jsonl</a> | `Question`、`Answer` |
| Kimi Code | <a href="/examples/kimi-code.jsonl" download>kimi-code.jsonl</a> | `Question`、`Answer`、`Legacy string` |
| Pi | <a href="/examples/pi.jsonl" download>pi.jsonl</a> | `Question`、`Answer`、`Other branch`，不限当前分支 |
| Antigravity CLI | <a href="/examples/antigravity-cli.jsonl" download>antigravity-cli.jsonl</a> | `Question`、`on two lines`，是一条多行提问 |
| opencode（旧版） | <a href="/examples/opencode.sql" download>opencode.sql</a> | 文本查询返回 `user` 与 `Question` |
| opencode（v2） | <a href="/examples/opencode-v2.sql" download>opencode-v2.sql</a> | 原始行按 `user`、`assistant` 排序；解析器显示 `Question`、分表中的 `Answer` |

例如下载 `claude-code.jsonl` 后：

```bash
jq -r 'select(.type=="user" or .type=="assistant")
  | .message.content
  | if type=="string" then . else .[]? | select(.type=="text") | .text end' \
  claude-code.jsonl
```

```text
Question
Answer
Legacy string
```

opencode 样例 SQL 会创建精简表结构，只能载入**新建临时数据库**，不能载入真实 opencode 库：

```bash
temp_dir="$(mktemp -d)"
sqlite3 "$temp_dir/synthetic.db" < opencode.sql
sqlite3 "file:$temp_dir/synthetic.db?mode=ro" \
  "SELECT json_extract(data, '$.text') FROM part WHERE json_extract(data, '$.type') = 'text';"
```

预期输出 `Question`。回归还检查父会话列表不包含子 agent、成本聚合计入子 agent、只读查询后样本数据库未变化；不验证服务商账单或真实 opencode 迁移。

v2 样例保留上游会话相关表的子集，并非完整原生数据库，也只能载入新建临时库。[v2 参考页](/zh/agents/opencode)说明查询方法，以及重命名、回收和还原暂不支持的边界。

## 离线解析测试证明了什么 {#offline-parser-tests}

区分三个验证范围：

1. **`jq`／SQL 抽取**：`npm run docs:test` 对上面的小样本运行文档命令，也检查内容／SEO 回归和截图完整性。命令输出不等于应用解析器输出。
2. **真实 Rust 文件解析器**：2026-10-06 在 macOS 对 0.6.0 的 Claude Code、Codex 解析与消息后处理运行两个隔离测试并通过。与公开预期投影对照角色、文字、Claude thinking、工具名／参数／ID／结果及消息顺序，并检查临时输入未被修改。Codex 传入空标题索引，不加载用户真实索引。不扫描 home，不执行 CLI、工具或服务商请求。
3. **真实 CLI／GUI 端到端验收**：本轮尚未执行。样例不能认证图片行为、浏览／搜索／导出／恢复、已安装 CLI 版本或服务商行为；矩阵没有新增真实运行验证的 CLI 版本。

| 离线解析样例 | 预期消息 |
| --- | --- |
| <a href="/examples/claude-code-adapter.jsonl" download>claude-code-adapter.jsonl</a> | 用户提问 → 助手 thinking／文本／工具调用 → 用户工具结果 → 旧式用户字符串 |
| <a href="/examples/codex-adapter.jsonl" download>codex-adapter.jsonl</a> | 用户提问 → 助手工具调用 → 用户工具结果 → 助手回答；重复 response 文本和无关元数据被忽略 |

下载<a href="/examples/adapter-expected.json" download>adapter-expected.json</a>查看精确核对字段。这些较完整的合成解析样例仍不是可导入或恢复的完整 CLI 会话；包括 `echo synthetic` 在内的工具参数只是数据，不执行命令。

在仓库根目录运行，需预先缓存 Rust／Tauri 构建依赖：

```bash
npm run docs:test
npm run docs:test:adapters
```

另新增一项隔离的 OpenCode v2 解析测试，读取上方公开 SQL 样本，检查 NULL 标题、Windows 格式路径、消息顺序、分表正文、记录成本与 token；不执行 OpenCode 或 GUI。

适配器命令为 `cargo test --manifest-path src-tauri/Cargo.toml --lib --locked --offline docs_public_fixture`，运行这三项测试；缺少 crate 缓存时直接失败，不联网下载。已有 Rust CI 包含它们，Node 文档任务不依赖 Rust。这是可复现的本地证据，不证明生产发布或收录。

## 格式变化怎么反馈 {#report-change}

提供应用版本、CLI 版本、系统、记录结构、自定义根目录与失败操作。用最小合成／脱敏结构，不公开凭据或整个私人数据库。只有实际跑通对应版本和操作后，才把 CLI 版本标成真实运行验证。

依据：[适配器源码版本 22fefc6](https://github.com/jerrywu001/cc-sessions-viewer/tree/22fefc6/src-tauri/src/agents)。另见[隐私说明](/zh/guide/privacy)、[安装](/zh/guide/install)和[项目维护说明](/zh/guide/about)。
