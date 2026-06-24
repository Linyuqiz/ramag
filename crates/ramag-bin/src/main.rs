//! 主入口：tracing → 装配数据层 → 注册 Tool → 启动 GPUI App → 打开主窗口

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gpui::{
    Action, App, Bounds, KeyBinding, Menu, MenuItem, Size, Subscription, TitlebarOptions,
    WindowBounds, WindowKind, WindowOptions, point, prelude::*, px, size,
};
use gpui_component::Root;
use ramag_app::{ClipboardService, ConnectionService, MongoService, RedisService, ToolRegistry};
use ramag_domain::traits::{ClipboardDriver, DocDriver, Driver, GitDriver, KvDriver, Storage};
use ramag_infra_clipboard::{HotkeyListener, MacClipboardDriver};
use ramag_infra_git::GitDriverImpl;
use ramag_infra_mongodb::MongoDriver;
use ramag_infra_mysql::MysqlDriver;
use ramag_infra_postgres::PostgresDriver;
use ramag_infra_redis::RedisDriver;
use ramag_infra_storage::RedbStorage;
use ramag_tool_clipboard::{
    ClipboardTool, CopySelectedClip, DeleteSelectedClip, FocusClipSearch, SelectNextClip,
    SelectPrevClip, create_clipboard_drawer, create_clipboard_view,
};
use ramag_tool_dbclient::{
    DbClientTool, ExplainQuery, FindInResults, FormatSql, NewQueryTab, RunQuery,
    RunStatementAtCursor, ToggleRedisConsole, ToggleSqlEditor, create_dbclient_view,
};
use ramag_tool_mongodb::{FormatMongoJson, NewMongoQueryTab, RunMongoQuery, ToggleMongoEditor};
use ramag_tool_vcs::{
    CommitNow, FocusCommitMessage, PullNow, PushNow, RefreshWorkspace, ToggleHistoryPane, VcsTool,
    create_vcs_view,
};
use ramag_ui::{
    CloseTab, HomeEvent, HomeView, Mode, NavTarget, RamagAssets, Shell, StorageGlobal, apply_theme,
    init_theme,
};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt as _;
use tracing_subscriber::util::SubscriberInitExt as _;

/// 绑 cmd-Q / macOS 菜单 Quit
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag)]
struct Quit;

