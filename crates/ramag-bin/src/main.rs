//! Ramag 主二进制入口
//!
//! 启动流程：
//! 1. 初始化 tracing 日志
//! 2. 构造数据层（MysqlDriver + RedbStorage + ConnectionService）
//! 3. 构建 ToolRegistry 注册 tools
//! 4. 启动 GPUI App，初始化 gpui-component + 应用 VSCode 暗色主题
//! 5. 打开主窗口（Shell + HomeView 默认在首页）

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;

use gpui::{
    Action, App, Bounds, KeyBinding, Menu, MenuItem, Subscription, TitlebarOptions, WindowBounds,
    WindowOptions, prelude::*, px, size,
};
use gpui_component::Root;
use ramag_app::{ConnectionService, RedisService, ToolRegistry};
use ramag_domain::traits::{Driver, KvDriver, Storage};
use ramag_infra_mysql::MysqlDriver;
use ramag_infra_postgres::PostgresDriver;
use ramag_infra_redis::RedisDriver;
use ramag_infra_storage::RedbStorage;
use ramag_tool_dbclient::{
    DbClientTool, ExplainQuery, FindInResults, FormatSql, NewQueryTab, RunQuery,
    RunStatementAtCursor, SaveSqlFile, ToggleHistory, ToggleSqlEditor, create_dbclient_view,
};
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

/// 应用退出（绑定到 macOS 菜单 Quit / ⌘Q）
#[derive(Clone, Copy, PartialEq, Eq, Debug, Deserialize, JsonSchema, Action)]
#[action(namespace = ramag)]
struct Quit;

fn main() {
    init_tracing();
    info!(version = env!("CARGO_PKG_VERSION"), "ramag launching");

    // === 数据层装配 ===
    let (conn_service, storage) = match build_connection_service() {
        Ok(pair) => pair,
        Err(e) => {
            error!(error = %e, "failed to initialize data layer");
            std::process::exit(1);
        }
    };

    // Redis 数据层（共用 storage，独立 driver）
    let redis_service: Arc<RedisService> = build_redis_service(storage.clone());

    // 启动前读偏好里保存的主题模式（None / "system" → 跟随系统，需要 window.appearance() 才能定）
    let initial_pref = read_theme_preference(&storage);

    // === Tool 注册 ===
    let registry = build_tool_registry();
    info!(tool_count = registry.count(), "tools registered");

    // === GPUI App ===
    let app = gpui_platform::application().with_assets(RamagAssets);

    // dock 图标点击 / 应用激活（红 X 关窗后 macOS 仍保留 app）：重开主窗口
    // 仅在没有任何活窗口时才开新窗口，避免 dock 多次点击叠加
    // 必须在 app.run 之前注册（on_reopen 在 Application 上，不是 run 内的 App）
    let registry_for_reopen = registry.clone();
    let conn_service_for_reopen = conn_service.clone();
    let redis_service_for_reopen = redis_service.clone();
    let storage_for_reopen = storage.clone();
    app.on_reopen(move |cx: &mut App| {
        if cx.windows().is_empty() {
            // dock 重开时再次读 preference（用户期间可能改过；保持一致性）
            let pref = read_theme_preference(&storage_for_reopen);
            open_main_window(
                registry_for_reopen.clone(),
                conn_service_for_reopen.clone(),
                redis_service_for_reopen.clone(),
                pref,
                cx,
            );
        }
    });

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        // 先 apply 一个临时主题占位（避免窗口打开瞬间空白）；
        // 实际主题在 open_main_window 内拿到 window.appearance() 后由 init_theme 确定
        apply_theme(Mode::Dark, cx);
        // 把 storage 注入到 cx 全局，让 ActivityBar 切换主题时能持久化
        cx.set_global(StorageGlobal(storage.clone()));
        cx.activate(true);

        // ⌘Q 退出：必须先 bind_keys 把 cmd-q 绑到 Quit Action，菜单项才能在 NSMenuItem
        // 上自动显示快捷键并响应（GPUI macOS menu 实现是从 keymap 反查 keystroke）
        cx.on_action(|_: &Quit, cx| cx.quit());

        // ⌘W 全局 fallback：先让视图层处理（MySQL QueryPanel / Redis Session 关 tab），
        // 没有消费（HomeView / 仅剩主区 tab）就走到这里关窗
        // macOS 习惯：关最后一个窗后保留 app（on_reopen 已处理 dock 点击重开）
        cx.on_action(|_: &CloseTab, cx: &mut App| {
            if let Some(handle) = cx.active_window() {
                let _ = handle.update(cx, |_, window, _| window.remove_window());
            }
        });

        // 注册全局快捷键（含 cmd-q）
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
            initial_pref.clone(),
            cx,
        );
    });
}

