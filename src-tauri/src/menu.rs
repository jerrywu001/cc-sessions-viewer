// 原生应用菜单 —— 主要给 macOS 顶部菜单栏用。Windows 也会在窗口里出现，
// 但更典型的 Windows 工作流是窗口内自有 UI，所以菜单是「锦上添花」级别。
//
// 设计思路：菜单不直接持有 app state，所有动作都是「emit 一个事件给前端，
// 前端调对应函数」。Theme / Language 是例外 —— 这两个会反过来让 Rust 同步
// CheckMenuItem 的勾选态，所以 build_menu 把这些 item 句柄保存到一个全局
// MenuState 里，setup_menu_bridge 注册的 listener 在收到前端 emit 的
// `menu:sync` 时按 id 翻 `set_checked`。
//
// i18n：菜单文案是构造期固定（Tauri 2 不支持改文案，整棵重建代价大），先
// 全英文。中文用户可以靠 macOS 「系统设置 → 键盘 → 输入法」一并切，多数
// 用户其实更熟英文菜单标签（File / Edit / View …）。

use std::sync::Mutex;
use tauri::menu::{
    AboutMetadataBuilder, CheckMenuItem, MenuBuilder, MenuEvent, MenuItem, PredefinedMenuItem,
    SubmenuBuilder,
};
use tauri::{AppHandle, Emitter, Listener, Manager, Runtime};

/// 菜单点击事件 → 前端的桥：菜单项 `id` 直接做 payload，前端按 id 路由到对应函数。
#[derive(Clone, serde::Serialize)]
struct MenuActionPayload {
    id: String,
}

/// 主题 / 语言子菜单的勾选态需要从前端 `menu:sync` 事件反推。
/// 这俩里的 CheckMenuItem 句柄存在这里，事件 handler 按 group + value 翻 set_checked。
struct MenuState<R: Runtime> {
    theme_items: Vec<(String, CheckMenuItem<R>)>, // (value, item)
    lang_items: Vec<(String, CheckMenuItem<R>)>,
}

/// 给主线程包一层 Mutex 存到 app state；事件 handler 直接 lock 取。
pub struct MenuStateLock<R: Runtime>(Mutex<MenuState<R>>);

/// 从前端来的菜单同步事件：theme / lang 单选 group 改值。
#[derive(Clone, serde::Deserialize)]
struct MenuSyncPayload {
    group: String, // "theme" | "lang"
    value: String,
}

