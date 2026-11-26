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

#[cfg(target_os = "windows")]
pub mod single_instance;

use config::{load_config, save_config};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, PhysicalPosition, Theme,
    WebviewWindowBuilder, WindowEvent,
};

/// 挂件是否处于固定（鼠标穿透）状态，进程内共享
struct PinState(AtomicBool);

/// 托盘「固定挂件」勾选项句柄，用于在挂件侧切换时同步勾选状态
struct PinMenu(CheckMenuItem<tauri::Wry>);
struct WidgetMenu(CheckMenuItem<tauri::Wry>);
struct TrafficMonitorMenu(CheckMenuItem<tauri::Wry>);

#[derive(Default)]
struct WidgetPosition(tauri::async_runtime::Mutex<Option<PhysicalPosition<i32>>>);

/// 挂件背景不透明度取值范围（0.00 ~ 1.0，0 为全透明）
const WIDGET_OPACITY_MIN: f64 = 0.0;
const WIDGET_OPACITY_MAX: f64 = 1.0;
const WIDGET_WIDTH_MIN: f64 = 240.0;
/// 挂件固定默认高度，不随纵向缩放改变
const WIDGET_HEIGHT_DEFAULT: f64 = 246.0;

/// 应用挂件窗口持久化的尺寸（背景透明度由前端 CSS 控制，字体保持不透明）
fn restore_widget_state(app: &AppHandle, position: Option<PhysicalPosition<i32>>) {
    let Some(w) = app.get_webview_window("widget") else {
        return;
    };
    let cfg = load_config(app);

    let w_w = cfg.widget_width.unwrap_or(240.0).clamp(WIDGET_WIDTH_MIN, 640.0);
    // 高度固定默认值，不随配置/纵向缩放改变
    let w_h = WIDGET_HEIGHT_DEFAULT;
    let _ = w.set_size(LogicalSize::new(w_w, w_h));

    // 首次定位到主显示器最右侧、垂直居中（位置不持久化，重启后回到默认位置）
    if let Some(position) = position {
        let _ = w.set_position(position);
    } else if let Some(mon) = app.primary_monitor().ok().flatten() {
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
    save_config(&app, &cfg)
}

/// 保存挂件独立主题偏好；显式选择后不再跟随主界面主题
#[tauri::command]
fn set_widget_theme(app: AppHandle, theme: String) -> Result<(), String> {
    if theme != "light" && theme != "dark" {
        return Err("不支持的挂件主题".to_string());
    }
    let mut cfg = load_config(&app);
    cfg.widget_theme = Some(theme);
    save_config(&app, &cfg)
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
    let mut cfg = load_config(&app);
    cfg.host_history.clear();
    cfg
}

#[tauri::command]
fn save_config_cmd(
    app: AppHandle,
    mut config: types::AppConfig,
    traffic_monitor_enabled: Option<bool>,
) -> Result<(), String> {
    let current = load_config(&app);
    // 挂件独立保存的偏好可能比主窗口中的配置副本更新。
    config.widget_opacity = current.widget_opacity;
    config.widget_width = current.widget_width;
    config.widget_height = current.widget_height;
    config.widget_visible = current.widget_visible;
    config.widget_adapter = current.widget_adapter;
    config.widget_theme = current.widget_theme;
    config.autostart = current.autostart;
    config.traffic_monitor_enabled = traffic_monitor_enabled.or(current.traffic_monitor_enabled);
    config.host_history = current.host_history;
    save_config(&app, &config)?;
    if let Some(enabled) = traffic_monitor_enabled {
        publish_traffic_monitor_enabled(&app, enabled);
    }
    Ok(())
}

fn publish_traffic_monitor_enabled(app: &AppHandle, enabled: bool) {
    let _ = app.state::<TrafficMonitorMenu>().0.set_checked(enabled);
    let _ = app.emit("traffic-monitor-changed", enabled);
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

async fn set_widget_visibility(app: &AppHandle, visible: Option<bool>, focus: bool) -> Result<(), String> {
    let state = app.state::<WidgetPosition>();
    let mut position = state.0.lock().await;
    let existing = app.get_webview_window("widget");
    let visible = visible.unwrap_or(existing.is_none());
    if visible && existing.is_none() {
        let config = app.config().app.windows.iter()
            .find(|config| config.label == "widget")
            .ok_or("找不到桌面挂件的窗口配置")?;
        let w = WebviewWindowBuilder::from_config(app, config)
            .and_then(|builder| builder.build())
            .map_err(|e| format!("无法创建桌面挂件：{e}"))?;
        #[cfg(target_os = "windows")]
        let _ = w.set_shadow(false);
        restore_widget_state(app, *position);
        let pinned = app.state::<PinState>().0.load(Ordering::SeqCst);
        let _ = w.set_ignore_cursor_events(pinned);
        w.show().map_err(|e| e.to_string())?;
        if focus && !pinned {
            let _ = w.set_focus();
        }
    } else if !visible {
        if let Some(w) = existing {
            *position = w.outer_position().ok().or(*position);
            w.destroy().map_err(|e| e.to_string())?;
            // 销毁由事件循环异步处理，等待标签释放后才允许创建下一窗口。
            let handle = app.clone();
            tauri::async_runtime::spawn_blocking(move || {
                for _ in 0..100 {
                    if handle.get_webview_window("widget").is_none() {
                        return Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
                Err("关闭桌面挂件超时".to_string())
            }).await.map_err(|e| e.to_string())??;
        }
    }
    let mut cfg = load_config(app);
    if cfg.widget_visible.unwrap_or(true) != visible {
        cfg.widget_visible = Some(visible);
        save_config(app, &cfg)?;
    }
    let _ = app.state::<WidgetMenu>().0.set_checked(visible);
    Ok(())
}

#[tauri::command]
async fn show_widget(app: AppHandle) -> Result<(), String> {
    set_widget_visibility(&app, Some(true), true).await
}

#[tauri::command]
async fn hide_widget(app: AppHandle) -> Result<(), String> {
    set_widget_visibility(&app, Some(false), false).await
}

#[tauri::command]
fn widget_pinned(app: AppHandle) -> bool {
    app.state::<PinState>().0.load(Ordering::SeqCst)
}

#[tauri::command]
fn set_widget_adapter(app: AppHandle, name: String) -> Result<(), String> {
    let mut cfg = load_config(&app);
    cfg.widget_adapter = Some(name.clone());
    cfg.default_adapter = Some(name.clone());
    save_config(&app, &cfg)?;
    let _ = app.emit_to("main", "widget-adapter-changed", name);
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

pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        let _ = app.emit_to("main", "main-visibility-changed", true);
    }
}

#[tauri::command]
fn restore_main_window(app: AppHandle) {
    show_main_window(&app);
}

/// 双击托盘：处于隐藏 / 最小化等收起状态时，仅恢复显示并聚焦；
/// 正常显示状态下临时前置，并把窗口复位为默认大小、回到屏幕居中
fn tray_double_click_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let visible = w.is_visible().unwrap_or(false);
        let minimized = w.is_minimized().unwrap_or(false);

        if !visible || minimized {
            show_main_window(app);
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
    app.manage(WidgetMenu(widget.clone()));
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
    let traffic_monitor = CheckMenuItem::with_id(
        app,
        "traffic-monitor",
        "流量监控",
        true,
        startup_cfg.traffic_monitor_enabled.unwrap_or(true),
        None::<&str>,
    )?;
    app.manage(TrafficMonitorMenu(traffic_monitor.clone()));
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

    let menu = Menu::with_items(
        app,
        &[&show, &widget, &autostart_item, &traffic_monitor, &pin, &sep, &quit],
    )?;

    let _tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("IPSwitcher 网卡IP快速切换工具")
        .icon(app.default_window_icon().unwrap().clone())
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "widget" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = set_widget_visibility(&app, None, true).await {
                        eprintln!("{error}");
                    }
                });
            }
            "autostart" => {
                let menu = app.state::<autostart::AutostartMenu>();
                // Windows 在发送菜单事件前已切换勾选状态。
                let result = menu.0
                    .is_checked()
                    .map_err(|error| error.to_string())
                    .and_then(|enabled| autostart::apply(app, enabled));
                if let Err(error) = result {
                    let _ = menu.0.set_checked(autostart::is_registered());
                    eprintln!("切换开机自启失败：{error}");
                }
            }
            "traffic-monitor" => {
                let mut config = load_config(app);
                let previous = config.traffic_monitor_enabled.unwrap_or(true);
                let enabled = !previous;
                config.traffic_monitor_enabled = Some(enabled);
                match save_config(app, &config) {
                    Ok(()) => publish_traffic_monitor_enabled(app, enabled),
                    Err(error) => {
                        let _ = app.state::<TrafficMonitorMenu>().0.set_checked(previous);
                        eprintln!("切换流量监控失败：{error}");
                    }
                }
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
    let args: Vec<String> = std::env::args().collect();
    let is_elevate_helper = args.iter().any(|arg| arg == "--elevate");
    let is_elevated_restart = args.iter().any(|arg| arg == "--elevated-restart");

    // 用户曾明确选择管理员重启时，下一次从普通快捷方式启动也会再请求 UAC。
    // Windows 不会跨进程保存管理员令牌，因此每次新启动仍需由用户确认 UAC。
    if !is_elevate_helper && !is_elevated_restart && !netcfg::is_admin() {
        let mut config = config::load_startup_config();
        if config.elevate_on_startup.unwrap_or(false) {
            if elevate::restart_as_admin_on_startup().is_ok() {
                std::process::exit(0);
            }
            // 用户取消 UAC 后继续以标准权限运行，避免每次启动反复弹 UAC。
            config.elevate_on_startup = Some(false);
            let _ = config::save_startup_config(&config);
        }
    }

    tauri::Builder::default()
        .setup(|app| {
            let args: Vec<String> = std::env::args().collect();
            if args.iter().any(|a| a == "--elevate") {
                elevate::process_pending_action(&app.handle());
                std::process::exit(0);
            }

            if let Err(error) = config::migrate_host_history(app.handle()) {
                eprintln!("扫描历史迁移失败：{error}");
            }
            setup_tray(app)?;
            app.manage(netcfg::AdapterSnapshots::default());
            app.manage(netcfg::TrafficCache::default());
            app.manage(PinState(AtomicBool::new(false)));
            app.manage(WidgetPosition::default());
            app.manage(update::DownloadCancel(AtomicBool::new(false)));

            #[cfg(target_os = "windows")]
            single_instance::start_activation_listener(app.handle().clone());

            // 启动时按配置恢复挂件可见性
            let mut startup_cfg = load_config(&app.handle());
            if args.iter().any(|arg| arg == "--elevated-restart")
                && netcfg::is_admin()
            {
                startup_cfg.elevate_on_startup = Some(true);
                startup_cfg.admin_prompt_dismissed = Some(true);
                let _ = save_config(&app.handle(), &startup_cfg);
            }
            if startup_cfg.widget_visible.unwrap_or(true) {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = set_widget_visibility(&handle, Some(true), false).await {
                        let _ = handle.state::<WidgetMenu>().0.set_checked(false);
                        eprintln!("{error}");
                    }
                });
            }

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
                                let _ = handle.emit_to("main", "main-visibility-changed", false);
                            }
                        }
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            netcfg::list_adapters,
            netcfg::schedule_adapter_refresh,
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
            restore_main_window,
            widget_pinned,
            set_widget_adapter,
            set_widget_opacity,
            set_widget_theme,
            save_widget_size,
            set_widget_pinned,
            autostart::set_autostart,
            autostart::get_autostart,
            shortcut::create_desktop_shortcut,
            update::check_update,
            update::download_update,
            update::cancel_download,
            update::apply_update
        ])
        .run(tauri::generate_context!())
        .expect("启动 IPSwitcher 失败");
}
