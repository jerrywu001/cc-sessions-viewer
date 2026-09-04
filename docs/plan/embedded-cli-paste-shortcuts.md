# 内嵌 CLI 粘贴快捷键统一开发计划

## 当前实际行为

以下数据来自 `快捷键整理表.md`，记录的是当前实测行为，不是目标实现方案。范围仅限软件的内嵌 session CLI；内嵌终端贴文字正常。

| 平台 | Agent | 操作 | 当前快捷键 | 备注 | 修改后效果 |
| --- | --- | --- | --- | --- | --- |
| Windows/Linux | codex | 贴图 | `Alt + V` | 应该是 `Ctrl + V` | |
| Windows/Linux | codex | 贴文字 | `Ctrl + V` | 正确 | |
| Windows/Linux | claude code | 贴图 | `Alt + V` | 应该是 `Ctrl + V` | |
| Windows/Linux | claude code | 贴文字 | `Alt + V` | 应该是 `Ctrl + V` | |
| Windows/Linux | agy | 贴图 | `Ctrl + V` / `Alt + V` | 希望统一使用 `Ctrl + V` | |
| Windows/Linux | agy | 贴文字 | `-` | `Ctrl/Alt + V` 都不行，只能右键点击粘贴，需要使用 `Ctrl + V` | |
| Windows/Linux | pi | 贴图 | `Alt + V` | 应该是 `Ctrl + V` | |
| Windows/Linux | pi | 贴文字 | `Alt + V` | 应该是 `Ctrl + V` | |
| Windows/Linux | kimicode | 贴图 | `Alt + V` | 应该是 `Ctrl + V` | |
| Windows/Linux | kimicode | 贴文字 | `-` | `Ctrl/Alt + V` 都不行，只能右键点击粘贴，需要使用 `Ctrl + V` | |
| Windows/Linux | opencode | 贴图 | `Ctrl + V` | 正确 | |
| Windows/Linux | opencode | 贴文字 | `-` | `Ctrl/Alt + V` 都不行，只能右键点击粘贴，需要使用 `Ctrl + V` | |
| Windows/Linux | grokbuild | 贴图 | `Ctrl + V` / `Alt + V` | 希望统一使用 `Ctrl + V` | |
| Windows/Linux | grokbuild | 贴文字 | `Ctrl + V` / `Alt + V` | 希望统一使用 `Ctrl + V` | |
| macOS | codex | 贴图 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | codex | 贴文字 | `Command + V` | 正确 | |
| macOS | claude code | 贴图 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | claude code | 贴文字 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | agy | 贴图 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | agy | 贴文字 | `Command + V` | 正确 | |
| macOS | pi | 贴图 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | pi | 贴文字 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | kimicode | 贴图 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | kimicode | 贴文字 | `Command + V` | 正确 | |
| macOS | opencode | 贴图 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | opencode | 贴文字 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | grokbuild | 贴图 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |
| macOS | grokbuild | 贴文字 | `Command + V` / `Control + V` | 希望统一使用 `Command + V` | |

## 1. 背景与目标

本计划只针对 Sessions Viewer 的**内嵌 session CLI**。软件的内嵌终端贴文字已经正常，不在本次改动范围内；原生系统终端也不需要修改。

当前不同 Agent 的粘贴快捷键不一致：Windows/Linux 下图片常使用 `Alt + V`，文字可能使用 `Ctrl + V`、`Alt + V`，或两个快捷键都无效；macOS 下多数场景同时接受 `Command + V` 和 `Control + V`。其中 `agy`、`kimicode`、`opencode` 在 Windows/Linux 下虽然可以通过右键粘贴文字，但键盘快捷键无效。

原生终端不存在无法粘贴文字的情况，因此这些失败不能归因于系统剪贴板或终端能力。问题应定位在 Viewer 对内嵌 CLI 的键盘事件拦截、剪贴板读取、输入转发或历史 Agent 快捷键分支。

目标是统一由 Viewer 输入层决定粘贴行为：

