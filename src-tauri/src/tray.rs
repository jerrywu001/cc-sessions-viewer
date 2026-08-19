//! macOS 菜单栏托盘：用 objc2 创建 NSStatusItem + NSMenu。
//! 统计条目用 NSMenuItem.view 嵌入自定义 NSView（参考 CodexBar）。
//! 点击 agent 标题 → 打开主窗口并切换到对应 agent。
#![cfg(target_os = "macos")]
#![allow(clippy::missing_safety_doc)]

use std::sync::Mutex;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Sel};
use objc2::{msg_send, sel};
use objc2_app_kit::{NSFont, NSMenu, NSMenuItem};
use objc2_foundation::{ns_string, NSString};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::types::TrayStats;

static STATUS_PTR: Mutex<usize> = Mutex::new(0);
static MENU_PTR: Mutex<usize> = Mutex::new(0);
static HANDLER_PTR: Mutex<usize> = Mutex::new(0);
/// 菜单里 stats 区域的 item 数量（不含 action items），用于原地替换。
static STATS_ITEM_COUNT: Mutex<usize> = Mutex::new(0);
/// Refresh 按钮的指针，用于更新文字。
static REFRESH_BTN_PTR: Mutex<usize> = Mutex::new(0);

type ActionCallback = Box<dyn Fn(&str) + Send + Sync>;
static ACTION_CB: Mutex<Option<ActionCallback>> = Mutex::new(None);

/// ObjC 方法：菜单项点击时调用，读取 representedObject 里的 action ID 执行回调。
unsafe extern "C" fn handle_menu_action(_this: *mut AnyObject, _sel: Sel, sender: *mut AnyObject) {
    if sender.is_null() {
        return;
    }
    let rep: *const AnyObject = msg_send![sender, representedObject];
    if rep.is_null() {
        return;
    }
    let ns_str: &NSString = &*(rep as *const NSString);
    let action_id = ns_str.to_string();
    if let Some(cb) = ACTION_CB.lock().unwrap().as_ref() {
        cb(&action_id);
    }
}

/// ObjC 方法：Refresh 按钮点击（view-based item，不关闭菜单）。
unsafe extern "C" fn handle_refresh(_this: *mut AnyObject, _sel: Sel, _sender: *mut AnyObject) {
    if let Some(cb) = ACTION_CB.lock().unwrap().as_ref() {
        cb("tray-refresh");
    }
}

/// NSMenuDelegate: 菜单打开时自动触发一次刷新。
unsafe extern "C" fn menu_will_open(_this: *mut AnyObject, _sel: Sel, _menu: *mut AnyObject) {
    if let Some(cb) = ACTION_CB.lock().unwrap().as_ref() {
        cb("tray-refresh");
    }
}

