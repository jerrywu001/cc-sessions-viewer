# 本地 Web Server 模式开发计划

日期：2026-08-22  
状态：规划中

## 结论与目标

本项目保持两个并存的产品入口，而不是将 Tauri 客户端改造成网页：

| 入口 | 启动方式 | 使用场景 | 保留能力 |
| --- | --- | --- | --- |
| Desktop | 现有 Tauri 应用 | 完整原生体验 | 托盘、桌面宠物、原生窗口、系统文件选择、外部终端、自动更新等。 |
| Local Web | `npx cc-sessions-viewer serve` | 浏览器访问本机会话数据 | 会话、搜索、统计、导出、CLI/PTY 等由本机守护进程执行。 |

Web 模式不是可部署到 Vercel、GitHub Pages 或公网的静态站点。浏览器没有权限读取 `~/.claude`、`~/.codex`、`~/.kimi-code`、`~/.pi`，也不能自行执行 CLI、监听 JSONL、操作 worktree 或启动 PTY。`serve` 必须在用户机器上启动本地后端，浏览器只连接该后端。

目标是让 Desktop 与 Local Web 复用同一套 Agent 解析、会话管理、统计、回收站、CLI 和终端核心逻辑；两端只在调用传输层和原生 UI 能力上分叉。

## 当前问题与改造原则

当前 `src/api.ts` 直接使用 `@tauri-apps/api/core` 的 `invoke()`；后端能力以大量 `#[tauri::command]` 注册在 `src-tauri/src/lib.rs`。直接用 Vite 启动页面时没有 Tauri invoke bridge，因此项目/会话/API 请求全部失败。

本计划遵循以下原则：

1. **核心逻辑只保留一份。** Agent source、统计、搜索、回收站、worktree、CLI/PTY 不复制成 Node 实现。
2. **Tauri 不退化。** 现有 Desktop 命令继续可用，原生功能不为 Web 让步。
3. **Web 后端默认私有。** 仅监听 loopback，所有浏览器请求有访问令牌和 Origin 校验。
4. **按 API 域渐进迁移。** 每次抽取一个可验收域，避免一次改动近百个命令。
5. **Web 首版先解决会话工作流。** 原生窗口、托盘和宠物不阻塞浏览器版上线。

## 目标架构

```text
                        +-------------------------+
                        |  Rust domain services   |
                        | agents / stats / trash  |
                        | search / pty / git      |
                        +------------+------------+
                                     |
                   +-----------------+-----------------+
                   |                                   |
          +--------v---------+                 +-------v--------+
          | Tauri adapter    |                 | HTTP/WS adapter |
          | invoke + events  |                 | local server    |
          +--------+---------+                 +-------+--------+
                   |                                   |
          Desktop Vue build                    Browser Vue build
          @tauri transport                     HTTP/WS transport
```

### Rust 分层

1. 新建面向业务的 service 层（建议 `src-tauri/src/services/`），把命令参数校验、调用 Agent source、写入动作和错误类型从 Tauri command 函数中抽出。
2. 保留 `lib.rs` 作为 Desktop adapter：Tauri command 只负责参数转换、调用 service、通过 Tauri event 发事件。
3. 新增独立 server binary（建议 `src-tauri/src/bin/cc-sessions-viewer-server.rs`）和 HTTP/WS adapter。建议采用 Axum + Tokio；服务静态托管 Web build，并暴露 API 与 WebSocket。
4. 抽象事件 sink：统计进度、session append、终端输出、turn hook 状态等业务事件不直接依赖 `tauri::AppHandle`，而是投递到 `EventSink`。Tauri sink 负责 emit，Web sink 负责按连接推送 WebSocket。
5. 服务 API 不暴露任意文件读写或任意命令执行接口；每个 endpoint 对应现有的明确业务操作并做路径/agent/会话验证。

### 前端分层