fn main() {
    init_tracing();
    info!(version = env!("CARGO_PKG_VERSION"), "ramag launching");

    let (conn_service, storage) = match build_connection_service() {
        Ok(pair) => pair,
        Err(e) => {
            error!(error = %e, "failed to initialize data layer");
            std::process::exit(1);
        }
    };

    // Redis 共用同一 storage
    let redis_service: Arc<RedisService> = build_redis_service(storage.clone());
    // MongoDB 共用同一 storage
    let mongo_service: Arc<MongoService> = build_mongo_service(storage.clone());
    // 剪贴板共用同一 storage（历史与设置走同一份加密 redb）
    let clipboard_service: Arc<ClipboardService> = build_clipboard_service(storage.clone());

    // 主题偏好。None / "system" 跟随系统，"dark"/"light" 用户固定
    let initial_pref = read_theme_preference(&storage);

    let registry = build_tool_registry();
    info!(tool_count = registry.count(), "tools registered");

    let app = gpui_platform::application().with_assets(RamagAssets);

    // on_reopen 必须在 app.run 之前注册（属 Application）。仅当无活窗口时重开主窗口，避免 dock 叠加
    let registry_for_reopen = registry.clone();
    let conn_service_for_reopen = conn_service.clone();
    let redis_service_for_reopen = redis_service.clone();
    let mongo_service_for_reopen = mongo_service.clone();
    let clipboard_service_for_reopen = clipboard_service.clone();
    let storage_for_reopen = storage.clone();
    app.on_reopen(move |cx: &mut App| {
        if cx.windows().is_empty() {
            // 重开时再读，期间用户可能改过偏好
            let pref = read_theme_preference(&storage_for_reopen);
            open_main_window(
                registry_for_reopen.clone(),
                conn_service_for_reopen.clone(),
                redis_service_for_reopen.clone(),
                mongo_service_for_reopen.clone(),
                clipboard_service_for_reopen.clone(),
                storage_for_reopen.clone(),
                pref,
                cx,
            );
        }
    });

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        // 先 apply 占位主题避免窗口空白闪烁；正式主题在 open_main_window 拿 appearance 后定
        apply_theme(Mode::Dark, cx);
        // storage 注入 cx 全局，ActivityBar 切主题用它持久化
        cx.set_global(StorageGlobal(storage.clone()));
        cx.activate(true);

        // 必须先 bind_keys 把 cmd-q 绑到 Quit，NSMenuItem 才会显示快捷键
        cx.on_action(|_: &Quit, cx| cx.quit());

        // cmd-w 全局 fallback：视图层先消费（关 tab），没消费就关窗。
        // 关窗须 defer：此刻正处在该窗口的按键分发栈内（window 已被 take 出），
        // 直接 handle.update 会重入 take 失败而静默不关；defer 到本次分发结束后再移除
        cx.on_action(|_: &CloseTab, cx: &mut App| {
            let Some(handle) = cx
                .active_window()
                .or_else(|| cx.windows().into_iter().next())
            else {
                return;
            };
            cx.defer(move |cx| {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            });
        });

        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            // dbclient (MySQL / PG) 视图的快捷键（context=QueryPanel/QueryTab 见 dbclient 视图实现）
            KeyBinding::new("cmd-enter", RunQuery, None),
            KeyBinding::new("cmd-shift-enter", RunStatementAtCursor, None),
            KeyBinding::new("cmd-t", NewQueryTab, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-f", FindInResults, None),
            KeyBinding::new("cmd-shift-f", FormatSql, None),
            KeyBinding::new("cmd-shift-e", ExplainQuery, None),
            KeyBinding::new("cmd-e", ToggleSqlEditor, None),
            // MongoDB 视图的快捷键，用 KeyContext 限定（焦点在 Mongo 视图时优先）
            KeyBinding::new("cmd-enter", RunMongoQuery, Some("MongoQueryTab")),
            KeyBinding::new("cmd-t", NewMongoQueryTab, Some("MongoQueryPanel")),
            KeyBinding::new("cmd-shift-f", FormatMongoJson, Some("MongoQueryTab")),
            KeyBinding::new("cmd-e", ToggleMongoEditor, Some("MongoQueryPanel")),
            // Redis 命令行控制台：cmd-e 在 Redis 会话上下文切换显隐
            KeyBinding::new("cmd-e", ToggleRedisConsole, Some("RedisSession")),
            // VCS 视图快捷键（context=VcsView，焦点在 VCS 视图时优先于上面的 None context 绑定）
            KeyBinding::new("cmd-k", FocusCommitMessage, Some("VcsView")),
            KeyBinding::new("cmd-enter", CommitNow, Some("VcsView")),
            KeyBinding::new("cmd-shift-k", PushNow, Some("VcsView")),
            KeyBinding::new("cmd-t", PullNow, Some("VcsView")),
            KeyBinding::new("cmd-r", RefreshWorkspace, Some("VcsView")),
            KeyBinding::new("cmd-shift-h", ToggleHistoryPane, Some("VcsView")),
            // 剪贴板视图快捷键（KeyContext=ClipboardView，焦点在剪贴板视图时生效）
            KeyBinding::new("cmd-f", FocusClipSearch, Some("ClipboardView")),
            KeyBinding::new("enter", CopySelectedClip, Some("ClipboardView")),
            KeyBinding::new("delete", DeleteSelectedClip, Some("ClipboardView")),
            KeyBinding::new("backspace", DeleteSelectedClip, Some("ClipboardView")),
            KeyBinding::new("down", SelectNextClip, Some("ClipboardView")),
            KeyBinding::new("up", SelectPrevClip, Some("ClipboardView")),
        ]);

        // 启动时清理孤儿媒体文件（崩溃 / 库磁盘不一致残留）
        {
            let svc = clipboard_service.clone();
            cx.spawn(async move |_| {
                if let Err(e) = svc.cleanup_orphans().await {
                    tracing::warn!(error = %e, "clipboard orphan cleanup failed");
                }
            })
            .detach();
        }
        // 预热窗口缓存：解密最近 N 条入内存，让首次唤起抽屉即同步带满内容
        {
            let svc = clipboard_service.clone();
            cx.spawn(async move |_| svc.preload().await).detach();
        }
        // App 级剪贴板采集循环：独立于窗口生死，关窗后仍持续记录
        spawn_clipboard_capture(clipboard_service.clone(), cx);
        // 全局热键（cmd-shift-V）唤起底部悬浮抽屉
        spawn_clipboard_hotkey(clipboard_service.clone(), cx);

        cx.set_menus(vec![Menu {
            name: "Ramag".into(),
            items: vec![MenuItem::action("Quit Ramag", Quit)],
            disabled: false,
        }]);

        open_main_window(
            registry.clone(),
            conn_service.clone(),
            redis_service.clone(),
            mongo_service.clone(),
            clipboard_service.clone(),
            storage.clone(),
            initial_pref.clone(),
            cx,
        );
    });
}