/// 带 hover 高亮的菜单项容器 view（模拟原生 NSMenuItem 高亮效果）。
fn ensure_highlight_view_class() -> &'static AnyClass {
    static ONCE: std::sync::Once = std::sync::Once::new();
    static mut CLS: Option<&'static AnyClass> = None;
    ONCE.call_once(|| {
        let superclass = AnyClass::get(c"NSView").unwrap();
        let mut builder = ClassBuilder::new(c"CCHighlightView", superclass).unwrap();
        unsafe {
            builder.add_method(
                sel!(mouseEntered:),
                mouse_entered as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            builder.add_method(
                sel!(mouseExited:),
                mouse_exited as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            builder.add_method(
                sel!(mouseUp:),
                mouse_up_highlight as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }
        unsafe { CLS = Some(builder.register()) };
    });
    unsafe { CLS.unwrap() }
}

unsafe extern "C" fn mouse_entered(this: *mut AnyObject, _sel: Sel, _event: *mut AnyObject) {
    // 添加一个圆角高亮子 layer（带左右 margin，匹配原生菜单项高亮样式）
    remove_highlight_layer(this);
    let layer: *mut AnyObject = msg_send![this, layer];
    if layer.is_null() {
        return;
    }
    let bounds: objc2_foundation::NSRect = msg_send![this, bounds];
    let margin = 5.0_f64;
    let hl: Retained<AnyObject> = msg_send![objc2::class!(CALayer), layer];
    let frame = objc2_foundation::NSRect {
        origin: objc2_foundation::NSPoint { x: margin, y: 1.0 },
        size: objc2_foundation::NSSize {
            width: bounds.size.width - margin * 2.0,
            height: bounds.size.height - 2.0,
        },
    };
    let _: () = msg_send![&hl, setFrame: frame];
    let _: () = msg_send![&hl, setCornerRadius: 6.0_f64];
    let base_color: Retained<AnyObject> =
        msg_send![objc2::class!(NSColor), selectedContentBackgroundColor];
    let color: Retained<AnyObject> = msg_send![
        &base_color, colorWithAlphaComponent: 0.6_f64
    ];
    let cg: *const AnyObject = msg_send![&color, CGColor];
    let _: () = msg_send![&hl, setBackgroundColor: cg];
    let _: () = msg_send![&hl, setName: &*NSString::from_str("_hl")];
    let _: () = msg_send![layer, insertSublayer: &*hl, atIndex: 0_u32];
}

unsafe extern "C" fn mouse_exited(this: *mut AnyObject, _sel: Sel, _event: *mut AnyObject) {
    remove_highlight_layer(this);
}

unsafe fn remove_highlight_layer(view: *mut AnyObject) {
    let layer: *mut AnyObject = msg_send![view, layer];
    if layer.is_null() {
        return;
    }
    let sublayers: *mut AnyObject = msg_send![layer, sublayers];
    if sublayers.is_null() {
        return;
    }
    let count: usize = msg_send![sublayers, count];
    let target_name = NSString::from_str("_hl");
    for i in (0..count).rev() {
        let sub: *mut AnyObject = msg_send![sublayers, objectAtIndex: i];
        let name: *mut AnyObject = msg_send![sub, name];
        if !name.is_null() {
            let eq: bool = msg_send![name, isEqualToString: &*target_name];
            if eq {
                let _: () = msg_send![sub, removeFromSuperlayer];
            }
        }
    }
}

unsafe extern "C" fn mouse_up_highlight(this: *mut AnyObject, _sel: Sel, _event: *mut AnyObject) {
    // 找到子视图里的 NSButton 并触发 performClick
    let subviews: *mut AnyObject = msg_send![this, subviews];
    let count: usize = msg_send![subviews, count];
    for i in 0..count {
        let sub: *mut AnyObject = msg_send![subviews, objectAtIndex: i];
        let is_btn: bool = msg_send![sub, isKindOfClass: objc2::class!(NSButton)];
        if is_btn {
            let _: () = msg_send![sub, performClick: std::ptr::null::<AnyObject>()];
            break;
        }
    }
}

fn ensure_handler_class() -> &'static AnyClass {
    static ONCE: std::sync::Once = std::sync::Once::new();
    static mut CLS: Option<&'static AnyClass> = None;
    ONCE.call_once(|| {
        let superclass = AnyClass::get(c"NSObject").unwrap();
        let mut builder = ClassBuilder::new(c"CCMenuHandler", superclass).unwrap();
        unsafe {
            builder.add_method(
                sel!(handleMenuAction:),
                handle_menu_action as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            builder.add_method(
                sel!(handleRefresh:),
                handle_refresh as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
            builder.add_method(
                sel!(menuWillOpen:),
                menu_will_open as unsafe extern "C" fn(*mut AnyObject, Sel, *mut AnyObject),
            );
        }
        unsafe { CLS = Some(builder.register()) };
    });
    unsafe { CLS.unwrap() }
}

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let mtm = objc2::MainThreadMarker::new().expect("must run on main thread");

    // 注册回调：菜单项点击 → 打开主窗口 + 发 Tauri 事件
    let app_for_cb = app.clone();
    *ACTION_CB.lock().unwrap() = Some(Box::new(move |action_id: &str| {
        if action_id == "tray-refresh" {
            // 保留现有数据，Refresh 切成转圈 → 后台重算 → 更新数据 + 恢复按钮
            set_refresh_loading(true);
            let app3 = app_for_cb.clone();
            std::thread::spawn(move || {
                let stats = crate::stats::tray::quick_stats().ok();
                let _ = app3.run_on_main_thread(move || {
                    if let Some(mtm) = objc2::MainThreadMarker::new() {
                        update_stats_in_place(mtm, stats.as_ref());
                        set_refresh_loading(false);
                    }
                });
            });
            return;
        }
        // 其余操作：显示主窗口 + 发事件
        if let Some(win) = app_for_cb.get_webview_window("main") {
            let _ = win.show();
            let _ = win.set_focus();
        }
        let _ = app_for_cb.emit("menu://action", serde_json::json!({ "id": action_id }));
    }));

    // 创建 ObjC handler 实例
    let cls = ensure_handler_class();
    let handler: Retained<AnyObject> = unsafe { msg_send![cls, new] };
    *HANDLER_PTR.lock().unwrap() = Retained::into_raw(handler) as usize;

    unsafe {
        // NSStatusItem
        let bar: Retained<AnyObject> = msg_send![objc2::class!(NSStatusBar), systemStatusBar];
        let item: Retained<AnyObject> = msg_send![&bar, statusItemWithLength: -1.0_f64];

        // Icon
        let icon_data = include_bytes!("../icons/tray-template.png");
        let ns_data: Retained<AnyObject> = msg_send![
            objc2::class!(NSData),
            dataWithBytes: icon_data.as_ptr(),
            length: icon_data.len()
        ];
        let ns_img: Retained<AnyObject> = msg_send![
            msg_send![objc2::class!(NSImage), alloc],
            initWithData: &*ns_data
        ];
        let _: () = msg_send![&ns_img, setTemplate: true];
        let sz = objc2_foundation::NSSize {
            width: 18.0,
            height: 18.0,
        };
        let _: () = msg_send![&ns_img, setSize: sz];
        let btn: *mut AnyObject = msg_send![&item, button];
        if !btn.is_null() {
            let _: () = msg_send![btn, setImage: &*ns_img];
        }

        let menu = build_menu(mtm, None);
        // 设置 delegate，menuWillOpen: 打开时自动刷新
        let handler_ptr = *HANDLER_PTR.lock().unwrap();
        if handler_ptr != 0 {
            let _: () = msg_send![&menu, setDelegate: handler_ptr as *mut AnyObject];
        }
        let _: () = msg_send![&item, setMenu: &*menu];

        *STATUS_PTR.lock().unwrap() = Retained::into_raw(item) as usize;
        *MENU_PTR.lock().unwrap() = Retained::into_raw(menu) as usize;
    }

    // 后台刷新
    let app2 = app.clone();
    std::thread::spawn(move || loop {
        let stats = crate::stats::tray::quick_stats().ok();
        let _ = app2.run_on_main_thread({
            let stats = stats.clone();
            move || {
                if let Some(mtm) = objc2::MainThreadMarker::new() {
                    update_stats_in_place(mtm, stats.as_ref());
                }
            }
        });
        std::thread::sleep(std::time::Duration::from_secs(300));
    });

    Ok(())
}

/// 原地替换菜单里的 stats 区域，不关闭菜单。NSMenu 会实时刷新。
fn update_stats_in_place(mtm: objc2::MainThreadMarker, stats: Option<&TrayStats>) {
    let menu_ptr = *MENU_PTR.lock().unwrap();
    if menu_ptr == 0 {
        return;
    }
    let menu: &NSMenu = unsafe { &*(menu_ptr as *const NSMenu) };

    // 移除旧的 stats items（从头部开始，数量记在 STATS_ITEM_COUNT 里）
    let old_count = {
        let mut c = STATS_ITEM_COUNT.lock().unwrap();
        let v = *c;
        *c = 0;
        v
    };
    for _ in 0..old_count {
        unsafe {
            let first: *mut AnyObject = msg_send![menu, itemAtIndex: 0_isize];
            let _: () = msg_send![menu, removeItemAtIndex: 0_isize];
            let _ = first; // suppress warning
        }
    }

    // 插入新的 stats items 到头部
    let mut insert_idx: isize = 0;
    let mut new_count: usize = 0;

    if let Some(stats) = stats {
        for agent in &stats.agents {
            insert_menu_item(menu, mtm, make_agent_card(mtm, agent), insert_idx, false);
            insert_idx += 1;
            new_count += 1;
        }
    }

    *STATS_ITEM_COUNT.lock().unwrap() = new_count;
}

fn insert_menu_item(
    menu: &NSMenu,
    mtm: objc2::MainThreadMarker,
    view: Retained<AnyObject>,
    index: isize,
    _enabled: bool,
) {
    let item = NSMenuItem::new(mtm);
    unsafe {
        let _: () = msg_send![&item, setView: &*view];
    }
    item.setEnabled(false);
    unsafe {
        let _: () = msg_send![menu, insertItem: &*item, atIndex: index];
    }
}

fn build_menu(mtm: objc2::MainThreadMarker, stats: Option<&TrayStats>) -> Retained<NSMenu> {
    let menu = NSMenu::new(mtm);
    let mut stats_count: usize = 0;

    if let Some(stats) = stats {
        for agent in &stats.agents {
            add_view_item(&menu, mtm, make_agent_card(mtm, agent));
            stats_count += 1;
        }
    }

    *STATS_ITEM_COUNT.lock().unwrap() = stats_count;

    // Refresh — view-based button（点击不关闭菜单）
    add_refresh_button(&menu, mtm);
    // Statistics / Settings — 可点击（会关闭菜单并打开主窗口）
    add_clickable_item(&menu, mtm, "Statistics", "open-stats");
    add_clickable_item(&menu, mtm, "Settings…", "open-settings");

    menu.addItem(&NSMenuItem::separatorItem(mtm));

    let quit = NSMenuItem::new(mtm);
    quit.setTitle(ns_string!("Quit"));
    quit.setKeyEquivalent(ns_string!("q"));
    unsafe { quit.setAction(Some(sel!(terminate:))) };
    menu.addItem(&quit);

    menu
}

fn add_refresh_button(menu: &NSMenu, mtm: objc2::MainThreadMarker) {
    unsafe {
        let btn: Retained<AnyObject> = msg_send![objc2::class!(NSButton), new];
        let font = NSFont::systemFontOfSize(13.0);

        // 用 NSAttributedString 设置白色文字
        let title_str = NSString::from_str("↻  Refresh");
        let color: Retained<AnyObject> = msg_send![objc2::class!(NSColor), labelColor];
        let keys: [*const AnyObject; 2] = [
            NSString::from_str("NSFont").as_ref() as *const _,
            NSString::from_str("NSColor").as_ref() as *const _,
        ];
        let vals: [*const AnyObject; 2] = [
            &*font as *const NSFont as *const AnyObject,
            &*color as *const AnyObject,
        ];
        let dict: Retained<AnyObject> = msg_send![
            objc2::class!(NSDictionary),
            dictionaryWithObjects: vals.as_ptr(),
            forKeys: keys.as_ptr(),
            count: 2_usize
        ];
        let attr_title: Retained<AnyObject> = msg_send![
            msg_send![objc2::class!(NSAttributedString), alloc],
            initWithString: &*title_str,
            attributes: &*dict
        ];
        let _: () = msg_send![&btn, setAttributedTitle: &*attr_title];
        let _: () = msg_send![&btn, setBordered: false];
        let _: () = msg_send![&btn, setAlignment: 0_isize];

        // 保存按钮指针用于后续更新文字
        *REFRESH_BTN_PTR.lock().unwrap() = &*btn as *const AnyObject as usize;

        // target/action
        let handler_ptr = *HANDLER_PTR.lock().unwrap();
        if handler_ptr != 0 {
            let _: () = msg_send![&btn, setTarget: handler_ptr as *mut AnyObject];
            let _: () = msg_send![&btn, setAction: sel!(handleRefresh:)];
        }

        // 用带 hover 高亮的容器 view
        let cls = ensure_highlight_view_class();
        let container: Retained<AnyObject> = msg_send![cls, new];
        let _: () = msg_send![&container, setFrameSize: objc2_foundation::NSSize { width: 380.0, height: 28.0 }];
        let _: () = msg_send![&container, setWantsLayer: true];

        // 添加 tracking area 让 NSMenu 知道要跟踪鼠标（触发 highlight + drawRect）
        let tracking: Retained<AnyObject> = msg_send![
            msg_send![objc2::class!(NSTrackingArea), alloc],
            initWithRect: objc2_foundation::NSRect {
                origin: objc2_foundation::NSPoint { x: 0.0, y: 0.0 },
                size: objc2_foundation::NSSize { width: 380.0, height: 28.0 },
            },
            options: 0x01 | 0x02 | 0x20_isize, // mouseEnteredAndExited | mouseMoved | activeAlways
            owner: &*container,
            userInfo: std::ptr::null::<AnyObject>()
        ];
        let _: () = msg_send![&container, addTrackingArea: &*tracking];
        let frame = objc2_foundation::NSRect {
            origin: objc2_foundation::NSPoint { x: 14.0, y: 2.0 },
            size: objc2_foundation::NSSize {
                width: 292.0,
                height: 24.0,
            },
        };
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&container, addSubview: &*btn];

        let item = NSMenuItem::new(mtm);
        let _: () = msg_send![&item, setView: &*container];
        item.setEnabled(true);
        *REFRESH_ITEM_PTR.lock().unwrap() = &*item as *const NSMenuItem as usize;
        menu.addItem(&item);
    }
}

/// Refresh 按钮所在 NSMenuItem 的指针（用于替换整个 view）。
static REFRESH_ITEM_PTR: Mutex<usize> = Mutex::new(0);

fn set_refresh_loading(loading: bool) {
    let item_ptr = *REFRESH_ITEM_PTR.lock().unwrap();
    if item_ptr == 0 {
        return;
    }
    let Some(mtm) = objc2::MainThreadMarker::new() else {
        return;
    };
    unsafe {
        let item = item_ptr as *mut AnyObject;
        if loading {
            // 替换为 spinner + "Computing..." 的 view
            let container: Retained<AnyObject> = msg_send![objc2::class!(NSView), new];
            let _: () = msg_send![&container, setFrameSize: objc2_foundation::NSSize { width: 380.0, height: 28.0 }];

            // NSProgressIndicator (spinning)
            let spinner: Retained<AnyObject> = msg_send![objc2::class!(NSProgressIndicator), new];
            let spinner_frame = objc2_foundation::NSRect {
                origin: objc2_foundation::NSPoint { x: 18.0, y: 6.0 },
                size: objc2_foundation::NSSize {
                    width: 16.0,
                    height: 16.0,
                },
            };
            let _: () = msg_send![&spinner, setFrame: spinner_frame];
            let _: () = msg_send![&spinner, setStyle: 1_isize]; // NSProgressIndicatorStyleSpinning
            let _: () = msg_send![&spinner, setControlSize: 1_isize]; // NSControlSizeSmall
            let _: () = msg_send![&spinner, setIndeterminate: true];
            let _: () = msg_send![&spinner, setDisplayedWhenStopped: false];
            let _: () = msg_send![&spinner, startAnimation: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![&container, addSubview: &*spinner];

            // Label
            let tf = make_tf(mtm, "Refreshing stats…", 13.0, false, false);
            let tf_frame = objc2_foundation::NSRect {
                origin: objc2_foundation::NSPoint { x: 40.0, y: 4.0 },
                size: objc2_foundation::NSSize {
                    width: 200.0,
                    height: 20.0,
                },
            };
            let _: () = msg_send![&tf, setFrame: tf_frame];
            let _: () = msg_send![&container, addSubview: &*tf];

            let _: () = msg_send![item, setView: &*container];
        } else {
            // 恢复为正常的 Refresh 按钮
            let refresh_view = make_refresh_view(mtm);
            let _: () = msg_send![item, setView: &*refresh_view];
        }
    }
}

fn make_refresh_view(_mtm: objc2::MainThreadMarker) -> Retained<AnyObject> {
    unsafe {
        let btn: Retained<AnyObject> = msg_send![objc2::class!(NSButton), new];
        let font = NSFont::systemFontOfSize(13.0);
        let title_str = NSString::from_str("↻  Refresh");
        let color: Retained<AnyObject> = msg_send![objc2::class!(NSColor), labelColor];
        let keys: [*const AnyObject; 2] = [
            NSString::from_str("NSFont").as_ref() as *const _,
            NSString::from_str("NSColor").as_ref() as *const _,
        ];
        let vals: [*const AnyObject; 2] = [
            &*font as *const NSFont as *const AnyObject,
            &*color as *const AnyObject,
        ];
        let dict: Retained<AnyObject> = msg_send![
            objc2::class!(NSDictionary),
            dictionaryWithObjects: vals.as_ptr(),
            forKeys: keys.as_ptr(),
            count: 2_usize
        ];
        let attr_title: Retained<AnyObject> = msg_send![
            msg_send![objc2::class!(NSAttributedString), alloc],
            initWithString: &*title_str,
            attributes: &*dict
        ];
        let _: () = msg_send![&btn, setAttributedTitle: &*attr_title];
        let _: () = msg_send![&btn, setBordered: false];
        let _: () = msg_send![&btn, setAlignment: 0_isize];

        *REFRESH_BTN_PTR.lock().unwrap() = &*btn as *const AnyObject as usize;

        let handler_ptr = *HANDLER_PTR.lock().unwrap();
        if handler_ptr != 0 {
            let _: () = msg_send![&btn, setTarget: handler_ptr as *mut AnyObject];
            let _: () = msg_send![&btn, setAction: sel!(handleRefresh:)];
        }

        let cls = ensure_highlight_view_class();
        let container: Retained<AnyObject> = msg_send![cls, new];
        let _: () = msg_send![&container, setFrameSize: objc2_foundation::NSSize { width: 380.0, height: 28.0 }];
        let _: () = msg_send![&container, setWantsLayer: true];

        let tracking: Retained<AnyObject> = msg_send![
            msg_send![objc2::class!(NSTrackingArea), alloc],
            initWithRect: objc2_foundation::NSRect {
                origin: objc2_foundation::NSPoint { x: 0.0, y: 0.0 },
                size: objc2_foundation::NSSize { width: 380.0, height: 28.0 },
            },
            options: 0x01 | 0x02 | 0x20_isize,
            owner: &*container,
            userInfo: std::ptr::null::<AnyObject>()
        ];
        let _: () = msg_send![&container, addTrackingArea: &*tracking];

        let frame = objc2_foundation::NSRect {
            origin: objc2_foundation::NSPoint { x: 14.0, y: 2.0 },
            size: objc2_foundation::NSSize {
                width: 292.0,
                height: 24.0,
            },
        };
        let _: () = msg_send![&btn, setFrame: frame];
        let _: () = msg_send![&container, addSubview: &*btn];

        container
    }
}

fn add_clickable_item(menu: &NSMenu, mtm: objc2::MainThreadMarker, title: &str, action_id: &str) {
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(title));
    item.setEnabled(true);
    unsafe {
        item.setAction(Some(sel!(handleMenuAction:)));
        let handler_ptr = *HANDLER_PTR.lock().unwrap();
        if handler_ptr != 0 {
            let handler = handler_ptr as *mut AnyObject;
            let _: () = msg_send![&item, setTarget: handler];
        }
        let rep = NSString::from_str(action_id);
        let _: () = msg_send![&item, setRepresentedObject: &*rep];
    }
    menu.addItem(&item);
}

fn add_view_item(menu: &NSMenu, mtm: objc2::MainThreadMarker, view: Retained<AnyObject>) {
    let item = NSMenuItem::new(mtm);
    unsafe {
        let _: () = msg_send![&item, setView: &*view];
    }
    item.setEnabled(false);
    menu.addItem(&item);
}

// ── View constructors ──

fn brand_color(agent: &str) -> (f64, f64, f64) {
    match agent {
        "claude" => (204.0 / 255.0, 124.0 / 255.0, 94.0 / 255.0),
        "codex" => (73.0 / 255.0, 163.0 / 255.0, 176.0 / 255.0),
        "grok" => (92.0 / 255.0, 92.0 / 255.0, 99.0 / 255.0),
        "agy" => (139.0 / 255.0, 92.0 / 255.0, 246.0 / 255.0),
        "opencode" => (113.0 / 255.0, 113.0 / 255.0, 122.0 / 255.0),
        _ => (0.5, 0.5, 0.5),
    }
}

fn make_agent_card(
    mtm: objc2::MainThreadMarker,
    a: &crate::types::TrayAgentSummary,
) -> Retained<AnyObject> {
    let name = match a.agent.as_str() {
        "claude" => "Claude Code",
        "codex" => "Codex CLI",
        "grok" => "Grok Build",
        "agy" => "Antigravity CLI",
        "opencode" => "opencode",
        _ => &a.agent,
    };
    let (br, bg, bb) = brand_color(&a.agent);
    let card_w: f64 = 380.0;
    let card_h: f64 = 120.0;
    let margin: f64 = 10.0;
    let inner_w = card_w - margin * 2.0;
    let inner_h = card_h - 8.0;

    unsafe {
        let card: Retained<AnyObject> = msg_send![objc2::class!(NSView), new];
        let _: () = msg_send![&card, setFrameSize: objc2_foundation::NSSize { width: card_w, height: card_h }];

        // Background panel
        let panel: Retained<AnyObject> = msg_send![objc2::class!(NSView), new];
        let panel_rect = objc2_foundation::NSRect {
            origin: objc2_foundation::NSPoint { x: margin, y: 4.0 },
            size: objc2_foundation::NSSize {
                width: inner_w,
                height: inner_h,
            },
        };
        let _: () = msg_send![&panel, setFrame: panel_rect];
        let _: () = msg_send![&panel, setWantsLayer: true];
        let layer: Retained<AnyObject> = msg_send![&panel, layer];
        let _: () = msg_send![&layer, setCornerRadius: 10.0_f64];
        let panel_bg: Retained<AnyObject> = msg_send![
            objc2::class!(NSColor),
            colorWithRed: br, green: bg, blue: bb, alpha: 0.08_f64
        ];
        let cg: *const AnyObject = msg_send![&panel_bg, CGColor];
        let _: () = msg_send![&layer, setBackgroundColor: cg];
        let border_color: Retained<AnyObject> = msg_send![
            objc2::class!(NSColor),
            colorWithRed: br, green: bg, blue: bb, alpha: 0.20_f64
        ];
        let bcg: *const AnyObject = msg_send![&border_color, CGColor];
        let _: () = msg_send![&layer, setBorderColor: bcg];
        let _: () = msg_send![&layer, setBorderWidth: 0.5_f64];
        let _: () = msg_send![&card, addSubview: &*panel];

        // Left accent bar
        let accent: Retained<AnyObject> = msg_send![objc2::class!(NSView), new];
        let accent_rect = objc2_foundation::NSRect {
            origin: objc2_foundation::NSPoint { x: margin, y: 4.0 },
            size: objc2_foundation::NSSize {
                width: 3.5,
                height: inner_h,
            },
        };
        let _: () = msg_send![&accent, setFrame: accent_rect];
        let _: () = msg_send![&accent, setWantsLayer: true];
        let accent_layer: Retained<AnyObject> = msg_send![&accent, layer];
        let _: () = msg_send![&accent_layer, setCornerRadius: 2.0_f64];
        let accent_color: Retained<AnyObject> = msg_send![
            objc2::class!(NSColor),
            colorWithRed: br, green: bg, blue: bb, alpha: 0.85_f64
        ];
        let acg: *const AnyObject = msg_send![&accent_color, CGColor];
        let _: () = msg_send![&accent_layer, setBackgroundColor: acg];
        let _: () = msg_send![&card, addSubview: &*accent];

        // Header: brand dot + name + sessions
        let dot: Retained<AnyObject> = msg_send![objc2::class!(NSView), new];
        let dot_rect = objc2_foundation::NSRect {
            origin: objc2_foundation::NSPoint { x: 24.0, y: 94.0 },
            size: objc2_foundation::NSSize {
                width: 8.0,
                height: 8.0,
            },
        };
        let _: () = msg_send![&dot, setFrame: dot_rect];
        let _: () = msg_send![&dot, setWantsLayer: true];
        let dot_layer: Retained<AnyObject> = msg_send![&dot, layer];
        let _: () = msg_send![&dot_layer, setCornerRadius: 4.0_f64];
        let dot_color: Retained<AnyObject> = msg_send![
            objc2::class!(NSColor),
            colorWithRed: br, green: bg, blue: bb, alpha: 1.0_f64
        ];
        let dcg: *const AnyObject = msg_send![&dot_color, CGColor];
        let _: () = msg_send![&dot_layer, setBackgroundColor: dcg];
        let _: () = msg_send![&card, addSubview: &*dot];

        let primary: Retained<AnyObject> = msg_send![objc2::class!(NSColor), labelColor];
        let secondary: Retained<AnyObject> = msg_send![objc2::class!(NSColor), secondaryLabelColor];

        macro_rules! tf {
            ($text:expr, $sz:expr, $bold:expr, $color:expr, $x:expr, $y:expr, $w:expr) => {
                place_tf(
                    mtm,
                    $text,
                    $sz,
                    $bold,
                    &TfPlacement {
                        parent: &card,
                        color: $color,
                        x: $x,
                        y: $y,
                        w: $w,
                    },
                )
            };
        }

        tf!(name, 13.0, true, &primary, 37.0, 88.0, 180.0);
        let sessions_text = format!("{} sessions", a.session_count);
        tf!(&sessions_text, 11.0, false, &secondary, 250.0, 90.0, 110.0);

        // 3-column stats grid
        let col_w: f64 = 108.0;
        let col1_x: f64 = 24.0;
        let col2_x = col1_x + col_w;
        let col3_x = col2_x + col_w;

        tf!("Today", 10.0, false, &secondary, col1_x, 64.0, col_w);
        tf!(
            &fmt_cost(
                a.today_cost,
                a.today_unpriced_calls,
                a.today_estimated_calls,
            ),
            18.0,
            true,
            &primary,
            col1_x,
            42.0,
            col_w
        );
        tf!(
            &fmt_tokens(a.today_tokens),
            10.0,
            false,
            &secondary,
            col1_x,
            26.0,
            col_w
        );

        tf!("7 Days", 10.0, false, &secondary, col2_x, 64.0, col_w);
        tf!(
            &fmt_cost(a.week_cost, a.week_unpriced_calls, a.week_estimated_calls,),
            18.0,
            true,
            &primary,
            col2_x,
            42.0,
            col_w
        );
        tf!(
            &fmt_tokens(a.week_tokens),
            10.0,
            false,
            &secondary,
            col2_x,
            26.0,
            col_w
        );

        tf!("30 Days", 10.0, false, &secondary, col3_x, 64.0, col_w);
        tf!(
            &fmt_cost(
                a.month_cost,
                a.month_unpriced_calls,
                a.month_estimated_calls,
            ),
            18.0,
            true,
            &primary,
            col3_x,
            42.0,
            col_w
        );
        tf!(
            &fmt_tokens(a.month_tokens),
            10.0,
            false,
            &secondary,
            col3_x,
            26.0,
            col_w
        );

        // Bottom: total tokens
        let total_text = format!("{} total tokens", fmt_num(a.month_tokens));
        tf!(&total_text, 10.0, false, &secondary, 24.0, 8.0, 330.0);

        card
    }
}

struct TfPlacement<'a> {
    parent: &'a AnyObject,
    color: &'a AnyObject,
    x: f64,
    y: f64,
    w: f64,
}

