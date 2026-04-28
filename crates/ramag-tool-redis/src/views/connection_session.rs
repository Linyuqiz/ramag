//! Redis 连接会话面板（dbclient 装载，作为 Redis 连接的会话视图）
//!
//! 布局（仿 MySQL：左树 + 右多 tab 工作区）：
//! ```text
//! ┌──────────────────┬──────────────────────────────────────┐
//! │ DB ▾ 0 共7 keys  │ [user:1 ✕][user:2 ✕][CLI ✕] | [+CLI][+监控][+Pub/Sub] │
//! │ ─────────────    ├──────────────────────────────────────┤
//! │ 🔍 [+][▼][▶][↻]  │                                      │
//! │ user/            │  当前激活 tab 内容                     │
//! │  ├ 1001          │  （KeyDetail / Cli / PubSub / Monitor）│
//! │  └ 1002          │                                      │
//! │ session/         │                                      │
//! └──────────────────┴──────────────────────────────────────┘
//! ```

use std::sync::Arc;

use gpui::{
    AnyView, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled,
    Subscription, Window, div, prelude::*, px,
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
use tracing::info;

use crate::views::cli_panel::{CliEvent, CliPanel};
use crate::views::hash_field_form::{HashFieldForm, HashFieldFormEvent, HashFieldFormMode};
use crate::views::key_create::{KeyCreateEvent, KeyCreateForm};
use crate::views::key_detail::{KeyDetailEvent, KeyDetailPanel};
use crate::views::key_tree::{KeyTreeEvent, KeyTreePanel};
use crate::views::list_element_form::{ListElementForm, ListElementFormEvent};
use crate::views::monitor_panel::MonitorPanel;
use crate::views::pubsub_panel::PubSubPanel;
use crate::views::set_element_form::{SetElementForm, SetElementFormEvent};
use crate::views::stream_entry_form::{StreamEntryForm, StreamEntryFormEvent};
use crate::views::ttl_edit::{TtlEditEvent, TtlEditForm};
use crate::views::value_edit::{ValueEditEvent, ValueEditForm};
use crate::views::zset_element_form::{ZSetElementForm, ZSetElementFormEvent, ZSetElementFormMode};

const TREE_WIDTH_INITIAL: f32 = 320.0;
const TREE_WIDTH_MIN: f32 = 200.0;
const TREE_WIDTH_MAX: f32 = 600.0;

/// 右侧工作区的动态 tab（仿 MySQL 多 SQL tab）
enum WorkspaceTab {
    KeyDetail {
        key: String,
        panel: Entity<KeyDetailPanel>,
    },
    Cli {
        id: u64,
        panel: Entity<CliPanel>,
    },
    PubSub {
        id: u64,
        panel: Entity<PubSubPanel>,
    },
    Monitor {
        id: u64,
        panel: Entity<MonitorPanel>,
    },
}

impl WorkspaceTab {
    fn label(&self) -> String {
        match self {
            WorkspaceTab::KeyDetail { key, .. } => key.clone(),
            WorkspaceTab::Cli { id, .. } => format!("CLI {id}"),
            WorkspaceTab::PubSub { id, .. } => format!("Pub/Sub {id}"),
            WorkspaceTab::Monitor { id, .. } => format!("监控 {id}"),
        }
    }
    fn icon_glyph(&self) -> &'static str {
        match self {
            WorkspaceTab::KeyDetail { .. } => "🔑",
            WorkspaceTab::Cli { .. } => "▶",
            WorkspaceTab::PubSub { .. } => "📡",
            WorkspaceTab::Monitor { .. } => "📊",
        }
    }
    fn to_view(&self) -> AnyView {
        match self {
            WorkspaceTab::KeyDetail { panel, .. } => panel.clone().into(),
            WorkspaceTab::Cli { panel, .. } => panel.clone().into(),
            WorkspaceTab::PubSub { panel, .. } => panel.clone().into(),
            WorkspaceTab::Monitor { panel, .. } => panel.clone().into(),
        }
    }
}