/// 采集间隔。macOS 无剪贴板变更通知，所有同类工具均靠轮询 changeCount（开销极小）
const CAPTURE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(400);

/// App 级采集循环：仅在 changeCount 变化时加载设置 + 处理，避免每拍解密设置。
/// driver 的 NSPasteboard 读取在前台 executor（主线程）执行，符合 AppKit 约定
fn spawn_clipboard_capture(service: Arc<ClipboardService>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last_count = service.driver().change_count();
        loop {
            cx.background_executor().timer(CAPTURE_INTERVAL).await;
            let count = service.driver().change_count();
            if count == last_count {
                continue;
            }
            last_count = count;
            let settings = service.load_settings().await;
            if let Err(e) = service.capture_tick(&settings).await {
                tracing::warn!(error = %e, "clipboard capture tick failed");
            }
        }
    })
    .detach();
}

/// 热键轮询间隔：channel 有事件即触发，间隔短以保证唤起手感
const HOTKEY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(80);

/// 悬浮抽屉高度
const DRAWER_HEIGHT: f32 = 280.0;
/// 抽屉四周与屏幕可见区的留白
const DRAWER_MARGIN: f32 = 5.0;

/// 注册全局热键并轮询：触发切换抽屉；并在每拍检测失焦自动隐藏（点击外部即关）。
/// 热键随"启用采集"开关动态注册/注销——关采集即释放 ⌘⇧V，不再与其他应用冲突。
/// 注册失败（缺权限等）仅记日志，不影响其余功能
fn spawn_clipboard_hotkey(service: Arc<ClipboardService>, cx: &mut App) {
    cx.spawn(async move |cx| {
        // 启动读持久化采集开关：关闭则不注册，避免抢占 ⌘⇧V
        let mut enabled = service.prime_capture_enabled().await;
        let mut listener = if enabled {
            let l = HotkeyListener::register_cmd_shift_v();
            if l.is_none() {
                error!("global hotkey register failed; clipboard drawer disabled");
            }
            l
        } else {
            None
        };

        let mut drawer: Option<gpui::AnyWindowHandle> = None;
        // 抽屉是否曾真正激活过：避免刚打开（尚未激活）就被失焦逻辑误关
        let mut was_active = false;
        loop {
            cx.background_executor().timer(HOTKEY_POLL_INTERVAL).await;

            // 采集开关变化 → 动态注册/注销热键
            let now_enabled = service.capture_enabled();
            if now_enabled != enabled {
                enabled = now_enabled;
                if enabled {
                    listener = HotkeyListener::register_cmd_shift_v();
                    if listener.is_none() {
                        error!("global hotkey re-register failed");
                    }
                } else {
                    // 置 None 触发 Drop 注销热键并移除 handler，释放 ⌘⇧V
                    listener = None;
                    // 关闭残留抽屉：热键已注销，否则无法再 toggle 关闭
                    if let Some(handle) = drawer.take() {
                        let _ = cx
                            .update(|cx| handle.update(cx, |_, window, _| window.remove_window()));
                        was_active = false;
                    }
                }
            }

            // 失焦自动隐藏：曾激活过又失去激活态 = 用户点了别处
            if let Some(handle) = &drawer {
                let active = cx.update(|cx| {
                    handle
                        .update(cx, |_, window, _| window.is_window_active())
                        .unwrap_or(false)
                });
                if active {
                    was_active = true;
                } else if was_active {
                    let _ =
                        cx.update(|cx| handle.update(cx, |_, window, _| window.remove_window()));
                    drawer = None;
                    was_active = false;
                }
            }

            // 采集关闭（无 listener）→ 跳过热键轮询
            let Some(listener) = &listener else {
                continue;
            };
            if !listener.poll() {
                continue;
            }
            // 已打开 → 关闭（toggle）
            if let Some(handle) = drawer.take() {
                let _ = cx.update(|cx| handle.update(cx, |_, window, _| window.remove_window()));
                was_active = false;
                continue;
            }
            // 未打开 → 唤起：记录前台应用后开抽屉
            let svc = service.clone();
            drawer = cx.update(|cx| open_drawer_window(svc, cx));
            was_active = false;
        }
    })
    .detach();
}