unsafe fn place_tf(
    mtm: objc2::MainThreadMarker,
    text: &str,
    size: f64,
    bold: bool,
    p: &TfPlacement,
) {
    let tf = make_tf(mtm, text, size, bold, false);
    let _: () = msg_send![&tf, setTextColor: p.color];
    let frame = objc2_foundation::NSRect {
        origin: objc2_foundation::NSPoint { x: p.x, y: p.y },
        size: objc2_foundation::NSSize {
            width: p.w,
            height: if bold { 20.0 } else { 14.0 },
        },
    };
    let _: () = msg_send![&tf, setFrame: frame];
    let _: () = msg_send![p.parent, addSubview: &*tf];
}

fn make_tf(
    _mtm: objc2::MainThreadMarker,
    text: &str,
    size: f64,
    bold: bool,
    dimmed: bool,
) -> Retained<AnyObject> {
    unsafe {
        let ns_str = NSString::from_str(text);
        let tf: Retained<AnyObject> =
            msg_send![objc2::class!(NSTextField), labelWithString: &*ns_str];
        let font = if bold {
            NSFont::boldSystemFontOfSize(size)
        } else {
            NSFont::systemFontOfSize(size)
        };
        let _: () = msg_send![&tf, setFont: &*font];
        if dimmed {
            let color: Retained<AnyObject> = msg_send![objc2::class!(NSColor), secondaryLabelColor];
            let _: () = msg_send![&tf, setTextColor: &*color];
        }
        let _: () = msg_send![&tf, setDrawsBackground: false];
        let _: () = msg_send![&tf, setBezeled: false];
        let _: () = msg_send![&tf, setEditable: false];
        let _: () = msg_send![&tf, setSelectable: false];
        tf
    }
}

