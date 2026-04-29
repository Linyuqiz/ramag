//! Redis 连接会话面板（dbclient 装载，作为 Redis 连接的会话视图）
//!
//! 布局：
//! ```text
//! ┌──────────────────┬──────────────────────────────────────────┐
//! │ DB ▾ 0 共7 keys  │ [🔑 Key:foo][▶ CLI 1 ✕][📡 PubSub 1 ✕]  [⌘][📡] │
//! │ ─────────────    ├──────────────────────────────────────────┤
//! │ 🔍 [+][▼][▶][↻]  │                                          │
//! │ user/            │   active tab 内容                          │
//! │  ├ 1001          │   （KeyDetail 主区 / CLI 实例 / PubSub 实例） │
//! │  └ 1002          │                                          │
//! └──────────────────┴──────────────────────────────────────────┘
//! ```
//!
//! tab 模型：
//! - 第一个 tab 固定 = KeyDetail（永远在 index 0，不可关闭，标题随当前 key 切换）
//! - 后续 tab = CLI / PubSub（右上角图标点一下加一个，可关闭）
//! - 点 key 树 → 切到 KeyDetail tab + 加载该 key
//! - DB 切换 → 主区清空 + CLI 同步 db；不动 tab 列表

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyView, App, Context, Entity, IntoElement, ParentElement, Point, Render, ScrollHandle,
    SharedString, Styled, Subscription, Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    resizable::{ResizableState, h_resizable, resizable_panel},
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use ramag_ui::CloseTab;
use tracing::info;

use crate::views::cli_panel::{CliEvent, CliPanel};
use crate::views::hash_field_form::{HashFieldForm, HashFieldFormEvent, HashFieldFormMode};
use crate::views::key_create::{KeyCreateEvent, KeyCreateForm};
use crate::views::key_detail::{KeyDetailEvent, KeyDetailPanel};
use crate::views::key_tree::{KeyTreeEvent, KeyTreePanel};
use crate::views::list_element_form::{ListElementForm, ListElementFormEvent};
use crate::views::pubsub_panel::PubSubPanel;
use crate::views::set_element_form::{SetElementForm, SetElementFormEvent};
use crate::views::stream_entry_form::{StreamEntryForm, StreamEntryFormEvent};
use crate::views::ttl_edit::{TtlEditEvent, TtlEditForm};
use crate::views::value_edit::{ValueEditEvent, ValueEditForm};
use crate::views::zset_element_form::{ZSetElementForm, ZSetElementFormEvent, ZSetElementFormMode};

const TREE_WIDTH_INITIAL: f32 = 320.0;
const TREE_WIDTH_MIN: f32 = 200.0;
const TREE_WIDTH_MAX: f32 = 600.0;

/// 工具 tab（KeyDetail 不在此，单独固定占第一个 tab 位）
enum ToolTab {
    Cli {
        id: u64,
        panel: Entity<CliPanel>,
    },
    PubSub {
        id: u64,
        panel: Entity<PubSubPanel>,
    },
}

impl ToolTab {
    fn label(&self) -> String {
        match self {
            ToolTab::Cli { id, .. } => format!("CLI {id}"),
            ToolTab::PubSub { id, .. } => format!("Pub/Sub {id}"),
        }
    }
    fn icon_glyph(&self) -> &'static str {
        match self {
            ToolTab::Cli { .. } => "▶",
            ToolTab::PubSub { .. } => "📡",
        }
    }
    fn to_view(&self) -> AnyView {
        match self {
            ToolTab::Cli { panel, .. } => panel.clone().into(),
            ToolTab::PubSub { panel, .. } => panel.clone().into(),
        }
    }
}

/// 当前激活的 tab：Detail 占固定第一位，Tool(i) 是 tools[i]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActiveTab {
    Detail,
    Tool(usize),
}