1. 将现有 `src/api.ts` 重构成仅导出领域 API 的 facade；不允许业务组件直接 import Tauri `invoke`。
2. 新增 `src/api/tauriTransport.ts` 与 `src/api/httpTransport.ts`。前者维持现有行为，后者调用 loopback HTTP 和 WebSocket。
3. 用编译期目标或启动注入选择 transport：Desktop build 选 Tauri，Web build 选 HTTP。Vue 页面、types、格式化、导出视图和 agent UI 尽量完全复用。
4. 将 dialog、opener、notification、webview、updater、tray 等原生调用封装为 `platform` 能力接口。Web 不支持时必须显示明确状态或使用浏览器可行替代，不能在运行时抛出 Tauri 未初始化错误。

## API 与安全约束

### HTTP/WS 契约

- REST 用于项目、会话、搜索、统计启动、导出、改名、回收站、CLI 检查等请求/响应操作。
- WebSocket 用于 session append、stats progress/done/error、PTY 输出/状态、turn 状态、后台任务和服务生命周期事件。
- endpoint 与 TypeScript type 通过共享 schema 或生成代码对齐；禁止手写两套不受约束的 JSON 类型。
- 支持 API version，例如 `/api/v1/...`；npm server 与 Web 静态资源版本不匹配时返回明确错误，不能静默调用不兼容 API。

### 本地安全模型

1. 默认绑定 `127.0.0.1` 和 `::1`，不提供 `0.0.0.0`、局域网或公网模式。
2. 每次启动生成至少 256 bit 随机 token；token 仅放在启动浏览器 URL fragment 或一次性本地启动页中，不写入日志、历史记录或页面 query。
3. 对 HTTP 和 WebSocket 同时验证 token、Origin、Host 和同源静态资源来源；拒绝非 loopback peer。
4. 仅服务启动时可选 `--port`，默认随机可用端口；已有 server 的发现/复用通过本机受限 IPC 或带权限的 state 文件完成，不能凭端口扫描盲连。
5. 改名、删除、恢复、永久删除、worktree 删除、启动终端等写操作沿用现有确认和路径校验；Web API 不降低安全边界。
6. session 文件正在被 CLI 写入时，继续使用现有 snapshot/revision/mtime 防护；Desktop 和 Local Web 同时运行也必须共享同一套写前复核规则。

## 分阶段开发计划

### 阶段 0：契约盘点与基线

涉及：`src/api.ts`、`src-tauri/src/lib.rs`、`src/types.ts`、现有 Rust/Vitest 测试。

1. 清点所有 Tauri command、event、插件调用和直接 Tauri import，按领域标记为：会话只读、搜索/统计、会话写入、文件/导出、CLI/PTY、窗口/托盘/宠物。
2. 为每个准备进入 Web 的 command 写下输入、输出、错误、权限、事件和路径边界，形成 API inventory 文档和 endpoint schema。
3. 为现有 Desktop 行为补足高风险回归测试：每个 Agent 的 list/read/search、回收站、rename、统计 stream、终端状态、路径校验。
4. 确定 server 运行时依赖、跨平台端口/浏览器打开策略、npm package 名称和最低支持 Node 版本。

验收：没有任何 endpoint 开发前，团队可以回答每个 command 是否属于 Web 首版、其客户端替代方式及安全风险；Desktop 基线测试通过。

### 阶段 1：抽取共享 service 层

涉及：`src-tauri/src/lib.rs`、`src-tauri/src/agents/`、`src-tauri/src/stats/`、`src-tauri/src/trash.rs`、新增 `src-tauri/src/services/`。

1. 优先抽取只读域：项目发现、分页会话、会话详情、session usage、last prompt、搜索和 Agent stats。
2. 将 command 里的 Tauri 类型、AppHandle、emit 调用移出 service；service 接收明确的 domain 参数并返回 domain result/error。
3. 定义 `EventSink`、`TerminalSink` 等最小 trait；先让 Tauri adapter 包装为当前事件名，确保 Desktop 前端无需同步大改。
4. 保留现有 Agent source 的路径验证、缓存、JSONL snapshot 与 OpenCode SQLite 并发策略，禁止为 Web 拆层时削弱它们。
5. 为每个抽取 service 增加 Rust 单元或集成测试，Tauri command 测试只验证 adapter 映射。