// ── Formatting ──

fn fmt_cost(v: f64, unpriced_calls: u64, estimated_calls: u64) -> String {
    if unpriced_calls > 0 {
        if v <= 0.0 {
            return "Unknown".to_string();
        }
        return format!(
            "{}{}+",
            fmt_cost(v, 0, 0),
            if estimated_calls > 0 { "~" } else { "" }
        );
    }
    let formatted = if v <= 0.0 {
        "$0".into()
    } else if v < 0.01 {
        "<$0.01".into()
    } else {
        format!("${v:.2}")
    };
    if estimated_calls > 0 {
        format!("{formatted}~")
    } else {
        formatted
    }
}

fn fmt_tokens(n: u64) -> String {
    if n == 0 {
        "0".into()
    } else if n < 1_000 {
        format!("{n}")
    } else if n < 1_000_000 {
        let s = format!("{:.1}", n as f64 / 1e3);
        format!("{}K", s.trim_end_matches(".0"))
    } else {
        let s = format!("{:.1}", n as f64 / 1e6);
        format!("{}M", s.trim_end_matches(".0"))
    }
}

fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut r = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            r.push(',');
        }
        r.push(c);
    }
    r.chars().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::fmt_cost;

    #[test]
    fn tray_cost_distinguishes_free_partial_and_unknown_prices() {
        assert_eq!(fmt_cost(0.0, 0, 0), "$0");
        assert_eq!(fmt_cost(0.0, 2, 0), "Unknown");
        assert_eq!(fmt_cost(1.25, 2, 0), "$1.25+");
        assert_eq!(fmt_cost(1.25, 0, 2), "$1.25~");
        assert_eq!(fmt_cost(1.25, 1, 2), "$1.25~+");
    }
}