/// 构造主菜单并把句柄存进 app state。run() 的 setup 钩里调一次即可。
pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    // ---------- 应用菜单（macOS 自带 + 我们追加的几项） ----------
    let pkg = app.package_info();
    let about_meta = AboutMetadataBuilder::new()
        .name(Some(pkg.name.clone()))
        .version(Some(pkg.version.to_string()))
        .copyright(Some("MIT © wuchao".to_string()))
        .build();
    let about =
        PredefinedMenuItem::about(app, Some(&format!("About {}", pkg.name)), Some(about_meta))?;
    let check_update = MenuItem::with_id(
        app,
        "check-update",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;
    let prefs = MenuItem::with_id(
        app,
        "open-settings",
        "Preferences…",
        true,
        Some("CmdOrCtrl+,"),
    )?;
    let services = PredefinedMenuItem::services(app, None)?;
    let hide = PredefinedMenuItem::hide(app, None)?;
    let hide_others = PredefinedMenuItem::hide_others(app, None)?;
    let show_all = PredefinedMenuItem::show_all(app, None)?;
    let quit = PredefinedMenuItem::quit(app, None)?;
    let app_menu = SubmenuBuilder::new(app, &pkg.name)
        .item(&about)
        .separator()
        .item(&check_update)
        .separator()
        .item(&prefs)
        .separator()
        .item(&services)
        .separator()
        .item(&hide)
        .item(&hide_others)
        .item(&show_all)
        .separator()
        .item(&quit)
        .build()?;

    // ---------- File ----------
    let new_session = MenuItem::with_id(
        app,
        "new-session",
        "New Session in Current Project",
        true,
        Some("CmdOrCtrl+N"),
    )?;
    let new_tab = MenuItem::with_id(app, "new-tab", "New Tab", true, Some("CmdOrCtrl+T"))?;
    let close_tab = MenuItem::with_id(app, "close-tab", "Close Tab", true, Some("CmdOrCtrl+W"))?;
    let rename_tab = MenuItem::with_id(app, "rename-tab", "Rename Tab", true, Some("CmdOrCtrl+R"))?;
    let add_folder =
        MenuItem::with_id(app, "add-folder", "Add Folder…", true, Some("CmdOrCtrl+O"))?;
    let export = MenuItem::with_id(
        app,
        "export-session",
        "Export Session…",
        true,
        Some("CmdOrCtrl+E"),
    )?;
    let file_menu = SubmenuBuilder::new(app, "File")
        .item(&new_session)
        .item(&new_tab)
        .item(&close_tab)
        .item(&rename_tab)
        .separator()
        .item(&add_folder)
        .separator()
        .item(&export)
        .build()?;

    // ---------- Edit ----------
    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .item(&PredefinedMenuItem::undo(app, None)?)
        .item(&PredefinedMenuItem::redo(app, None)?)
        .separator()
        .item(&PredefinedMenuItem::cut(app, None)?)
        .item(&PredefinedMenuItem::copy(app, None)?)
        .item(&PredefinedMenuItem::paste(app, None)?)
        .item(&PredefinedMenuItem::select_all(app, None)?)
        .build()?;

    // ---------- View ----------
    let toggle_sidebar = MenuItem::with_id(
        app,
        "toggle-sidebar",
        "Toggle Sidebar",
        true,
        Some("CmdOrCtrl+B"),
    )?;
    let open_stats = MenuItem::with_id(
        app,
        "open-stats",
        "Statistics",
        true,
        Some("CmdOrCtrl+Shift+S"),
    )?;
    // Theme 子菜单 —— 三选一，CheckMenuItem。默认先全不勾，setup_menu_bridge 等
    // 前端启动后会发一次 menu:sync 把当前值勾上。
    let theme_light =
        CheckMenuItem::with_id(app, "theme:light", "Light", true, false, None::<&str>)?;
    let theme_dark = CheckMenuItem::with_id(app, "theme:dark", "Dark", true, false, None::<&str>)?;
    let theme_system =
        CheckMenuItem::with_id(app, "theme:system", "System", true, false, None::<&str>)?;
    let theme_codex =
        CheckMenuItem::with_id(app, "theme:codex", "Codex", true, false, None::<&str>)?;
    let theme_dracula =
        CheckMenuItem::with_id(app, "theme:dracula", "Dracula", true, false, None::<&str>)?;
    let theme_menu = SubmenuBuilder::new(app, "Theme")
        .item(&theme_light)
        .item(&theme_dark)
        .item(&theme_system)
        .item(&theme_codex)
        .item(&theme_dracula)
        .build()?;
    // Language 子菜单 —— 四选一。
    let lang_en = CheckMenuItem::with_id(app, "lang:en", "English", true, false, None::<&str>)?;
    let lang_zh = CheckMenuItem::with_id(app, "lang:zh", "简体中文", true, false, None::<&str>)?;
    let lang_zh_tw =
        CheckMenuItem::with_id(app, "lang:zh-TW", "繁體中文", true, false, None::<&str>)?;
    let lang_ja = CheckMenuItem::with_id(app, "lang:ja", "日本語", true, false, None::<&str>)?;
    let lang_menu = SubmenuBuilder::new(app, "Language")
        .item(&lang_en)
        .item(&lang_zh)
        .item(&lang_zh_tw)
        .item(&lang_ja)
        .build()?;
    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&toggle_sidebar)
        .item(&open_stats)
        .separator()
        .item(&theme_menu)
        .item(&lang_menu)
        .build()?;

    // ---------- Find ----------
    let find_in_session = MenuItem::with_id(
        app,
        "find-in-session",
        "Find in Session…",
        true,
        Some("CmdOrCtrl+F"),
    )?;
    let find_next = MenuItem::with_id(app, "find-next", "Find Next", true, Some("CmdOrCtrl+G"))?;
    let find_prev = MenuItem::with_id(
        app,
        "find-prev",
        "Find Previous",
        true,
        Some("CmdOrCtrl+Shift+G"),
    )?;
    let find_global = MenuItem::with_id(
        app,
        "open-global-search",
        "Find in All Sessions…",
        true,
        Some("CmdOrCtrl+Shift+F"),
    )?;
    let find_menu = SubmenuBuilder::new(app, "Find")
        .item(&find_in_session)
        .item(&find_next)
        .item(&find_prev)
        .separator()
        .item(&find_global)
        .build()?;

    // ---------- Window ----------
    let trash = MenuItem::with_id(app, "open-trash", "Trash", true, Some("CmdOrCtrl+Shift+T"))?;
    let window_menu = SubmenuBuilder::new(app, "Window")
        .item(&PredefinedMenuItem::minimize(app, None)?)
        .item(&PredefinedMenuItem::maximize(app, None)?)
        .separator()
        .item(&trash)
        .item(&PredefinedMenuItem::fullscreen(app, None)?)
        .build()?;

    // ---------- Help ----------
    let help_docs = MenuItem::with_id(app, "help-docs", "Documentation", true, None::<&str>)?;
    let help_repo = MenuItem::with_id(app, "help-repo", "GitHub Repository", true, None::<&str>)?;
    let help_issue = MenuItem::with_id(app, "help-issue", "Report an Issue", true, None::<&str>)?;
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&help_docs)
        .item(&help_repo)
        .item(&help_issue)
        .build()?;

    // ---------- 装到 root 上 ----------
    let menu = MenuBuilder::new(app)
        .items(&[
            &app_menu,
            &file_menu,
            &edit_menu,
            &view_menu,
            &find_menu,
            &window_menu,
            &help_menu,
        ])
        .build()?;
    app.set_menu(menu)?;

    // ---------- 存 CheckMenuItem 句柄到 app state，供 menu:sync 用 ----------
    let state = MenuState {
        theme_items: vec![
            ("light".into(), theme_light),
            ("dark".into(), theme_dark),
            ("system".into(), theme_system),
            ("codex".into(), theme_codex),
            ("dracula".into(), theme_dracula),
        ],
        lang_items: vec![
            ("en".into(), lang_en),
            ("zh".into(), lang_zh),
            ("zh-TW".into(), lang_zh_tw),
            ("ja".into(), lang_ja),
        ],
    };
    app.manage(MenuStateLock::<R>(Mutex::new(state)));

    Ok(())
}