pub struct RedisSessionPanel {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    tree: Entity<KeyTreePanel>,
    /// 主区固定一个 KeyDetail，点 key 树就 load_key 切换显示
    detail: Entity<KeyDetailPanel>,
    /// CLI / PubSub 工具 tab 列表（懒添加，可关闭）
    tools: Vec<ToolTab>,
    /// 当前激活的 tab
    active: ActiveTab,
    /// 工具 tab 自增 id（CLI 1 / CLI 2 / Pub/Sub 1 …）
    next_tool_id: u64,
    resize_state: Entity<ResizableState>,
    /// tab bar 横向滚动句柄：tab 多溢出时新建后滚到末尾
    workspace_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl RedisSessionPanel {
    pub fn new(
        config: ConnectionConfig,
        service: Arc<RedisService>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let initial_db: u8 = config
            .database
            .as_deref()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        let tree = cx.new(|cx| KeyTreePanel::new(service.clone(), window, cx));
        let conn_for_tree = config.clone();
        tree.update(cx, |t, cx| {
            t.set_connection(Some(conn_for_tree), initial_db, cx)
        });

        // 主区 KeyDetail：单实例 + 初始即聚焦面板，确保 ⌘W 等 action 链路通畅
        let svc = service.clone();
        let detail = cx.new(|cx| KeyDetailPanel::new(svc, cx));
        detail.update(cx, |p, cx| {
            p.set_connection(Some(config.clone()), initial_db, cx);
            p.focus_panel(window, cx);
        });

        // 树事件：选中 → detail.load_key + 切回 KeyDetail tab；新建 → 弹 dialog；DB 切换 → 同步
        let mut subs = Vec::new();
        subs.push(cx.subscribe_in(
            &tree,
            window,
            move |this: &mut Self, _, e: &KeyTreeEvent, window, cx| match e {
                KeyTreeEvent::Selected(key) => {
                    let key_clone = key.clone();
                    this.detail.update(cx, |p, cx_inner| {
                        p.load_key(key_clone, cx_inner);
                        // 选中 key 后把焦点收到 Detail，让 ⌘W 等 action 走 Session 监听
                        p.focus_panel(window, cx_inner);
                    });
                    this.active = ActiveTab::Detail;
                    cx.notify();
                }
                KeyTreeEvent::RequestCreate => {
                    this.open_create_dialog(window, cx);
                }
                KeyTreeEvent::DbSelected(db) => {
                    this.handle_db_change(*db, cx);
                }
            },
        ));

        // 详情事件：删除完成刷新树 + 编辑请求转弹窗 + 各种二次确认
        subs.push(cx.subscribe_in(
            &detail,
            window,
            move |this: &mut Self, _, e: &KeyDetailEvent, window, cx| match e {
                KeyDetailEvent::Deleted(_) => {
                    this.tree.update(cx, |t, cx| t.refresh(cx));
                }
                KeyDetailEvent::RequestEditTtl(key, ttl_ms) => {
                    this.open_ttl_dialog(key.clone(), *ttl_ms, window, cx);
                }
                KeyDetailEvent::RequestEditValue(key, value) => {
                    this.open_value_dialog(key.clone(), value.clone(), window, cx);
                }
                KeyDetailEvent::RequestAddHashField(key) => {
                    this.open_hash_field_dialog(
                        key.clone(),
                        HashFieldFormMode::Add,
                        String::new(),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestEditHashField(key, field, current_value) => {
                    this.open_hash_field_dialog(
                        key.clone(),
                        HashFieldFormMode::Edit {
                            field: field.clone(),
                        },
                        current_value.clone(),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestAddListElement(key) => {
                    this.open_list_element_dialog(key.clone(), window, cx);
                }
                KeyDetailEvent::RequestAddSetElement(key) => {
                    this.open_set_element_dialog(key.clone(), window, cx);
                }
                KeyDetailEvent::RequestAddZSetElement(key) => {
                    this.open_zset_element_dialog(
                        key.clone(),
                        ZSetElementFormMode::Add,
                        String::new(),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestEditZSetScore(key, member, score) => {
                    this.open_zset_element_dialog(
                        key.clone(),
                        ZSetElementFormMode::EditScore {
                            member: member.clone(),
                        },
                        score.clone(),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestAddStreamEntry(key) => {
                    this.open_stream_entry_dialog(key.clone(), window, cx);
                }
                KeyDetailEvent::RequestDeleteKey(key) => {
                    let panel_for_run = this.detail.clone();
                    this.confirm_delete_op(
                        "删除 Key？".into(),
                        format!(
                            "将永久删除 key「{}」，此操作不可撤销。",
                            truncate_for_dialog(key, 80)
                        ),
                        Rc::new(move |_w, app| {
                            panel_for_run.update(app, |p, cx| p.delete_key_now(cx));
                        }),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestDeleteHashField(_key, field) => {
                    let panel_for_run = this.detail.clone();
                    let field = field.clone();
                    let field_label = truncate_for_dialog(&field, 80);
                    this.confirm_delete_op(
                        "删除 Hash 字段？".into(),
                        format!("将删除字段「{field_label}」，此操作不可撤销。"),
                        Rc::new(move |_w, app| {
                            let field = field.clone();
                            panel_for_run.update(app, |p, cx| p.delete_hash_field(field, cx));
                        }),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestDeleteListElement(_key, value, idx) => {
                    let panel_for_run = this.detail.clone();
                    let value = value.clone();
                    let idx_v = *idx;
                    let value_label = truncate_for_dialog(&value, 80);
                    this.confirm_delete_op(
                        "删除 List 元素？".into(),
                        format!(
                            "将删除序号 {idx_v} 的元素「{value_label}」（按值首匹配，仅删 1 个），\
                             此操作不可撤销。"
                        ),
                        Rc::new(move |_w, app| {
                            let value = value.clone();
                            panel_for_run.update(app, |p, cx| p.delete_list_element(value, cx));
                        }),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestDeleteSetElement(_key, member) => {
                    let panel_for_run = this.detail.clone();
                    let member = member.clone();
                    let member_label = truncate_for_dialog(&member, 80);
                    this.confirm_delete_op(
                        "删除 Set 成员？".into(),
                        format!("将删除成员「{member_label}」，此操作不可撤销。"),
                        Rc::new(move |_w, app| {
                            let member = member.clone();
                            panel_for_run.update(app, |p, cx| p.delete_set_element(member, cx));
                        }),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestDeleteZSetMember(_key, member) => {
                    let panel_for_run = this.detail.clone();
                    let member = member.clone();
                    let member_label = truncate_for_dialog(&member, 80);
                    this.confirm_delete_op(
                        "删除 ZSet 成员？".into(),
                        format!("将删除成员「{member_label}」，此操作不可撤销。"),
                        Rc::new(move |_w, app| {
                            let member = member.clone();
                            panel_for_run.update(app, |p, cx| p.delete_zset_member(member, cx));
                        }),
                        window,
                        cx,
                    );
                }
                KeyDetailEvent::RequestDeleteStreamEntry(_key, entry_id) => {
                    let panel_for_run = this.detail.clone();
                    let entry_id = entry_id.clone();
                    let id_label = truncate_for_dialog(&entry_id, 80);
                    this.confirm_delete_op(
                        "删除 Stream 条目？".into(),
                        format!("将删除条目「{id_label}」，此操作不可撤销。"),
                        Rc::new(move |_w, app| {
                            let entry_id = entry_id.clone();
                            panel_for_run.update(app, |p, cx| p.delete_stream_entry(entry_id, cx));
                        }),
                        window,
                        cx,
                    );
                }
            },
        ));

        let resize_state = cx.new(|_| ResizableState::default());

        Self {
            service,
            config,
            db: initial_db,
            tree,
            detail,
            tools: Vec::new(),
            active: ActiveTab::Detail,
            next_tool_id: 1,
            resize_state,
            workspace_scroll: ScrollHandle::new(),
            _subscriptions: subs,
        }
    }

    /// 新建工具 tab 后调一次：让 tab bar 自动滚到末尾，新 tab 立即可见
    fn scroll_tabs_to_end(&mut self) {
        self.workspace_scroll
            .set_offset(Point::new(px(-99999.0), px(0.0)));
    }

    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    pub fn title(&self) -> &str {
        &self.config.name
    }

    fn alloc_tool_id(&mut self) -> u64 {
        let id = self.next_tool_id;
        self.next_tool_id += 1;
        id
    }

    fn open_cli_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.alloc_tool_id();
        let svc = self.service.clone();
        let cfg = self.config.clone();
        let db = self.db;
        let panel = cx.new(|cx| CliPanel::new(svc, cfg, db, window, cx));
        // 危险命令二次确认订阅
        let panel_for_confirm = panel.clone();
        let sub = cx.subscribe_in(
            &panel,
            window,
            move |this: &mut Self, _, ev: &CliEvent, window, cx| match ev {
                CliEvent::RequestDangerConfirm(raw, argv) => {
                    this.confirm_dangerous_cmd(
                        raw.clone(),
                        argv.clone(),
                        panel_for_confirm.clone(),
                        window,
                        cx,
                    );
                }
            },
        );
        self._subscriptions.push(sub);
        // 自动聚焦底部命令输入框：用户新建 CLI 后立刻可以打字
        panel.update(cx, |p, cx| p.focus_input(window, cx));
        self.tools.push(ToolTab::Cli { id, panel });
        let new_idx = self.tools.len() - 1;
        self.active = ActiveTab::Tool(new_idx);
        info!(tool = "cli", id, new_idx, "open tool tab + activate");
        self.scroll_tabs_to_end();
        cx.notify();
    }

    fn open_pubsub_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.alloc_tool_id();
        let svc = self.service.clone();
        let cfg = self.config.clone();
        let panel = cx.new(|cx| PubSubPanel::new(svc, cfg, window, cx));
        // 自动聚焦顶部 channel 输入框：用户新建 Pub/Sub 后立刻可以填 channel
        panel.update(cx, |p, cx| p.focus_input(window, cx));
        self.tools.push(ToolTab::PubSub { id, panel });
        let new_idx = self.tools.len() - 1;
        self.active = ActiveTab::Tool(new_idx);
        info!(tool = "pubsub", id, new_idx, "open tool tab + activate");
        self.scroll_tabs_to_end();
        cx.notify();
    }

    fn close_tool_tab(&mut self, idx: usize, window: &mut Window, cx: &mut Context<Self>) {
        if idx >= self.tools.len() {
            return;
        }
        self.tools.remove(idx);
        // 关闭后重新选择 active
        self.active = match self.active {
            ActiveTab::Detail => ActiveTab::Detail,
            ActiveTab::Tool(active_idx) => {
                if active_idx == idx {
                    // 关掉的就是当前激活：优先选前一个 tool，没有则回 Detail
                    if idx == 0 {
                        if self.tools.is_empty() {
                            ActiveTab::Detail
                        } else {
                            ActiveTab::Tool(0)
                        }
                    } else {
                        ActiveTab::Tool(idx - 1)
                    }
                } else if active_idx > idx {
                    // 关掉的在当前激活之前，激活索引前移 1
                    ActiveTab::Tool(active_idx - 1)
                } else {
                    ActiveTab::Tool(active_idx)
                }
            }
        };
        // 关闭后让"新当前 tab"拿到焦点：CLI → 命令输入；PubSub → channel 输入；
        // Detail → 整面板 focus_handle（让 ⌘W 等 action 仍能命中 Session 监听）
        match self.active {
            ActiveTab::Detail => {
                self.detail
                    .update(cx, |p, cx_inner| p.focus_panel(window, cx_inner));
            }
            ActiveTab::Tool(new_idx) => {
                if let Some(tool) = self.tools.get(new_idx) {
                    match tool {
                        ToolTab::Cli { panel, .. } => {
                            panel.update(cx, |p, cx_inner| p.focus_input(window, cx_inner));
                        }
                        ToolTab::PubSub { panel, .. } => {
                            panel.update(cx, |p, cx_inner| p.focus_input(window, cx_inner));
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    fn select_active(&mut self, target: ActiveTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.active == target {
            return;
        }
        self.active = target;
        // 切换 tab 后顺手聚焦：CLI/PubSub 的输入框 / KeyDetail 整面板
        // 让 ⌘W 等 action 能通过焦点链路由到 Session 的 on_action 监听
        match target {
            ActiveTab::Detail => {
                self.detail
                    .update(cx, |p, cx_inner| p.focus_panel(window, cx_inner));
            }
            ActiveTab::Tool(idx) => {
                if let Some(tool) = self.tools.get(idx) {
                    match tool {
                        ToolTab::Cli { panel, .. } => {
                            panel.update(cx, |p, cx_inner| p.focus_input(window, cx_inner));
                        }
                        ToolTab::PubSub { panel, .. } => {
                            panel.update(cx, |p, cx_inner| p.focus_input(window, cx_inner));
                        }
                    }
                }
            }
        }
        cx.notify();
    }

    /// DB 切换：树重连 + 主区清空 + CLI 同步 db（PubSub 不绑 db，不动）
    fn handle_db_change(&mut self, new_db: u8, cx: &mut Context<Self>) {
        if self.db == new_db {
            return;
        }
        info!(db = new_db, "redis session db change");
        self.db = new_db;
        let conf = self.config.clone();
        self.tree
            .update(cx, |t, cx| t.set_connection(Some(conf.clone()), new_db, cx));
        // 主区清空当前 key（旧 key 在原 db，与新 db 无关）
        self.detail
            .update(cx, |p, cx| p.set_connection(Some(conf), new_db, cx));
        // CLI tab 同步新 db
        for tool in &self.tools {
            if let ToolTab::Cli { panel, .. } = tool {
                panel.update(cx, |c, cx| c.set_db(new_db, cx));
            }
        }
        cx.notify();
    }

    /// 弹窗保存后：仅当主区当前 key 与弹窗目标 key 一致时才刷新；
    /// 用户在弹窗期间切了别的 key 则跳过（已经看不到了）
    fn reload_detail_if_key(&mut self, key: &str, cx: &mut Context<Self>) {
        let matches = self
            .detail
            .read(cx)
            .current_key()
            .map(|k| k == key)
            .unwrap_or(false);
        if matches {
            self.detail.update(cx, |p, cx| p.reload_current(cx));
        }
    }

    // ===== 弹窗方法 =====

    fn open_create_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let form = cx.new(|cx| KeyCreateForm::new(svc, config, db, window, cx));
        let tree_for_refresh = self.tree.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |_this: &mut Self, _, ev: &KeyCreateEvent, window, cx| match ev {
                KeyCreateEvent::Created(key) => {
                    info!(?key, "key created via dialog");
                    let new_key = key.clone();
                    window.close_dialog(cx);
                    tree_for_refresh.update(cx, |t, cx| {
                        t.refresh(cx);
                        t.select_key_external(new_key.clone(), cx);
                    });
                }
                KeyCreateEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            dialog
                .title("新建 Key")
                .close_button(true)
                .w(px(640.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn open_ttl_dialog(
        &mut self,
        key: String,
        ttl_ms: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key_for_form = key.clone();
        let form = cx.new(|cx| TtlEditForm::new(svc, config, db, key_for_form, ttl_ms, window, cx));
        let key_for_reload = key.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |this: &mut Self, _, ev: &TtlEditEvent, window, cx| match ev {
                TtlEditEvent::Updated(label) => {
                    info!(?key_for_reload, ?label, "ttl updated");
                    window.close_dialog(cx);
                    this.reload_detail_if_key(&key_for_reload, cx);
                }
                TtlEditEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            dialog
                .title(format!("编辑 TTL · {key}"))
                .close_button(true)
                .w(px(520.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn open_value_dialog(
        &mut self,
        key: String,
        current_value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key_for_form = key.clone();
        let form = cx
            .new(|cx| ValueEditForm::new(svc, config, db, key_for_form, current_value, window, cx));
        let key_for_reload = key.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |this: &mut Self, _, ev: &ValueEditEvent, window, cx| match ev {
                ValueEditEvent::Saved => {
                    info!(?key_for_reload, "value saved");
                    window.close_dialog(cx);
                    this.reload_detail_if_key(&key_for_reload, cx);
                }
                ValueEditEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            dialog
                .title(format!("编辑值 · {key}"))
                .close_button(true)
                .w(px(640.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn open_hash_field_dialog(
        &mut self,
        key: String,
        mode: HashFieldFormMode,
        initial_value: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key_for_form = key.clone();
        let mode_for_form = mode.clone();
        let form = cx.new(|cx| {
            HashFieldForm::new(
                svc,
                config,
                db,
                key_for_form,
                mode_for_form,
                initial_value,
                window,
                cx,
            )
        });
        let key_for_reload = key.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |this: &mut Self, _, ev: &HashFieldFormEvent, window, cx| match ev {
                HashFieldFormEvent::Saved { field } => {
                    info!(?field, "hash field saved");
                    window.close_dialog(cx);
                    this.reload_detail_if_key(&key_for_reload, cx);
                }
                HashFieldFormEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let title = match &mode {
            HashFieldFormMode::Add => format!("新增字段 · {key}"),
            HashFieldFormMode::Edit { field } => format!("编辑字段 · {key} · {field}"),
        };
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            let title = title.clone();
            dialog
                .title(title)
                .close_button(true)
                .w(px(640.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn open_list_element_dialog(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let form = cx.new(|cx| ListElementForm::new(svc, config, db, key.clone(), window, cx));
        let key_for_reload = key.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |this: &mut Self, _, ev: &ListElementFormEvent, window, cx| match ev {
                ListElementFormEvent::Saved => {
                    window.close_dialog(cx);
                    this.reload_detail_if_key(&key_for_reload, cx);
                }
                ListElementFormEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            let title = format!("新增 List 元素 · {key}");
            dialog
                .title(title)
                .close_button(true)
                .w(px(640.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn open_set_element_dialog(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let form = cx.new(|cx| SetElementForm::new(svc, config, db, key.clone(), window, cx));
        let key_for_reload = key.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |this: &mut Self, _, ev: &SetElementFormEvent, window, cx| match ev {
                SetElementFormEvent::Saved => {
                    window.close_dialog(cx);
                    this.reload_detail_if_key(&key_for_reload, cx);
                }
                SetElementFormEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            let title = format!("新增 Set 元素 · {key}");
            dialog
                .title(title)
                .close_button(true)
                .w(px(640.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn open_zset_element_dialog(
        &mut self,
        key: String,
        mode: ZSetElementFormMode,
        initial_score: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let key_for_form = key.clone();
        let mode_for_form = mode.clone();
        let form = cx.new(|cx| {
            ZSetElementForm::new(
                svc,
                config,
                db,
                key_for_form,
                mode_for_form,
                initial_score,
                window,
                cx,
            )
        });
        let key_for_reload = key.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |this: &mut Self, _, ev: &ZSetElementFormEvent, window, cx| match ev {
                ZSetElementFormEvent::Saved => {
                    window.close_dialog(cx);
                    this.reload_detail_if_key(&key_for_reload, cx);
                }
                ZSetElementFormEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let title = match &mode {
            ZSetElementFormMode::Add => format!("新增 ZSet 成员 · {key}"),
            ZSetElementFormMode::EditScore { member } => format!("改 Score · {key} · {member}"),
        };
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            let title = title.clone();
            dialog
                .title(title)
                .close_button(true)
                .w(px(560.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    fn open_stream_entry_dialog(
        &mut self,
        key: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let svc = self.service.clone();
        let config = self.config.clone();
        let db = self.db;
        let form = cx.new(|cx| StreamEntryForm::new(svc, config, db, key.clone(), window, cx));
        let key_for_reload = key.clone();
        let sub = cx.subscribe_in(
            &form,
            window,
            move |this: &mut Self, _, ev: &StreamEntryFormEvent, window, cx| match ev {
                StreamEntryFormEvent::Saved => {
                    window.close_dialog(cx);
                    this.reload_detail_if_key(&key_for_reload, cx);
                }
                StreamEntryFormEvent::Cancelled => window.close_dialog(cx),
            },
        );
        self._subscriptions.push(sub);
        let form_for_dialog = form.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let form = form_for_dialog.clone();
            let title = format!("新增 Stream 条目 · {key}");
            dialog
                .title(title)
                .close_button(true)
                .w(px(640.0))
                .p(px(24.0))
                .content(move |content, _, _| content.child(form.clone()))
        });
    }

    /// 通用「破坏性操作二次确认」弹窗
    fn confirm_delete_op(
        &mut self,
        title: SharedString,
        desc: String,
        on_confirm: Rc<dyn Fn(&mut Window, &mut App) + 'static>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.open_dialog(cx, move |dialog, _w, _app| {
            let desc = desc.clone();
            let on_confirm_ok = on_confirm.clone();
            let cancel_btn = Button::new("del-op-cancel")
                .ghost()
                .small()
                .label("取消")
                .on_click(|_, window, app| window.close_dialog(app));
            let confirm_btn = Button::new("del-op-confirm")
                .danger()
                .small()
                .label("删除")
                .on_click(move |_, window, app| {
                    on_confirm_ok(window, app);
                    window.close_dialog(app);
                });
            dialog
                .title(title.clone())
                .margin_top(px(180.0))
                .content(move |content, _, cx| {
                    let muted_fg = cx.theme().muted_foreground;
                    let desc = desc.clone();
                    content.child(
                        div()
                            .py(px(4.0))
                            .text_sm()
                            .text_color(muted_fg)
                            .child(desc),
                    )
                })
                .footer(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(cancel_btn)
                        .child(confirm_btn),
                )
        });
    }

    fn confirm_dangerous_cmd(
        &mut self,
        raw: String,
        argv: Vec<String>,
        cli_entity: Entity<CliPanel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let raw_for_label = raw.clone();
        window.open_dialog(cx, move |dialog, _w, _app| {
            let raw = raw.clone();
            let argv = argv.clone();
            let cli = cli_entity.clone();
            let desc = format!(
                "命令 `{}` 可能造成数据丢失或服务影响。\n确定执行？",
                raw_for_label
            );
            let cancel_btn = Button::new("danger-cancel")
                .ghost()
                .small()
                .label("取消")
                .on_click(|_, window, app| window.close_dialog(app));
            let confirm_btn = Button::new("danger-confirm")
                .danger()
                .small()
                .label("确定执行")
                .on_click({
                    let cli = cli.clone();
                    let raw = raw.clone();
                    let argv = argv.clone();
                    move |_, window, app| {
                        let raw = raw.clone();
                        let argv = argv.clone();
                        cli.update(app, |c, cx| c.execute_confirmed(raw, argv, window, cx));
                        window.close_dialog(app);
                    }
                });
            dialog
                .title("⚠️ 危险命令确认")
                .margin_top(px(180.0))
                .content(move |content, _, cx| {
                    let muted_fg = cx.theme().muted_foreground;
                    content.child(
                        div()
                            .py(px(4.0))
                            .text_sm()
                            .text_color(muted_fg)
                            .child(desc.clone()),
                    )
                })
                .footer(
                    h_flex()
                        .w_full()
                        .items_center()
                        .justify_end()
                        .gap(px(8.0))
                        .child(cancel_btn)
                        .child(confirm_btn),
                )
        });
    }
}

impl Render for RedisSessionPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let muted_fg = theme.muted_foreground;
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;
        let secondary_bg = theme.secondary;
        let accent = theme.accent;
        let muted_bg = theme.muted;

        // 第一个固定 tab 的标题：当前 key 名 / 占位
        let detail_label: String = self
            .detail
            .read(cx)
            .current_key()
            .map(|k| k.to_string())
            .unwrap_or_else(|| "Key 详情".to_string());

        // 渲染单个 tab（KeyDetail 或某个 tool）；is_active 决定视觉高亮
        // 提取成 helper 让两边样式严格一致
        // CloseTarget：决定点 ✕ 时调哪条路径（清空详情 vs 删 tool）
        #[derive(Clone, Copy)]
        enum CloseTarget {
            Detail,
            Tool(usize),
        }
        struct TabSpec {
            id_select: SharedString,
            id_close: Option<SharedString>,
            glyph: &'static str,
            label: String,
            is_active: bool,
            on_select: ActiveTab,
            on_close: Option<CloseTarget>,
        }

        // KeyDetail tab：仅在已加载具体 key 时可关（点 ✕ 清空回到默认占位态）；
        // 默认空态（"Key 详情"）则不可关，作为永远可见的入口
        let detail_has_key = self.detail.read(cx).current_key().is_some();
        let mut tab_specs: Vec<TabSpec> = Vec::with_capacity(1 + self.tools.len());
        tab_specs.push(TabSpec {
            id_select: SharedString::from("ws-tab-detail"),
            id_close: if detail_has_key {
                Some(SharedString::from("ws-tab-detail-close"))
            } else {
                None
            },
            glyph: "🔑",
            label: detail_label,
            is_active: self.active == ActiveTab::Detail,
            on_select: ActiveTab::Detail,
            on_close: if detail_has_key {
                Some(CloseTarget::Detail)
            } else {
                None
            },
        });
        for (i, tool) in self.tools.iter().enumerate() {
            tab_specs.push(TabSpec {
                id_select: SharedString::from(format!("ws-tab-tool-{i}")),
                id_close: Some(SharedString::from(format!("ws-tab-tool-close-{i}"))),
                glyph: tool.icon_glyph(),
                label: tool.label(),
                is_active: self.active == ActiveTab::Tool(i),
                on_select: ActiveTab::Tool(i),
                on_close: Some(CloseTarget::Tool(i)),
            });
        }

        // 顶部 tab bar
        let mut tab_bar = h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .bg(secondary_bg);

        let mut tabs_strip = h_flex()
            .id("ws-tabs-scroll")
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.workspace_scroll);

        // active 与 inactive 视觉区分加强（亮主题下 bg/secondary_bg 颜色相近）：
        // - 顶部 accent 横条加粗到 3px（替代原 2px）
        // - active 给一层 15% 透明度的 accent 染色盖在 bg 上（对齐 MySQL QueryPanel 同款 a=0.15）
        let mut active_tab_tint = accent;
        active_tab_tint.a = 0.15;

        for spec in tab_specs {
            let label_color = if spec.is_active { fg } else { muted_fg };
            let glyph_color = if spec.is_active { accent } else { muted_fg };
            let top_bar_color = if spec.is_active {
                accent
            } else {
                gpui::transparent_black()
            };
            let tab_bg = if spec.is_active {
                active_tab_tint
            } else {
                secondary_bg
            };
            let target = spec.on_select;

            let mut body = h_flex()
                .id(spec.id_select)
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(7.0))
                .cursor_pointer()
                .child(div().text_xs().text_color(glyph_color).child(spec.glyph))
                .child(
                    div()
                        .text_xs()
                        .text_color(label_color)
                        .when(spec.is_active, |this| {
                            this.font_weight(gpui::FontWeight::SEMIBOLD)
                        })
                        .max_w(px(220.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(spec.label),
                );
            // KeyDetail 已加载 key 时显示 ✕（点击清空详情回到默认空态）；
            // 默认空态没 ✕；tool tab 始终带 ✕
            if let (Some(close_id), Some(close_target)) = (spec.id_close, spec.on_close) {
                body = body.child(
                    Button::new(close_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                            match close_target {
                                CloseTarget::Tool(idx) => this.close_tool_tab(idx, window, cx),
                                CloseTarget::Detail => {
                                    // 清空详情后把焦点拿回到 Detail 面板，
                                    // 让 ⌘W / 后续键盘动作仍能正确路由
                                    this.detail.update(cx, |p, cx_inner| {
                                        p.clear_key(cx_inner);
                                        p.focus_panel(window, cx_inner);
                                    });
                                    cx.notify();
                                }
                            }
                        })),
                );
            }
            body = body.on_click(cx.listener(move |this, _: &gpui::ClickEvent, window, cx| {
                this.select_active(target, window, cx);
            }));

            // tab 用 v_flex 包裹：顶部 3px accent 横条（active）/ 透明（inactive）+ body
            let mut tab = v_flex()
                .flex_none()
                .border_r_1()
                .border_color(border)
                .bg(tab_bg)
                .child(div().h(px(3.0)).w_full().bg(top_bar_color))
                .child(body);
            if !spec.is_active {
                tab = tab.hover(move |this| this.bg(muted_bg));
            }
            tabs_strip = tabs_strip.child(tab);
        }
        tab_bar = tab_bar.child(tabs_strip);

        // 右上角悬浮工具栏：CLI / Pub/Sub 按钮，点击各加一个 tab
        // 与左侧 tab 同结构（3px 横条 + py(7) body）保高度齐
        tab_bar = tab_bar.child(
            v_flex()
                .flex_none()
                .child(div().h(px(3.0)).w_full().bg(gpui::transparent_black()))
                .child(
                    h_flex()
                        .items_center()
                        .gap(px(2.0))
                        .px(px(6.0))
                        .py(px(7.0))
                        .child(
                            Button::new("ws-add-cli")
                                .ghost()
                                .small()
                                .icon(IconName::SquareTerminal)
                                .tooltip("新建 CLI 命令面板")
                                .on_click(cx.listener(
                                    |this, _: &gpui::ClickEvent, window, cx| {
                                        this.open_cli_tab(window, cx)
                                    },
                                )),
                        )
                        .child(
                            Button::new("ws-add-pubsub")
                                .ghost()
                                .small()
                                .icon(ramag_ui::icons::radio_tower())
                                .tooltip("新建 Pub/Sub 订阅面板")
                                .on_click(cx.listener(
                                    |this, _: &gpui::ClickEvent, window, cx| {
                                        this.open_pubsub_tab(window, cx)
                                    },
                                )),
                        ),
                ),
        );

        // 当前激活 tab 的内容区
        let content: gpui::AnyElement = match self.active {
            ActiveTab::Detail => self.detail.clone().into_any_element(),
            ActiveTab::Tool(i) => match self.tools.get(i) {
                Some(t) => div().size_full().child(t.to_view()).into_any_element(),
                None => div().size_full().into_any_element(),
            },
        };

        // ⌘W 监听放在 workspace 内部 v_flex（最靠近 tab_bar 与 content，
        // 焦点链上的天然父节点；放在最外 v_flex 时实测有些场景 action 不冒泡到外层）
        let workspace = v_flex()
            .size_full()
            .key_context("RedisSession")
            // ⌘W 三态：
            // - 当前是 tool tab → 关闭它
            // - 当前是 KeyDetail 且加载了具体 key → 清空 key（等价于点 ✕，回到默认占位态）
            // - 当前是 KeyDetail 且无 key（默认空态）→ 冒泡到全局 fallback 关窗
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let active = this.active;
                info!(?active, "redis session: CloseTab action received");
                match active {
                    ActiveTab::Tool(idx) => {
                        this.close_tool_tab(idx, window, cx);
                    }
                    ActiveTab::Detail => {
                        let has_key = this.detail.read(cx).current_key().is_some();
                        if has_key {
                            this.detail.update(cx, |p, cx_inner| {
                                p.clear_key(cx_inner);
                                p.focus_panel(window, cx_inner);
                            });
                            cx.notify();
                        } else {
                            cx.propagate();
                        }
                    }
                }
            }))
            .child(tab_bar)
            .child(div().flex_1().min_h_0().child(content));

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(
                h_resizable("redis-session-resize")
                    .with_state(&self.resize_state)
                    .child(
                        resizable_panel()
                            .size(px(TREE_WIDTH_INITIAL))
                            .size_range(px(TREE_WIDTH_MIN)..px(TREE_WIDTH_MAX))
                            .child(
                                div()
                                    .size_full()
                                    .border_r_1()
                                    .border_color(border)
                                    .child(self.tree.clone()),
                            ),
                    )
                    .child(
                        resizable_panel().child(div().size_full().min_w_0().child(workspace)),
                    ),
            )
    }
}

/// 截断弹窗中要展示的字符串到指定字符数（按 char 计，避免破坏 utf-8 边界）
/// 超长加省略号，便于在「删除 X」对话框里清晰展示目标
fn truncate_for_dialog(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let prefix: String = s.chars().take(max_chars).collect();
    format!("{prefix}…")
}