/// 在主显示器底部打开满宽 Floating 抽屉窗口。
/// 用 Floating（非 PopUp）+ 激活 app，搜索框输入法（中文）才能工作；可见区贴底避开 Dock
fn open_drawer_window(
    service: Arc<ClipboardService>,
    cx: &mut App,
) -> Option<gpui::AnyWindowHandle> {
    let target_bundle = service.driver().frontmost_app().map(|s| s.bundle_id);

    let display = cx.primary_display()?;
    // visible_bounds 排除菜单栏 / Dock；四周留 margin，不贴边
    let db = display.visible_bounds();
    let x = db.origin.x.to_f64() as f32 + DRAWER_MARGIN;
    let screen_y = db.origin.y.to_f64() as f32;
    let width = db.size.width.to_f64() as f32 - DRAWER_MARGIN * 2.0;
    let screen_h = db.size.height.to_f64() as f32;
    let y = screen_y + screen_h - DRAWER_HEIGHT - DRAWER_MARGIN;
    let bounds = Bounds {
        origin: point(px(x), px(y)),
        size: Size {
            width: px(width),
            height: px(DRAWER_HEIGHT),
        },
    };

    // PopUp + 激活 app：PopUp 自带 CanJoinAllSpaces（全屏 Space 也能弹出）；
    // cx.activate 让 app active，搜索框输入法（中文）方可工作；粘贴时再激活回原应用
    let result = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: None,
            kind: WindowKind::PopUp,
            is_movable: false,
            focus: true,
            show: true,
            ..Default::default()
        },
        move |window, cx| {
            let drawer = create_clipboard_drawer(service, target_bundle, window, cx);
            cx.new(|cx| Root::new(drawer, window, cx))
        },
    );
    cx.activate(true);
    match result {
        Ok(handle) => Some(handle.into()),
        Err(e) => {
            error!(error = %e, "open drawer window failed");
            None
        }
    }
}

/// init / on_reopen 共用
#[allow(clippy::too_many_arguments)]
fn open_main_window(
    registry: Arc<ToolRegistry>,
    conn_service: Arc<ConnectionService>,
    redis_service: Arc<RedisService>,
    mongo_service: Arc<MongoService>,
    clipboard_service: Arc<ClipboardService>,
    storage: Arc<dyn Storage>,
    theme_pref: Option<String>,
    cx: &mut App,
) {
    // Maximized 需 fallback Bounds 给取消最大化复位
    let bounds = Bounds::centered(None, size(px(1200.0), px(780.0)), cx);

    cx.spawn(async move |cx| {
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(bounds)),
                window_min_size: Some(size(px(800.0), px(500.0))),
                // 原生标题栏需 appears_transparent=false，否则失去双击 zoom 命中区
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            move |window, cx| {
                // 拿 window.appearance 后才能正式 init 主题
                init_theme(theme_pref.as_deref(), window.appearance(), cx);

                let home_view =
                    cx.new(|cx| HomeView::new(registry.clone(), conn_service.clone(), cx));

                let dbclient_view = create_dbclient_view(
                    conn_service.clone(),
                    redis_service.clone(),
                    mongo_service.clone(),
                    window,
                    cx,
                );

                let git_driver: Arc<dyn GitDriver> = Arc::new(GitDriverImpl::new());
                let vcs_view = create_vcs_view(git_driver, storage.clone(), window, cx);

                let clipboard_view = create_clipboard_view(clipboard_service.clone(), window, cx);

                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(registry.clone(), window, cx);
                    shell.set_home_view(home_view.clone().into());
                    shell.register_tool_view(DbClientTool::ID, dbclient_view);
                    shell.register_tool_view(VcsTool::ID, vcs_view.into());
                    shell.register_tool_view(ClipboardTool::ID, clipboard_view.into());

                    let _sub: Subscription = cx.subscribe_in(
                        &home_view,
                        window,
                        move |this: &mut Shell, _, event: &HomeEvent, window, cx| match event {
                            HomeEvent::OpenTool(tool_id) => {
                                this.navigate_to(NavTarget::Tool(tool_id.clone()), window, cx);
                            }
                            HomeEvent::OpenConnection(_id) => {
                                this.navigate_to(
                                    NavTarget::Tool(DbClientTool::ID.to_string()),
                                    window,
                                    cx,
                                );
                            }
                        },
                    );
                    // 让订阅活到 Shell 一样长
                    std::mem::forget(_sub);

                    shell
                });

                cx.new(|cx| Root::new(shell, window, cx))
            },
        );
        if let Err(err) = result {
            error!(error = %err, "open window failed");
        }
    })
    .detach();
}