/// 注册菜单点击 → 前端 emit 的桥，以及前端 → CheckMenuItem 同步的桥。
/// build 之后调一次即可。
pub fn install_bridges<R: Runtime>(app: &AppHandle<R>) {
    let app_for_menu = app.clone();
    app.on_menu_event(move |app_handle, event: MenuEvent| {
        let id = event.id().as_ref().to_string();
        // 托盘菜单可能在主窗口被 close-to-tray 隐藏时点击 —— 先把窗口显示并聚焦，
        // 否则点"设置"会在看不见的窗口里打开。主菜单点击时窗口本就可见，幂等无害。
        if let Some(win) = app_handle.get_webview_window("main") {
            let _ = win.show();
            let _ = win.set_focus();
        }
        // 托盘专属项：纯粹唤回窗口，无需派发给前端。
        if id == "show-window" {
            return;
        }
        // 把 id 直接当 payload 派给前端；前端的 menu router 按 id 分发。
        let _ = app_for_menu.emit("menu://action", MenuActionPayload { id });
    });

    // 前端 → Rust：theme / lang 切了，把对应 CheckMenuItem 勾上。
    // 用 once / listen 都行；listen 持久订阅。
    let app_for_sync = app.clone();
    app.listen("menu:sync", move |event| {
        let payload: MenuSyncPayload = match serde_json::from_str(event.payload()) {
            Ok(p) => p,
            Err(_) => return,
        };
        if let Some(state) = app_for_sync.try_state::<MenuStateLock<R>>() {
            if let Ok(guard) = state.0.lock() {
                let group = match payload.group.as_str() {
                    "theme" => &guard.theme_items,
                    "lang" => &guard.lang_items,
                    _ => return,
                };
                for (v, item) in group {
                    let _ = item.set_checked(v == &payload.value);
                }
            }
        }
    });
}