验收：Desktop 行为和事件名称不变；service 层不直接引用 Tauri command API，至少只读会话域可由非 Tauri 测试调用。

### 阶段 2：本地 HTTP/WS Server 骨架

涉及：`src-tauri/Cargo.toml`、新增 server binary/router/auth/static modules、release scripts。

1. 以 Axum/Tokio 建立 loopback-only server，提供 `/health`、版本协商、token middleware、静态资源服务和统一 JSON error 格式。
2. 实现 server 生命周期：启动、端口选择、浏览器打开、优雅停止、单实例保护和崩溃后的 state 清理。
3. 落地 WebSocket 握手、认证、连接心跳、订阅模型和 backpressure 限制；断线重连不得重复执行写操作。
4. 将阶段 1 的只读 service 暴露为 `/api/v1/projects`、`/sessions`、`/session/read`、`/search`、`/usage` 等 endpoint。
5. 加入 localhost 攻击面测试：错误 token、跨 Origin、非 loopback Host、重复 token、畸形 JSON、大 payload、慢消费者和未授权 WS。

验收：无需 Tauri 窗口，server 可启动且浏览器/HTTP 客户端只能在本机经 token 读取项目、会话和搜索结果。

### 阶段 3：Web 前端可读 MVP

涉及：`src/api.ts` 拆分、`src/main.ts`、新增 Web entry/build config、会话列表/详情/搜索/统计视图。

1. 建立 transport facade 和 HTTP transport；页面不再直接依赖 `invoke`、`listen`、`convertFileSrc`。
2. 增加 Web build target，server 托管 build 产物；Desktop build 仍以现有 Vite/Tauri 流程构建。
3. 打通 Agent 切换、项目列表、分页会话、详情渲染、图片渲染、全局搜索、统计与价格展示。
4. 把 stats progress 和 session live append 接到 WebSocket；断线后以安全重读同步，而不是假设增量永远完整。
5. Web 模式隐藏或禁用纯原生入口，所有被禁用操作提供原因，不显示无响应按钮。

验收：`npx ... serve` 后，可在 Chrome/Safari/Edge 浏览会话、打开详情、搜索、查看统计；Claude、Codex、Grok、Kimi、Agy、OpenCode 的历史渲染与 Desktop 一致。

### 阶段 4：会话操作与导出

涉及：trash、rename、export、background media、worktree service 与对应前端操作。

1. 暴露 rename、软删、恢复、永久删除、清空回收站、批量操作；HTTP request 必须保留 Desktop 的确认语义和 error 文案。
2. 导出改为 server 在用户指定目录生成文件，并提供浏览器下载 fallback；默认不授予任意目录写入，路径必须经允许的业务 flow 选择/校验。
3. 文件打开、Reveal in Finder、外部 opener 在 Web 首版改为：下载、复制路径或明确“不支持”；不借由任意 path API 绕过浏览器边界。
4. 处理 Desktop 与 Web 同时读写的刷新和冲突提示，特别覆盖 Kimi directory session、OpenCode SQLite、Pi append-only JSONL、Codex archive 和 worktree hard delete。
5. 端到端验证删除/恢复/导出后，列表、详情 tab、缓存和 WebSocket 状态保持一致。

验收：核心会话管理操作可在 Web 完整使用，所有写操作都经现有安全校验，失败时不会留下半移动或半改名的数据。

### 阶段 5：CLI、终端与运行状态

涉及：`cli_env.rs`、`agent_command.rs`、`pty.rs`、`turn.rs`、`src/terminals.ts`、`TerminalStrip.vue`。