pub struct RedisSessionPanel {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    tree: Entity<KeyTreePanel>,
    workspace_tabs: Vec<WorkspaceTab>,
    active_tab: Option<usize>,
    /// 自增 id 用于 CLI/PubSub/Monitor 的唯一标识（KeyDetail 用 key 名识别）
    next_tab_id: u64,
    resize_state: Entity<ResizableState>,
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

        // 树事件：选中加载（开 KeyDetail tab）/ 请求新建（弹 dialog）/ DB 切换
        let subs = vec![cx.subscribe_in(
            &tree,
            window,
            move |this: &mut Self, _, e: &KeyTreeEvent, window, cx| match e {
                KeyTreeEvent::Selected(key) => {
                    this.open_key_tab(key.clone(), window, cx);
                }
                KeyTreeEvent::RequestCreate => {
                    this.open_create_dialog(window, cx);
                }
                KeyTreeEvent::DbSelected(db) => {
                    this.handle_db_change(*db, cx);
                }
            },
        )];

        let resize_state = cx.new(|_| ResizableState::default());

        Self {
            service,
            config,
            db: initial_db,
            tree,
            workspace_tabs: Vec::new(),
            active_tab: None,
            next_tab_id: 1,
            resize_state,
            _subscriptions: subs,
        }
    }

    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    pub fn title(&self) -> &str {
        &self.config.name
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        id
    }

    /// 打开（或激活）某 key 的 KeyDetail tab
    fn open_key_tab(&mut self, key: String, window: &mut Window, cx: &mut Context<Self>) {
        // 已有该 key tab → 直接激活
        if let Some(idx) = self
            .workspace_tabs
            .iter()
            .position(|t| matches!(t, WorkspaceTab::KeyDetail { key: k, .. } if k == &key))
        {
            self.active_tab = Some(idx);
            cx.notify();
            return;
        }

        let svc = self.service.clone();
        let panel = cx.new(|_| KeyDetailPanel::new(svc));
        let conf = self.config.clone();
        let db = self.db;
        let key_for_load = key.clone();
        panel.update(cx, |p, cx| {
            p.set_connection(Some(conf), db, cx);
            p.load_key(key_for_load, cx);
        });

        // 订阅 KeyDetail 事件（删除 / 编辑请求）
        let tree_for_refresh = self.tree.clone();
        let panel_for_close = panel.clone();
        let sub = cx.subscribe_in(
            &panel,
            window,
            move |this: &mut Self, _, e: &KeyDetailEvent, window, cx| match e {
                KeyDetailEvent::Deleted(_) => {
                    tree_for_refresh.update(cx, |t, cx| t.refresh(cx));
                    // 关掉对应的 tab
                    let target = panel_for_close.clone();
                    if let Some(idx) = this.workspace_tabs.iter().position(
                        |t| matches!(t, WorkspaceTab::KeyDetail { panel: p, .. } if p == &target),
                    ) {
                        this.close_tab(idx, cx);
                    }
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
            },
        );
        self._subscriptions.push(sub);

        self.workspace_tabs
            .push(WorkspaceTab::KeyDetail { key, panel });
        self.active_tab = Some(self.workspace_tabs.len() - 1);
        cx.notify();
    }

    fn open_cli_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.alloc_id();
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
        self.workspace_tabs.push(WorkspaceTab::Cli { id, panel });
        self.active_tab = Some(self.workspace_tabs.len() - 1);
        cx.notify();
    }

    fn open_pubsub_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let id = self.alloc_id();
        let svc = self.service.clone();
        let cfg = self.config.clone();
        let panel = cx.new(|cx| PubSubPanel::new(svc, cfg, window, cx));
        self.workspace_tabs.push(WorkspaceTab::PubSub { id, panel });
        self.active_tab = Some(self.workspace_tabs.len() - 1);
        cx.notify();
    }

    fn open_monitor_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let id = self.alloc_id();
        let svc = self.service.clone();
        let cfg = self.config.clone();
        let panel = cx.new(|_| MonitorPanel::new(svc, cfg));
        panel.update(cx, |m, cx| {
            m.refresh(cx);
            m.start_metrics_sampler(cx);
        });
        self.workspace_tabs
            .push(WorkspaceTab::Monitor { id, panel });
        self.active_tab = Some(self.workspace_tabs.len() - 1);
        cx.notify();
    }

    fn close_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx >= self.workspace_tabs.len() {
            return;
        }
        self.workspace_tabs.remove(idx);
        if self.workspace_tabs.is_empty() {
            self.active_tab = None;
        } else if let Some(active) = self.active_tab {
            if active == idx {
                self.active_tab = Some(idx.saturating_sub(1).min(self.workspace_tabs.len() - 1));
            } else if active > idx {
                self.active_tab = Some(active - 1);
            }
        }
        cx.notify();
    }

    fn select_tab(&mut self, idx: usize, cx: &mut Context<Self>) {
        if idx < self.workspace_tabs.len() {
            self.active_tab = Some(idx);
            cx.notify();
        }
    }

    /// DB 切换：通知树重新加载；关闭所有 KeyDetail tab；CLI 同步 db；监控/PubSub 不动
    fn handle_db_change(&mut self, new_db: u8, cx: &mut Context<Self>) {
        if self.db == new_db {
            return;
        }
        info!(db = new_db, "redis session db change");
        self.db = new_db;
        let conf = self.config.clone();
        self.tree
            .update(cx, |t, cx| t.set_connection(Some(conf), new_db, cx));

        // 关闭所有 KeyDetail tab（key 在原 db，不在新 db 内）
        let mut to_remove: Vec<usize> = self
            .workspace_tabs
            .iter()
            .enumerate()
            .filter(|(_, t)| matches!(t, WorkspaceTab::KeyDetail { .. }))
            .map(|(i, _)| i)
            .collect();
        to_remove.sort_unstable_by(|a, b| b.cmp(a));
        for idx in to_remove {
            self.close_tab(idx, cx);
        }

        // CLI tab 同步 db
        for t in &self.workspace_tabs {
            if let WorkspaceTab::Cli { panel, .. } = t {
                panel.update(cx, |c, cx| c.set_db(new_db, cx));
            }
        }
        cx.notify();
    }

    /// 在所有 KeyDetail tab 中找匹配 key 的 panel，触发重载
    fn reload_key_tab(&mut self, key: &str, cx: &mut Context<Self>) {
        for t in &self.workspace_tabs {
            if let WorkspaceTab::KeyDetail { key: k, panel } = t
                && k == key
            {
                panel.update(cx, |d, cx| d.reload_current(cx));
            }
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
                    this.reload_key_tab(&key_for_reload, cx);
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
                    this.reload_key_tab(&key_for_reload, cx);
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
                    this.reload_key_tab(&key_for_reload, cx);
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
                    this.reload_key_tab(&key_for_reload, cx);
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
                    this.reload_key_tab(&key_for_reload, cx);
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
                    this.reload_key_tab(&key_for_reload, cx);
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
                    this.reload_key_tab(&key_for_reload, cx);
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
        let (muted_fg, fg, border, bg, secondary_bg, accent, muted_bg) = {
            let theme = cx.theme();
            (
                theme.muted_foreground,
                theme.foreground,
                theme.border,
                theme.background,
                theme.secondary,
                theme.accent,
                theme.muted,
            )
        };
        let active = self.active_tab;
        let tabs_meta: Vec<(usize, String, &'static str, bool)> = self
            .workspace_tabs
            .iter()
            .enumerate()
            .map(|(i, t)| (i, t.label(), t.icon_glyph(), Some(i) == active))
            .collect();

        // 左侧：Key 树（已含 DB picker + 总数）
        let tree_view = self.tree.clone();

        // 右侧顶部：工作区 tab bar
        let mut tab_bar = h_flex()
            .w_full()
            .flex_none()
            .border_b_1()
            .border_color(border)
            .bg(secondary_bg);

        // 已开 tabs（横向可滚动）
        let mut tabs_strip = h_flex()
            .id("ws-tabs-scroll")
            .flex_1()
            .min_w_0()
            .overflow_x_scroll();
        for (idx, label, glyph, is_active) in tabs_meta {
            let tab_id = SharedString::from(format!("ws-tab-{idx}"));
            let close_id = SharedString::from(format!("ws-tab-close-{idx}"));
            // active tab 视觉（仿浏览器 tab）：
            // - 顶部 2px accent 横条（child div 替代 border_t_color）
            // - bg 与下方主区同色（"无缝"接到内容）
            // - 文字加粗 + glyph 用 accent 色
            // inactive：bg 与 tab_bar 同 secondary_bg + hover muted
            let label_color = if is_active { fg } else { muted_fg };
            let glyph_color = if is_active { accent } else { muted_fg };
            let top_bar_color = if is_active { accent } else { gpui::transparent_black() };
            let tab_bg = if is_active { bg } else { secondary_bg };

            let body = h_flex()
                .id(tab_id)
                .flex_none()
                .items_center()
                .gap(px(6.0))
                .px(px(10.0))
                .py(px(7.0))
                .cursor_pointer()
                .child(div().text_xs().text_color(glyph_color).child(glyph))
                .child(
                    div()
                        .text_xs()
                        .text_color(label_color)
                        .when(is_active, |this| this.font_weight(gpui::FontWeight::SEMIBOLD))
                        .max_w(px(180.0))
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(label),
                )
                .child(
                    Button::new(close_id)
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                            this.close_tab(idx, cx);
                        })),
                )
                .on_click(cx.listener(move |this, _: &gpui::ClickEvent, _, cx| {
                    this.select_tab(idx, cx);
                }));

            // tab 用 v_flex 包裹：顶部 2px 横条 + 主 body
            let mut tab = v_flex()
                .flex_none()
                .border_r_1()
                .border_color(border)
                .bg(tab_bg)
                .child(div().h(px(2.0)).w_full().bg(top_bar_color))
                .child(body);
            if !is_active {
                tab = tab.hover(move |this| this.bg(muted_bg));
            }
            tabs_strip = tabs_strip.child(tab);
        }
        tab_bar = tab_bar.child(tabs_strip);

        // 右上角的 [+ X] 三个按钮
        tab_bar = tab_bar.child(
            h_flex()
                .flex_none()
                .gap(px(4.0))
                .px(px(8.0))
                .child(
                    Button::new("ws-add-cli")
                        .ghost()
                        .xsmall()
                        .label("+ CLI")
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                            this.open_cli_tab(window, cx)
                        })),
                )
                .child(
                    Button::new("ws-add-pubsub")
                        .ghost()
                        .xsmall()
                        .label("+ Pub/Sub")
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                            this.open_pubsub_tab(window, cx)
                        })),
                )
                .child(
                    Button::new("ws-add-monitor")
                        .ghost()
                        .xsmall()
                        .label("+ 监控")
                        .on_click(cx.listener(|this, _: &gpui::ClickEvent, window, cx| {
                            this.open_monitor_tab(window, cx)
                        })),
                ),
        );

        // 工作区内容
        let content: gpui::AnyElement = match active.and_then(|i| self.workspace_tabs.get(i)) {
            Some(t) => div().size_full().child(t.to_view()).into_any_element(),
            None => v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap(px(8.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(muted_fg)
                        .child("双击左侧 Key 浏览，或点击右上角按钮打开 CLI / Pub/Sub / 监控"),
                )
                .into_any_element(),
        };

        let workspace = v_flex()
            .size_full()
            .child(tab_bar)
            .child(div().flex_1().min_h_0().child(content));

        let inner_border = border;
        v_flex().size_full().bg(bg).text_color(fg).child(
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
                                .border_color(inner_border)
                                .child(tree_view),
                        ),
                )
                .child(resizable_panel().child(div().size_full().min_w_0().child(workspace))),
        )
    }
}
