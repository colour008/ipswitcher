mod config;
mod autostart;
mod diag;
mod elevate;
mod encoding;
mod netcfg;
mod process;
mod shortcut;
mod scanner;
mod types;
mod update;

use config::{load_config, save_config};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, Theme, WindowEvent,
};

/// 挂件是否处于固定（鼠标穿透）状态，进程内共享
struct PinState(AtomicBool);

/// 托盘「固定挂件」勾选项句柄，用于在挂件侧切换时同步勾选状态
struct PinMenu(CheckMenuItem<tauri::Wry>);

/// 挂件背景不透明度取值范围（0.00 ~ 1.0，0 为全透明）
const WIDGET_OPACITY_MIN: f64 = 0.0;
const WIDGET_OPACITY_MAX: f64 = 1.0;
const WIDGET_WIDTH_MIN: f64 = 240.0;
/// 挂件固定默认高度，不随纵向缩放改变
const WIDGET_HEIGHT_DEFAULT: f64 = 246.0;

/// 应用挂件窗口持久化的尺寸（背景透明度由前端 CSS 控制，字体保持不透明）
fn restore_widget_state(app: &AppHandle) {
    let Some(w) = app.get_webview_window("widget") else {
        return;
    };
    let cfg = load_config(app);

    let w_w = cfg.widget_width.unwrap_or(240.0).clamp(WIDGET_WIDTH_MIN, 640.0);
    // 高度固定默认值，不随配置/纵向缩放改变
    let w_h = WIDGET_HEIGHT_DEFAULT;
    let _ = w.set_size(LogicalSize::new(w_w, w_h));

    // 首次定位到主显示器最右侧、垂直居中（位置不持久化，重启后回到默认位置）
    if let Some(mon) = app.primary_monitor().ok().flatten() {
        let scale = mon.scale_factor();
        let screen_w = mon.size().width as f64 / scale;
        let screen_h = mon.size().height as f64 / scale;
        let margin = 16.0;
        let x = (screen_w - w_w - margin).max(0.0);
        let y = ((screen_h - w_h) / 2.0).max(0.0);
        let _ = w.set_position(LogicalPosition::new(x, y));
    }
    // 挂件始终置于所有窗口最底层
    let _ = w.set_always_on_bottom(true);
}

/// 记录挂件背景不透明度（0.0 ~ 1.0）并持久化，仅前端卡片背景使用
#[tauri::command]
fn set_widget_opacity(app: AppHandle, opacity: f64) -> Result<(), String> {
    let opacity = opacity.clamp(WIDGET_OPACITY_MIN, WIDGET_OPACITY_MAX);
    let mut cfg = load_config(&app);
    cfg.widget_opacity = Some(opacity);
    let _ = save_config(&app, &cfg);
    Ok(())
}

/// 记录挂件窗口当前宽度到配置（高度固定默认值，拖拽缩放结束后由前端上报）
#[tauri::command]
fn save_widget_size(app: AppHandle, width: f64, _height: f64) -> Result<(), String> {
    let mut cfg = load_config(&app);
    cfg.widget_width = Some(width.clamp(WIDGET_WIDTH_MIN, 640.0));
    cfg.widget_height = Some(WIDGET_HEIGHT_DEFAULT);
    let _ = save_config(&app, &cfg);
    Ok(())
}

#[tauri::command]
fn get_config(app: AppHandle) -> types::AppConfig {
    load_config(&app)
}

#[tauri::command]
fn save_config_cmd(app: AppHandle, config: types::AppConfig) -> Result<(), String> {
    save_config(&app, &config)
}

#[tauri::command]
fn check_admin() -> Result<bool, String> {
    Ok(netcfg::is_admin())
}

/// 调试模式下由前端自绘右键菜单触发，打开开发者工具（发布构建下 DevTools 不可用）
#[cfg(debug_assertions)]
#[tauri::command]
fn open_devtools(window: tauri::WebviewWindow) {
    window.open_devtools();
}