1. 将 CLI 环境检测、诊断和安装能力接到 Web API；一键升级继续只对现有安全、非交互的 CLI 开放。
2. 将 PTY 生命周期、输入、resize、输出、退出码和重连协议映射到专用 WebSocket channel；每个 terminal 连接必须绑定已授权的本机 browser session。
3. 保留已有 shell quoting、Windows PowerShell PATH 刷新、resume/new command、agent launch args 和 worktree cwd 规则。
4. terminal 断线不立刻杀进程；提供恢复/关闭语义、资源上限、空闲清理和服务退出时的 child cleanup。
5. 将 turn hooks、terminal running state、live notification 和 tab 状态经 EventSink 同时发送给 Tauri 与 Web。

验收：Web 可新建/恢复内嵌终端，实时输出和输入稳定；Windows/macOS/Linux 的 CLI 解析、中文/空格路径、重连和退出清理有自动化或人工矩阵验证。

### 阶段 6：npm 分发、跨平台发布与可观测性

涉及：新增 npm launcher/package、CI release workflow、README 与诊断。

1. 发布 npm launcher 包，并按 `darwin-arm64`、`darwin-x64`、`win32-x64`、`linux-x64` 等拆分 optional dependency 的预编译 server binary；`npx` 不要求用户安装 Rust toolchain。
2. launcher 校验 Node 版本、平台、CPU 架构、binary hash/签名和 server 版本；不匹配时给出安装/升级说明。
3. `serve` 支持 `--open`、`--port`、`--no-open`、`--data-dir` 等最小参数；不提供默认远程监听参数。
4. CI 产出 Desktop 安装包和 npm platform packages，跑 server API 合约测试、Web E2E、Rust tests、类型检查和 smoke test。
5. 增加不含会话正文、token、认证信息和绝对敏感路径的诊断日志；文档说明数据始终留在本机、如何停止服务和如何反馈问题。

验收：一台没有仓库和 Rust 环境的干净机器可通过 `npx` 启动本地 Web；Desktop 安装包与 Web 包可独立发布、升级和回滚。

### 阶段 7：完整性、性能和发布门槛

1. 对全部已纳入 Web 的 Agent、文件型/目录型/SQLite session、长会话、两张以上截图、工具卡、AskUserQuestion、Windows 路径与多模型统计做 Desktop/Web 快照对比。
2. 压测大项目会话发现、全文搜索、统计扫描、多个浏览器 tab、终端高频输出和 WebSocket 慢客户端。
3. 安全审计 loopback server：CSRF、DNS rebinding、Origin/Host 混淆、token 泄漏、目录穿越、任意命令、日志敏感信息和端口劫持。
4. 建立兼容矩阵：macOS Apple Silicon/Intel、Windows、Linux；Chrome、Safari、Edge；Desktop/Web 同时运行。
5. 发布前冻结 API v1，编写升级与故障恢复说明，并让 Web MVP 以实验性 feature 进入一个小版本。

## Web 首版范围

首版必须包含：

- 六个 Agent 的项目、会话发现、详情渲染、图片/工具/交互卡、搜索和分页。
- 用量、统计、模型价格和导出。
- 重命名、回收站删除/恢复、CLI 环境检查。
- Local Web 启动、认证、浏览器打开、实时会话刷新。
- 文档、跨平台 npm 二进制分发和安全测试。

首版可延后到后续阶段：浏览器内嵌终端、复杂 worktree 管理、所有写操作的完整 parity。它们应在 service/API 契约已经稳定后接入，不能为了赶 MVP 绕开安全层。

## 暂不做的能力与原因