/// 注册 SQL 类 driver 到 `HashMap<DriverKind, Arc<dyn Driver>>`，按 `config.driver` 分发；Redis 走独立 service
fn build_connection_service() -> anyhow::Result<(Arc<ConnectionService>, Arc<dyn Storage>)> {
    use ramag_domain::entities::DriverKind;
    use std::collections::HashMap;

    let mut drivers: HashMap<DriverKind, Arc<dyn Driver>> = HashMap::new();
    drivers.insert(DriverKind::Mysql, Arc::new(MysqlDriver::new()));
    drivers.insert(DriverKind::Postgres, Arc::new(PostgresDriver::new()));

    let storage_impl =
        RedbStorage::open_default().map_err(|e| anyhow::anyhow!("初始化 redb 存储失败: {e}"))?;
    info!(path = %storage_impl.path().display(), "storage opened");
    let storage: Arc<dyn Storage> = Arc::new(storage_impl);

    let svc = Arc::new(ConnectionService::new(drivers, storage.clone()));
    Ok((svc, storage))
}

/// 任何错误都返回 None，等价跟随系统
fn read_theme_preference(storage: &Arc<dyn Storage>) -> Option<String> {
    let storage = storage.clone();
    let rt = tokio::runtime::Runtime::new().ok()?;
    rt.block_on(async move { storage.get_preference("theme_mode").await.ok().flatten() })
}

/// MySQL / Postgres / Redis 共用 DbClient 入口，driver 在表单选择器内
fn build_tool_registry() -> Arc<ToolRegistry> {
    let registry = Arc::new(ToolRegistry::new());
    registry.register(Arc::new(DbClientTool::new()));
    registry.register(Arc::new(VcsTool::new()));
    registry.register(Arc::new(ClipboardTool::new()));
    registry
}

fn build_redis_service(storage: Arc<dyn Storage>) -> Arc<RedisService> {
    let driver: Arc<dyn KvDriver> = Arc::new(RedisDriver::new());
    Arc::new(RedisService::new(driver, storage))
}

fn build_mongo_service(storage: Arc<dyn Storage>) -> Arc<MongoService> {
    let driver: Arc<dyn DocDriver> = Arc::new(MongoDriver::new());
    Arc::new(MongoService::new(driver, storage))
}

fn build_clipboard_service(storage: Arc<dyn Storage>) -> Arc<ClipboardService> {
    let driver: Arc<dyn ClipboardDriver> = Arc::new(MacClipboardDriver::new());
    Arc::new(ClipboardService::new(driver, storage))
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ramag=debug"));

    // stderr：cargo run 直观；DMG 装后写 macOS 系统日志。文件层另存一份方便自查
    let stderr_layer = tracing_subscriber::fmt::layer()
        .with_writer(std::io::stderr)
        .with_target(false);

    let log_path = log_file_path();
    let file_layer = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok()
        .map(|f| {
            tracing_subscriber::fmt::layer()
                .with_writer(std::sync::Mutex::new(f))
                .with_target(false)
                .with_ansi(false)
        });

    tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .init();

    eprintln!("ramag log file: {}", log_path.display());
    info!(log = %log_path.display(), "log file ready");
}

/// macOS：~/Library/Application Support/com.ramag.ramag/logs/ramag.log
/// 定位失败退回临时目录，保证 init_tracing 不 panic
fn log_file_path() -> std::path::PathBuf {
    let dir = directories::ProjectDirs::from("com", "ramag", "ramag")
        .map(|p| p.data_dir().join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("ramag-logs"));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ramag.log")
}
