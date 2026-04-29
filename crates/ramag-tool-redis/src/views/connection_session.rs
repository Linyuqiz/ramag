//! Redis 连接会话面板（dbclient 装载，作为 Redis 连接的会话视图）
//!
//! 布局：左侧 Key 树（含 DB 切换 / 搜索 / 新建），右侧 KeyDetail 主区。
//! 点 key 树某项 → 主区 load_key；DB 切换 → 主区清空。无 tab、无 CLI、无 Pub/Sub。

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Context, Entity, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Window, div, prelude::*, px,
};
use gpui_component::{
    ActiveTheme, Sizable as _, WindowExt as _,
    button::{Button, ButtonVariants as _},
    h_flex,
    resizable::{ResizableState, h_resizable, resizable_panel},
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use ramag_ui::CloseTab;
use tracing::info;

use crate::views::hash_field_form::{HashFieldForm, HashFieldFormEvent, HashFieldFormMode};
use crate::views::key_create::{KeyCreateEvent, KeyCreateForm};
use crate::views::key_detail::{KeyDetailEvent, KeyDetailPanel};
use crate::views::key_tree::{KeyTreeEvent, KeyTreePanel};
use crate::views::list_element_form::{ListElementForm, ListElementFormEvent};
use crate::views::set_element_form::{SetElementForm, SetElementFormEvent};
use crate::views::stream_entry_form::{StreamEntryForm, StreamEntryFormEvent};
use crate::views::ttl_edit::{TtlEditEvent, TtlEditForm};
use crate::views::value_edit::{ValueEditEvent, ValueEditForm};
use crate::views::zset_element_form::{ZSetElementForm, ZSetElementFormEvent, ZSetElementFormMode};

const TREE_WIDTH_INITIAL: f32 = 320.0;
const TREE_WIDTH_MIN: f32 = 200.0;
const TREE_WIDTH_MAX: f32 = 600.0;

/// 二次确认弹窗的回调签名（避免 `Rc<dyn Fn(&mut Window, &mut App)>` 长类型重复出现）
type ConfirmCallback = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

pub struct RedisSessionPanel {
    service: Arc<RedisService>,
    config: ConnectionConfig,
    db: u8,
    tree: Entity<KeyTreePanel>,
    detail: Entity<KeyDetailPanel>,
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

    /// DB 切换：树重连 + 主区清空当前 key（旧 key 在原 db，无关）
    fn handle_db_change(&mut self, new_db: u8, cx: &mut Context<Self>) {
        if self.db == new_db {
            return;
        }
        info!(db = new_db, "redis session db change");
        self.db = new_db;
        let conf = self.config.clone();
        self.tree
            .update(cx, |t, cx| t.set_connection(Some(conf.clone()), new_db, cx));
        self.detail
            .update(cx, |p, cx| p.set_connection(Some(conf), new_db, cx));
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
        on_confirm: ConfirmCallback,
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
                    content.child(div().py(px(4.0)).text_sm().text_color(muted_fg).child(desc))
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
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;

        // ⌘W：KeyDetail 加载了 key 时清空回到默认空态；空态时冒泡到全局 fallback 关窗
        let workspace = v_flex()
            .size_full()
            .key_context("RedisSession")
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                info!("redis session: CloseTab action received");
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
            }))
            .child(self.detail.clone());

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
                                .border_color(border)
                                .child(self.tree.clone()),
                        ),
                )
                .child(resizable_panel().child(div().size_full().min_w_0().child(workspace))),
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
