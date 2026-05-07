//! 主入口：tracing → 装配数据层 → 注册 Tool → 启动 GPUI App → 打开主窗口

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gpui::{
    Action, App, Bounds, KeyBinding, Menu, MenuItem, Subscription, TitlebarOptions, WindowBounds,
    WindowOptions, prelude::*, px, size,
};
use gpui_component::Root;
use ramag_app::{ConnectionService, RedisService, ToolRegistry};
use ramag_domain::traits::{Driver, GitDriver, KvDriver, Storage};
use ramag_infra_git::GitDriverImpl;
use ramag_infra_mysql::MysqlDriver;
use ramag_infra_postgres::PostgresDriver;
use ramag_infra_redis::RedisDriver;
use ramag_infra_storage::RedbStorage;
use ramag_tool_dbclient::{
    DbClientTool, ExplainQuery, FindInResults, FormatSql, NewQueryTab, RunQuery,
    RunStatementAtCursor, SaveSqlFile, ToggleHistory, ToggleSqlEditor, create_dbclient_view,
};
use ramag_tool_vcs::{VcsTool, create_vcs_view};
use ramag_ui::{
    CloseTab, HomeEvent, HomeView, Mode, NavTarget, RamagAssets, Shell, StorageGlobal, apply_theme,
    init_theme,
};
use schemars::JsonSchema;
use serde::Deserialize;
use tracing::{Level, error, info};
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

    // 主题偏好。None / "system" 跟随系统，"dark"/"light" 用户固定
    let initial_pref = read_theme_preference(&storage);

    let registry = build_tool_registry();
    info!(tool_count = registry.count(), "tools registered");

    let app = gpui_platform::application().with_assets(RamagAssets);

    // on_reopen 必须在 app.run 之前注册（属 Application）。仅当无活窗口时重开主窗口，避免 dock 叠加
    let registry_for_reopen = registry.clone();
    let conn_service_for_reopen = conn_service.clone();
    let redis_service_for_reopen = redis_service.clone();
    let storage_for_reopen = storage.clone();
    app.on_reopen(move |cx: &mut App| {
        if cx.windows().is_empty() {
            // 重开时再读，期间用户可能改过偏好
            let pref = read_theme_preference(&storage_for_reopen);
            open_main_window(
                registry_for_reopen.clone(),
                conn_service_for_reopen.clone(),
                redis_service_for_reopen.clone(),
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

        // cmd-w 全局 fallback：视图层先消费（关 tab），没消费就关窗
        cx.on_action(|_: &CloseTab, cx: &mut App| {
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        });

        cx.bind_keys([
            KeyBinding::new("cmd-q", Quit, None),
            KeyBinding::new("cmd-enter", RunQuery, None),
            KeyBinding::new("cmd-shift-enter", RunStatementAtCursor, None),
            KeyBinding::new("cmd-t", NewQueryTab, None),
            KeyBinding::new("cmd-w", CloseTab, None),
            KeyBinding::new("cmd-f", FindInResults, None),
            KeyBinding::new("cmd-shift-f", FormatSql, None),
            KeyBinding::new("cmd-shift-e", ExplainQuery, None),
            KeyBinding::new("cmd-s", SaveSqlFile, None),
            KeyBinding::new("cmd-shift-h", ToggleHistory, None),
            KeyBinding::new("cmd-e", ToggleSqlEditor, None),
        ]);

        cx.set_menus(vec![Menu {
            name: "Ramag".into(),
            items: vec![MenuItem::action("Quit Ramag", Quit)],
            disabled: false,
        }]);

        open_main_window(
            registry.clone(),
            conn_service.clone(),
            redis_service.clone(),
            storage.clone(),
            initial_pref.clone(),
            cx,
        );
    });
}

/// init / on_reopen 共用
fn open_main_window(
    registry: Arc<ToolRegistry>,
    conn_service: Arc<ConnectionService>,
    redis_service: Arc<RedisService>,
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

                let dbclient_view =
                    create_dbclient_view(conn_service.clone(), redis_service.clone(), window, cx);

                let git_driver: Arc<dyn GitDriver> = Arc::new(GitDriverImpl::new());
                let vcs_view = create_vcs_view(git_driver, storage.clone(), window, cx);

                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(registry.clone(), window, cx);
                    shell.set_home_view(home_view.clone().into());
                    shell.register_tool_view(DbClientTool::ID, dbclient_view);
                    shell.register_tool_view(VcsTool::ID, vcs_view.into());

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
    registry
}

fn build_redis_service(storage: Arc<dyn Storage>) -> Arc<RedisService> {
    let driver: Arc<dyn KvDriver> = Arc::new(RedisDriver::new());
    Arc::new(RedisService::new(driver, storage))
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

    let _ = Level::TRACE;
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