/// 打开主窗口（init / on_reopen 共用）
fn open_main_window(
    registry: Arc<ToolRegistry>,
    conn_service: Arc<ConnectionService>,
    redis_service: Arc<RedisService>,
    theme_pref: Option<String>,
    cx: &mut App,
) {
    // 启动时窗口最大化（macOS 等价于 zoom 到屏幕可用区）
    // Maximized 仍需要传一个 fallback Bounds，供用户取消最大化时复位
    let bounds = Bounds::centered(None, size(px(1200.0), px(780.0)), cx);

    cx.spawn(async move |cx| {
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Maximized(bounds)),
                window_min_size: Some(size(px(800.0), px(500.0))),
                // macOS 原生标题栏：实体（不透明）+ 无文字
                // 透明会让 macOS 失去"双击标题栏 zoom"的命中区，所以保持实体
                titlebar: Some(TitlebarOptions {
                    title: None,
                    appears_transparent: false,
                    traffic_light_position: None,
                }),
                ..Default::default()
            },
            move |window, cx| {
                // 0. 拿到 window.appearance 后正式 init 主题：
                //    - preference 已设 dark/light → 使用该值，关闭跟随系统
                //    - preference 空/system → 用系统当前外观，开启跟随系统（之后系统切换会自动同步）
                init_theme(theme_pref.as_deref(), window.appearance(), cx);

                // 1. 创建 Home 视图
                let home_view =
                    cx.new(|cx| HomeView::new(registry.clone(), conn_service.clone(), cx));

                // 2. 创建 DB Client 视图（统一管理 MySQL + Redis 连接，driver 在表单内选）
                let dbclient_view =
                    create_dbclient_view(conn_service.clone(), redis_service.clone(), window, cx);

                // 3. 创建 Shell 并注入视图
                let shell = cx.new(|cx| {
                    let mut shell = Shell::new(registry.clone(), window, cx);
                    shell.set_home_view(home_view.clone().into());
                    shell.register_tool_view(DbClientTool::ID, dbclient_view);

                    // 监听 HomeView 的事件，转换成 Shell 导航
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
                    // 让订阅活到 Shell 一样长（Shell 内部的 Subscriptions 会保管）
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

/// 装配数据层：所有 SQL driver + RedbStorage + ConnectionService
///
/// SQL 类 driver 注册到 HashMap<DriverKind, Arc<dyn Driver>>；ConnectionService 按
/// `config.driver` 分发。Redis 走独立 RedisService，不在此 service 内
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

/// 启动时读取主题偏好的原始字符串
///
/// 返回 None / Some("system") → 跟随系统；Some("dark"|"light") → 用户已显式选过
/// 任何错误（rt 创建失败 / storage 读失败）都返回 None，等价于"跟随系统"
fn read_theme_preference(storage: &Arc<dyn Storage>) -> Option<String> {
    let storage = storage.clone();
    let rt = tokio::runtime::Runtime::new().ok()?;
    rt.block_on(async move { storage.get_preference("theme_mode").await.ok().flatten() })
}

/// 注册所有 Tool 到 Registry
///
/// MySQL + Redis 都归入 DbClient 工具（统一"数据源管理"入口），
/// 区别仅在新建连接表单的 driver 选择器内体现
fn build_tool_registry() -> Arc<ToolRegistry> {
    let registry = Arc::new(ToolRegistry::new());
    registry.register(Arc::new(DbClientTool::new()));
    registry
}

/// 装配 Redis 数据层：RedisDriver + 共用 Storage
fn build_redis_service(storage: Arc<dyn Storage>) -> Arc<RedisService> {
    let driver: Arc<dyn KvDriver> = Arc::new(RedisDriver::new());
    Arc::new(RedisService::new(driver, storage))
}

fn init_tracing() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,ramag=debug"));

    // stderr 层：开发时 cargo run 直接看；DMG 安装后 stderr 默认重定向到 macOS 系统日志
    // 文件层：所有运行时都写一份到固定路径，方便用户自查（尤其是错误日志）
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

    // 启动时同时往 stderr 和文件打一行路径，便于 .app 安装后直接定位日志
    let _ = Level::TRACE; // 保留 Level import 给未来需要时用
    eprintln!("ramag log file: {}", log_path.display());
    info!(log = %log_path.display(), "log file ready");
}

/// 日志文件路径
/// macOS：~/Library/Application Support/com.ramag.ramag/logs/ramag.log
/// Linux：~/.local/share/ramag/logs/ramag.log
/// 失败时退回临时目录，保证 init_tracing 不 panic
fn log_file_path() -> std::path::PathBuf {
    let dir = directories::ProjectDirs::from("com", "ramag", "ramag")
        .map(|p| p.data_dir().join("logs"))
        .unwrap_or_else(|| std::env::temp_dir().join("ramag-logs"));
    let _ = std::fs::create_dir_all(&dir);
    dir.join("ramag.log")
}