| 平台 | 快捷键 | 剪贴板内容 | 结果 |
| --- | --- | --- | --- |
| Windows/Linux | `Ctrl + V` | 有可读取图片 | 走图片粘贴/上传流程 |
| Windows/Linux | `Ctrl + V` | 无图片但有文字 | 将文字写入当前内嵌 CLI 输入 |
| macOS | `Control + V` | 有可读取图片 | 走图片粘贴/上传流程 |
| macOS | `Command + V` | 有文字 | 将文字写入当前内嵌 CLI 输入 |

`Alt + V` 不再作为正式入口。没有可识别图片或文字时，不应吞掉快捷键，应保留原生行为。

## 2. 范围与非目标

### 本期范围

- 内嵌 session CLI 的键盘粘贴事件。
- 图片剪贴板读取、图片插入现有 CLI 输入流程。
- 文字/富文本剪贴板读取，并写入当前 CLI 输入。
- Windows、Linux、macOS 平台修饰键判断。
- codex、claude code、agy、pi、kimicode、opencode、grokbuild 的统一入口验证。
- 输入框、CLI PTY、消息发送状态和焦点状态的回归测试。

### 非目标

- 不修改原生终端的快捷键行为。
- 不修改各 Agent CLI 本身的配置或源码。
- 不继续为每个 Agent 增加独立快捷键映射。
- 不把内嵌终端的普通文字粘贴逻辑和内嵌 CLI 逻辑混在一起。
- 不把剪贴板内容写入日志、统计或会话历史，除非用户随后实际提交消息。
- 不在本期解决剪贴板权限申请、远程桌面剪贴板同步等操作系统级故障；这些情况只做可见失败提示和原生回退。

## 3. 当前事实与设计判断

### 3.1 表格反映的行为分组

Windows/Linux 当前行为：

- `codex`：图片 `Alt + V`，文字 `Ctrl + V`。
- `claude code`、`pi`、`kimicode`：图片主要为 `Alt + V`。
- `agy`、`grokbuild`：图片同时支持 `Ctrl + V` 和 `Alt + V`。
- `opencode`：图片 `Ctrl + V`。
- 文字只有 `codex` 明确支持 `Ctrl + V`；`claude code`、`pi` 使用 `Alt + V`；`agy`、`kimicode`、`opencode` 的键盘粘贴无效，只能右键。

macOS 当前行为：

- 文字多数可用 `Command + V`。
- 图片多数同时可用 `Command + V` 和 `Control + V`。
- `opencode`、`grokbuild` 的图片和文字记录均存在双快捷键现象，说明当前事件边界并不稳定。

### 3.2 关键推论

原生终端能够贴文字，且内嵌 CLI 右键粘贴也能成功，说明：

1. 系统剪贴板通常有可读取内容。
2. CLI 输入通道本身具备接收文字的能力。
3. 失败点位于键盘事件进入 Viewer/CLI 的路径，而不是“该 Agent 不支持粘贴文字”。
4. 统一方案必须在共享的内嵌 CLI 输入层实现，不能只修改某个 Agent 的启动参数。

## 4. 预期输入处理流程

### 4.1 Windows/Linux `Ctrl + V`

1. 内嵌 CLI 获得焦点时，输入层捕获 `Ctrl + V`。
2. 阻止事件继续以原始控制字符、浏览器默认粘贴或 Agent 历史 `Alt + V` 分支传播，避免同一事件重复处理。
3. 读取系统剪贴板的结构化内容，不通过字符串猜测图片。
4. 优先判断是否存在可解码的图片 MIME 数据。
5. 如果存在图片，调用现有图片粘贴/上传/插入流程，等待流程完成后再恢复输入状态。
6. 如果没有图片但存在纯文本或可转换为纯文本的富文本，按原始文字内容写入当前 CLI 输入通道；不得模拟键盘逐字发送，也不得把换行错误转换为提交。
7. 如果剪贴板没有支持的图片或文字内容，撤销本次拦截，保留系统/CLI 的原生行为。
8. 整个流程必须只执行一次；图片处理失败不得继续追加一份文字或再次触发 Agent 快捷键。

### 4.2 macOS `Control + V`

