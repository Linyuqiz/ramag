//! Redis 会话：左 Key 树（DB 切换 / 搜索 / 新建），右 KeyDetail。点 key→load_key，切 DB→清主区

mod dialogs;

use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription, Window, div,
    prelude::*, px,
};
use gpui_component::{
    ActiveTheme,
    resizable::{ResizableState, h_resizable, resizable_panel},
    v_flex,
};
use ramag_app::RedisService;
use ramag_domain::entities::ConnectionConfig;
use ramag_ui::CloseTab;
use tracing::info;

use crate::views::hash_field_form::HashFieldFormMode;
use crate::views::key_detail::{KeyDetailEvent, KeyDetailPanel};
use crate::views::key_tree::{DeletedScope, KeyTreeEvent, KeyTreePanel};
use crate::views::zset_element_form::ZSetElementFormMode;

use dialogs::truncate_for_dialog;

const TREE_WIDTH_INITIAL: f32 = 280.0;
const TREE_WIDTH_MIN: f32 = 180.0;
const TREE_WIDTH_MAX: f32 = 600.0;

pub struct RedisSessionPanel {
    pub(super) service: Arc<RedisService>,
    pub(super) config: ConnectionConfig,
    pub(super) db: u8,
    pub(super) tree: Entity<KeyTreePanel>,
    pub(super) detail: Entity<KeyDetailPanel>,
    resize_state: Entity<ResizableState>,
    subscriptions: Vec<Subscription>,
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

        // 主区 KeyDetail：单实例 + 初始即聚焦，让 cmd-w 等 action 走焦点链
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
                // 树侧右键删除完成：若详情面板正在展示受影响的 key，清空之
                KeyTreeEvent::KeysDeleted(scope) => {
                    let current = this.detail.read(cx).current_key().map(str::to_string);
                    let invalidated = match (scope, current.as_deref()) {
                        (_, None) => false,
                        (DeletedScope::Key(k), Some(c)) => k == c,
                        (DeletedScope::Prefix(p), Some(c)) => c.starts_with(&format!("{p}:")),
                        (DeletedScope::Db, Some(_)) => true,
                    };
                    if invalidated {
                        this.detail.update(cx, |p, cx| p.clear_key(cx));
                    }
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
            subscriptions: subs,
        }
    }

    pub fn config(&self) -> &ConnectionConfig {
        &self.config
    }

    pub fn title(&self) -> &str {
        &self.config.name
    }

    /// 让 dialogs.rs 内的方法把 subscription 推进 Vec（保持字段私有）
    pub(super) fn push_subscription(&mut self, sub: Subscription) {
        self.subscriptions.push(sub);
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
}

impl Render for RedisSessionPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fg = theme.foreground;
        let border = theme.border;
        let bg = theme.background;

        // CloseTab：KeyDetail 有 key 时清回空态；空态时冒泡到全局 fallback 关窗
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