| 暂不做 | 原因 | 后续条件 |
| --- | --- | --- |
| 公网、局域网或 `0.0.0.0` 访问 | 服务具备读会话、删文件、启动终端的本机权限；远程访问会引入身份、TLS、授权、审计和威胁模型。 | 单独设计账户、设备配对、TLS、细粒度授权和安全审计后再立项。 |
| 云端托管/多用户协作/共享链接 | 与“数据和 CLI 仅在本机”的产品边界相反，也会涉及隐私、同步冲突和服务端成本。 | 有独立云产品需求、数据协议和隐私方案时再做。 |
| 将 Web 版部署为纯静态网站 | 静态页面不能安全读取本机 Agent 数据或运行 CLI；即使页面能加载也没有后端能力。 | 始终需要用户本机 server，除非未来引入受控远程 agent daemon。 |
| Desktop 托盘、桌面宠物、原生窗口标题栏 | 它们是操作系统窗口能力，不是浏览器能力；强行模拟不会给 Web 用户带来等价体验。 | Desktop 保持现有实现；Web 不复制。 |
| 原生应用自动更新 | npm/npx 有自己的版本与包管理模型，复用 Tauri updater 会造成双重更新和状态混乱。 | npm package 通过 npm 更新；Desktop 继续 Tauri updater。 |
| 浏览器直接调用任意本地文件/命令 API | 这会把 Local Web Server 变成本机 RCE/文件泄露接口。 | 不做；只允许经过明确业务 endpoint 的受限操作。 |
| Web 首版 GUI Chat parity | 当前 GUI Chat 本身仅覆盖部分 Agent，且涉及长连接、权限、附件、模型选择和进程恢复；会拖慢核心历史浏览交付。 | Local Web 的会话/PTY/事件层稳定后，以 Claude/Codex 为首批单独设计。 |
| Web 首版完整 worktree 管理 | 创建/删除 worktree 影响真实 Git 工作区和分支，且与活跃 CLI/terminal 并发时风险高。 | 阶段 4 的写操作与 revision/占用保护通过后再接。 |
| 浏览器中系统级文件选择、Finder/Explorer 打开 | 浏览器无法可靠提供等价的原生路径选择与外部程序启动。 | 首版使用下载、上传/拖放、复制路径；必要时再由受限 server endpoint 打开系统对话框。 |
| 让 `npx` 在用户机器编译 Rust | 首次启动慢、失败率高、要求安装编译工具链，不符合 CLI 分发体验。 | 不做；只发布签名/校验过的预编译平台二进制。 |

## 工期与人员假设

以一名熟悉现有 Rust/Tauri/Vue 代码的工程师为基准：

| 交付目标 | 预估时间 |
| --- | --- |
| 阶段 0-2：服务分层、loopback API、安全骨架 | 1.5-2.5 周 |
| 阶段 3：可读 Web MVP | 1.5-2 周 |
| 阶段 4：写操作与导出 | 1-1.5 周 |
| 阶段 5：PTY/实时状态 | 1.5-2.5 周 |
| 阶段 6-7：npm 分发、跨平台 QA、安全与发布 | 1.5-2 周 |

完整核心功能 parity 预计 6-10 周。若只交付阶段 0-3 的只读 Web MVP，预计 3-4 周；该版本仍需完整完成 loopback token 与 Origin 安全模型，不能以“开发服务器”替代。

## 最终验收清单

1. Desktop 与 `npx ... serve` 可以独立启动，且可同时访问同一份本机会话数据，不互相损坏数据。
2. Web 未加载 Tauri bridge 时，所有已承诺操作都走 HTTP/WS transport，不出现 `invoke is not available` 一类错误。
3. 未携带有效 token、跨 Origin 或非 loopback 的请求无法读取任何会话或执行任何动作。
4. Desktop 和 Web 对同一会话的消息、图片标签、工具卡、搜索结果和统计数据一致。
5. Web 终端上线后，断线、重连、关闭 browser tab、服务退出和 child process 清理行为可预测且有测试。
6. `npx` 在支持的平台无需 Rust toolchain 即可启动，并在不支持架构、二进制缺失、端口占用和版本不匹配时给出可操作错误。
7. 文档明确区分 Desktop 专属能力、Web 支持范围、本机数据边界和停止服务方法。