只作为图片入口处理：捕获后读取剪贴板，存在可解码图片时走图片流程；没有图片时不把普通文字误当成图片，也不抢占 `Command + V` 的文字入口。无图片时应回退原生行为或显示明确的不可用结果，具体行为以现有图片粘贴契约为准。

### 4.3 macOS `Command + V`

只作为文字入口处理：读取纯文本/富文本并写入当前 CLI 输入。若剪贴板只有图片，不应把图片隐式转成文件路径或文字；应保留原生行为或由现有图片入口处理，不能造成重复插入。

### 4.4 焦点与生命周期

- 只有内嵌 CLI 当前 tab、且输入区域处于可交互状态时才拦截。
- 会话详情、搜索框、设置输入框、普通内嵌终端和其他编辑器必须继续使用浏览器/系统默认粘贴。
- CLI 正在启动、已退出、正在发送消息、被禁用或 tab 已销毁时，不应向不存在的 PTY 写入。
- 粘贴期间禁用重复触发，完成、失败或取消后都必须释放锁。
- 切换 tab、关闭 tab、重启 PTY 时清理剪贴板处理状态和事件监听器。

## 5. 技术实施阶段

### 阶段 0：现有实现盘点与行为基线

1. 搜索所有 `paste`、`clipboard`、`Alt + V`、`Ctrl + V`、`Command + V`、图片上传、PTY 写入和内嵌 CLI 组件。
2. 画出当前事件链：DOM/窗口事件、Vue 组件、Tauri command、剪贴板 API、图片处理、PTY 写入。
3. 明确内嵌终端与内嵌 CLI 的组件边界，确认不会误改终端逻辑。
4. 记录当前每个 Agent 的实际触发路径，尤其是 `agy`、`kimicode`、`opencode` 的“右键成功、键盘失败”路径。
5. 建立基线测试和手工记录：当前行为、焦点位置、是否重复粘贴、是否产生控制字符、失败后的输入状态。

完成标准：能够明确指出共享粘贴入口和所有 Agent 特判；没有在未确认入口前修改快捷键。

### 阶段 1：抽象共享粘贴输入控制器

建议新增一个仅服务于内嵌 CLI 的粘贴控制模块，职责包括：

- 判断当前是否为可拦截的内嵌 CLI 上下文。
- 将平台原始键盘事件规范化为 `image-paste` 或 `text-paste` 意图。
- 读取剪贴板并分类。
- 调用图片处理或 CLI 文字写入适配器。
- 管理处理中状态、去重和失败回退。

该模块不应接收 Agent 名称作为主要分支条件。Agent 只提供已有的“图片插入”和“向 PTY/输入缓冲写文字”能力；快捷键决策由平台和剪贴板类型决定。

建议内部数据契约：

```text
PasteIntent = Image | Text | NativeFallback
PasteContext = { surface: EmbeddedCli, agent, tabId, sessionId, focused }
ClipboardPayload = { kind: Image | Text | Empty, mime?, bytes?, text? }
```

实现要求：

1. 使用结构化 Clipboard API 判断 MIME 和内容，不用文件扩展名或字符串前缀判断图片。
2. 图片先验证 MIME、大小和可解码性，再交给现有图片流程。
3. 富文本优先取纯文本表示，去除浏览器不应写入 CLI 的 HTML 包装。
4. 文字写入使用现有 PTY 输入协议，保留换行、Unicode 和长文本，不触发自动提交。
5. 事件处理返回明确结果：`handled`、`fallback`、`failed`，便于日志和测试，但日志不得包含剪贴板正文或图片二进制。

完成标准：共享模块可在不区分 Agent 的情况下处理图片/文字；内嵌终端的粘贴行为无变化。

### 阶段 2：平台快捷键映射与旧入口收敛

1. Windows/Linux 注册 `Ctrl + V` 为统一粘贴入口。
2. macOS 注册 `Control + V` 图片入口、`Command + V` 文字入口。
3. 删除或停用 `Alt + V` 作为正式入口，避免旧 handler 与新 handler 双重触发。
4. 对组合键判断使用 `event.ctrlKey`、`event.metaKey`、`event.altKey`、`event.key` 的规范化结果，不依赖浏览器 `event.code` 在不同键盘布局下的偶然值。
5. 明确 macOS `Control + V` 和 `Command + V` 不可互相降级为同一意图。
6. 对快捷键事件调用 `preventDefault`/`stopPropagation` 的范围做最小化，只在确认是内嵌 CLI 且存在可处理内容后阻止默认行为；若设计需要先读剪贴板再决定，则使用一次性异步处理锁，避免默认行为和自定义行为同时执行。
7. 兼容窗口级监听、组件级监听和 PTY webview 事件时，确保只保留一个最终消费者。