/// 设置主窗口原生标题栏深浅主题（深色 / 浅色）
#[tauri::command]
fn set_window_theme(app: AppHandle, dark: bool) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("main") {
        let theme = if dark { Theme::Dark } else { Theme::Light };
        w.set_theme(Some(theme)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 显示挂件窗口（透明悬浮小窗）
#[tauri::command]
fn show_widget(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("widget") {
        #[cfg(target_os = "windows")]
        {
            let _ = w.set_shadow(false);
        }
        let _ = w.show();
        let _ = w.set_always_on_bottom(true);
        let _ = w.set_focus();
    }
    Ok(())
}

/// 隐藏挂件窗口
#[tauri::command]
fn hide_widget(app: AppHandle) -> Result<(), String> {
    if let Some(w) = app.get_webview_window("widget") {
        let _ = w.hide();
    }
    Ok(())
}

/// 切换挂件固定（鼠标穿透）状态：固定后位置锁定、不接收鼠标事件；再次取消后恢复拖拽缩放
#[tauri::command]
fn set_widget_pinned(app: AppHandle, pinned: bool) -> Result<(), String> {
    app.state::<PinState>().0.store(pinned, Ordering::SeqCst);
    if let Some(w) = app.get_webview_window("widget") {
        let _ = w.set_ignore_cursor_events(pinned);
    }
    if let Some(menu) = app.try_state::<PinMenu>() {
        let _ = menu.0.set_checked(pinned);
    }
    let _ = app.emit("widget-pin-changed", pinned);
    Ok(())
}

/// 主窗默认尺寸（与 tauri.conf.json 中 main 窗口一致）
const MAIN_DEFAULT_SIZE: LogicalSize<f64> = LogicalSize::new(1180.0, 830.0);

fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

/// 双击托盘：处于隐藏 / 最小化等收起状态时，仅恢复显示并聚焦；
/// 正常显示状态下临时前置，并把窗口复位为默认大小、回到屏幕居中
fn tray_double_click_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let minimized = w.is_minimized().unwrap_or(false);

        if !visible || minimized {
            let _ = w.show();
            let _ = w.unminimize();
            let _ = w.set_focus();
            return;
        }

        // 最大化状态先还原，否则尺寸重置不生效
        if w.is_maximized().unwrap_or(false) {
            let _ = w.unmaximize();
        }
        let _ = w.set_size(MAIN_DEFAULT_SIZE);
        let _ = w.center();
        let _ = w.set_focus();
    }
}

/// 挂件窗口当前是否可见
fn widget_visible(app: &AppHandle) -> bool {
    app.get_webview_window("widget")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false)
}

/// 切换挂件窗口显示 / 隐藏，并把状态写入配置持久化
fn toggle_widget(app: &AppHandle) {
    let visible = widget_visible(app);
    if visible {
        if let Some(w) = app.get_webview_window("widget") {
            let _ = w.hide();
        }
    } else {
        if let Some(w) = app.get_webview_window("widget") {
            let _ = w.show();
            let _ = w.set_focus();
        }
    }
    let mut cfg = load_config(app);
    cfg.widget_visible = Some(!visible);
    let _ = save_config(app, &cfg);
}