完成标准：同一 `Ctrl/Command/Control + V` 不会产生两次写入、图片和文字不会同时执行，普通输入区域不受影响。

### 阶段 3：图片与文字通道接入

1. 复用当前图片粘贴的 MIME、临时文件、上传或 CLI 参数转换流程，不另造 Agent 专用图片协议。
2. 将文字写入统一收敛到当前内嵌 CLI 的输入通道，优先复用已经能被右键粘贴成功的底层写入能力。
3. 验证 PTY 写入的编码、换行、控制字符转义和长文本分片策略。
4. 图片处理失败时显示现有失败样式，不能静默变成文字粘贴；文字写入失败时保留剪贴板内容，允许用户重试。
5. 处理没有焦点、PTY 尚未 ready、CLI 已退出、tab 切换和会话恢复期间的竞态。

完成标准：`agy`、`kimicode`、`opencode` 不再依赖右键即可贴文字；所有 Agent 共享同一图片/文字处理结果。

### 阶段 4：自动化测试

#### 单元测试

- 平台修饰键归一化：Windows/Linux `Ctrl + V`、macOS `Control + V`、macOS `Command + V`。
- `Alt + V` 不再产生正式粘贴意图。
- 剪贴板分类：PNG/JPEG/WebP 图片、纯文本、HTML+纯文本、空剪贴板、未知 MIME。
- 图片优先级：同时含图片和文字时，Windows/Linux `Ctrl + V` 走图片。
- macOS 意图隔离：`Control + V` 不贴文字，`Command + V` 不把图片转文字。
- 文字保留换行、Unicode、长文本和空字符串边界。
- 事件去重、异步失败释放锁、tab 销毁后不再写 PTY。
- 非内嵌 CLI surface 不拦截。

#### 组件/集成测试

- 伪造内嵌 CLI 输入上下文，验证事件捕获到图片处理或 PTY 写入的完整链路。
- 验证右键粘贴通道和统一快捷键最终写入相同输入缓冲。
- 验证图片处理完成后输入框状态、发送按钮和附件展示不重复。
- 验证 CLI 未启动、退出和重启期间的回退行为。
- 验证切换 Agent、切换 tab、切换会话后不会把粘贴内容写入旧 tab。

#### 平台人工矩阵

每个平台至少验证以下内容：

| 平台 | Agent | 图片 | 文字 | 额外验证 |
| --- | --- | --- | --- | --- |
| Windows | codex、claude code、agy、pi、kimicode、opencode、grokbuild | `Ctrl + V` | `Ctrl + V` | `Alt + V` 不触发；右键仍可用 |
| Linux | codex、claude code、agy、pi、kimicode、opencode、grokbuild | `Ctrl + V` | `Ctrl + V` | 与 Windows 分开记录，不假设完全相同 |
| macOS | codex、claude code、agy、pi、kimicode、opencode、grokbuild | `Control + V` | `Command + V` | 反向组合键不误触发 |

每个 Agent 至少使用一份纯文本、一份多行文本、一张 PNG、一张 JPEG，并记录“剪贴板同时含图片和文字”的结果。

#### 原生终端对照

在同一设备、同一剪贴板内容下，分别验证原生终端和内嵌 CLI：

- 原生终端可用的文字粘贴快捷键保持可用。
- Viewer 修复不能改变原生终端行为。
- 内嵌 CLI 右键粘贴和统一快捷键写入结果一致。

### 阶段 5：回归、可观测性与文档