fn setup_tray(app: &tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let show = MenuItem::with_id(app, "show", "显示主窗口", true, None::<&str>)?;
    let startup_cfg = load_config(app.handle());
    let widget_on = startup_cfg.widget_visible.unwrap_or(true);
    let widget = CheckMenuItem::with_id(
        app,
        "widget",
        "桌面挂件",
        true,
        widget_on,
        None::<&str>,
    )?;
    let auto_on = startup_cfg.autostart.unwrap_or(true);
    let autostart_item = CheckMenuItem::with_id(
        app,
        "autostart",
        "开机自启",
        true,
        auto_on,
        None::<&str>,
    )?;
    app.manage(autostart::AutostartMenu(autostart_item.clone()));
    let pin = CheckMenuItem::with_id(
        app,
        "pin",
        "固定挂件（鼠标穿透）",
        true,
        false,
        None::<&str>,
    )?;
    app.manage(PinMenu(pin.clone()));
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(app, &[&show, &widget, &autostart_item, &pin, &sep, &quit])?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("IPSwitcher 网卡IP快速切换工具")
        .icon(app.default_window_icon().unwrap().clone())
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "widget" => {
                let now = !widget_visible(app);
                let _ = widget.set_checked(now);
                toggle_widget(app);
            }
            "autostart" => {
                let current = app
                    .state::<autostart::AutostartMenu>()
                    .0
                    .is_checked()
                    .unwrap_or(false);
                let _ = autostart::apply(app, !current);
            }
            "pin" => {
                let now = !app.state::<PinState>().0.load(Ordering::SeqCst);
                let _ = pin.set_checked(now);
                let _ = set_widget_pinned(app.clone(), now);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            let app = tray.app_handle().clone();
            match event {
                TrayIconEvent::DoubleClick { .. } => tray_double_click_main(&app),
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } => show_main_window(&app),
                _ => {}
            }
        })
        .build(app)?;

    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--elevate") {
                elevate::process_pending_action(&app.handle());
                std::process::exit(0);
            }

            setup_tray(app)?;
            app.manage(PinState(AtomicBool::new(false)));

            // 启动时按配置恢复挂件可见性
            let startup_cfg = load_config(&app.handle());
            if let Some(w) = app.get_webview_window("widget") {
                // 关闭 Windows DWM 为无边框窗口默认渲染的阴影/细边框，避免全透明时残留一圈白线
                #[cfg(target_os = "windows")]
                {
                    let _ = w.set_shadow(false);
                }
            }
            if startup_cfg.widget_visible.unwrap_or(true) {
                if let Some(w) = app.get_webview_window("widget") {
                    let _ = w.show();
                }
            }
            restore_widget_state(&app.handle());

            // 开机自启：缺省开启，按配置同步注册表登记/移除
            let auto_enabled = startup_cfg.autostart.unwrap_or(true);
            let _ = autostart::apply(&app.handle(), auto_enabled);

            if let Some(window) = app.get_webview_window("main") {
                // 启动按配置设置原生标题栏深浅（缺省深色）
                let main_dark = startup_cfg.theme.as_deref() != Some("light");
                let _ = window.set_theme(Some(if main_dark { Theme::Dark } else { Theme::Light }));
                let w = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let handle = w.app_handle();
                        let cfg = load_config(&handle);
                        // 缺省（未配置）默认最小化到托盘；仅显式选择「直接退出」才退出进程
                        match cfg.close_action.as_deref() {
                            Some("exit") => {
                                handle.exit(0);
                            }
                            _ => {
                                let _ = w.hide();
                            }
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            netcfg::list_adapters,
            netcfg::get_adapter_traffic,
            netcfg::apply_static,
            netcfg::apply_dhcp,
            netcfg::apply_dns,
            netcfg::set_adapter_enabled,
            netcfg::open_network_connections,
            netcfg::rename_adapter,
            diag::run_diagnostic,
            diag::stop_diagnostic,
            scanner::run_host_scan,
            scanner::stop_host_scan,
            elevate::elevate_action,
            elevate::restart_as_admin,
            elevate::should_prompt_admin,
            get_config,
            save_config_cmd,
            check_admin,
            #[cfg(debug_assertions)]
            open_devtools,
            set_window_theme,
            show_widget,
            hide_widget,
            set_widget_opacity,
            save_widget_size,
            set_widget_pinned,
            autostart::set_autostart,
            autostart::get_autostart,
            shortcut::create_desktop_shortcut,
            update::check_update,
            update::download_update,
            update::apply_update
        ])
        .run(tauri::generate_context!())
        .expect("启动 IPSwitcher 失败");
}