1. 运行 Rust、前端类型检查、单元测试、组件测试和构建。
2. 增加开发环境下的非敏感诊断：平台、surface、快捷键意图、剪贴板 kind、处理结果；不得记录文字正文、文件路径中的敏感部分或图片内容。
3. 遇到剪贴板权限、读取失败或不支持 MIME 时，提供明确错误信息，并保留重试路径。
4. 更新本表“当前快捷键”或新增“已实现行为”时，不能覆盖历史实测数据；计划文档记录目标与验收结果。
5. 在 release checklist 中加入 Windows、Linux、macOS 三个平台的内嵌 CLI 粘贴测试。

## 6. 风险与处理

| 风险 | 表现 | 处理 |
| --- | --- | --- |
| 事件双重监听 | 一次粘贴出现两条文字或两个附件 | 统一消费者、事件去重、异步锁和集成测试 |
| 误拦截普通输入 | 设置/搜索框无法粘贴 | 以 surface 和焦点为前置条件，只在内嵌 CLI 拦截 |
| 图片与文字同时存在 | 同时上传图片并插入文字 | Windows/Linux 明确图片优先；macOS 按组合键区分 |
| CLI 输入通道差异 | 某 Agent 仍不能收到文字 | 使用已验证的右键底层写入路径，Agent 仅作为适配器，不恢复独立快捷键 |
| PTY 未就绪 | 粘贴内容丢失 | ready 状态检查、失败提示、保留剪贴板供重试 |
| 平台键盘布局差异 | 某些键盘上 `key/code` 判断失败 | 使用修饰键加逻辑键规范化，并在真实平台测试 |
| 原有图片流程回归 | 图片附件重复或不显示 | 复用现有流程并增加图片 fixture 回归 |
| 剪贴板隐私 | 日志泄漏用户文字/图片 | 只记录 kind 和结果，不记录正文/二进制 |
| Linux/Windows 差异 | 合并平台导致问题漏测 | 测试矩阵分开执行，代码尽量共享 |

## 7. 验收标准

功能验收必须同时满足：

1. Windows 和 Linux 的 7 个 Agent 均可使用 `Ctrl + V` 贴文字和贴图。
2. macOS 的 7 个 Agent 均可使用 `Control + V` 贴图、`Command + V` 贴文字。
3. `Alt + V` 不再作为正式快捷键，不产生重复或错误输入。
4. `agy`、`kimicode`、`opencode` 不再需要右键才能贴文字。
5. 图片优先级、纯文本、富文本、多行文本、Unicode 和空剪贴板行为符合设计。
6. 普通输入框、内嵌终端、会话详情和设置页面的原生粘贴行为不回归。
7. CLI 未启动、已退出、切换 tab、发送中和剪贴板读取失败时，不丢失状态、不向错误会话写入。
8. 自动化测试、类型检查和构建通过；三平台人工矩阵有记录。

## 8. 建议实施顺序

先完成阶段 0，确认当前共享入口和右键成功所对应的底层写入路径；再按阶段 1–3 实现统一控制器和平台映射。阶段 4 的自动化测试应与实现同步补齐，最后用阶段 5 的三平台矩阵验收。

不建议先按 Agent 逐个修 `Alt + V` 或 `Ctrl + V`，因为表格已证明同一个 Agent 的图片和文字路径也不一致，这样会继续保留重复监听和平台差异。

## 9. 当前实现进度

已完成第一阶段和跨平台统一入口的基础实现：

- 新增共享 `embeddedCliPaste` 控制器，按平台修饰键返回 `image`、`text` 或 `unified` 意图。
- Windows/Linux 内嵌 session CLI 的 `Ctrl+V`、macOS 的 `Control+V` / `Command+V` 统一走共享控制器；不再按 Agent 名称分支。
- 图片读取使用结构化 `navigator.clipboard.read()`，继续复用 `save_clipboard_image` 临时文件流程。
- shell tab 未接入该拦截器，保留原有粘贴行为。
- 增加平台快捷键、图片优先、文本回退和空剪贴板单元测试。

待完成：剪贴板权限失败提示、跨平台人工矩阵，以及切换 tab/PTY 生命周期的集成测试。异步读取前无法判断剪贴板内容，因此空剪贴板时快捷键仍可能被消费，这是浏览器/WebView 事件模型的已知限制。
