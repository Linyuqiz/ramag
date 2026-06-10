//! 树节点破坏性操作：删除 key / 删除前缀下全部 key / 清空当前 DB。
//! 右键菜单 → open_confirm 二次确认 → 异步执行 → 刷新 + toast；
//! 删除完成 emit KeysDeleted，上层据此清理详情面板

use gpui::{Context, Entity};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use gpui_component::notification::Notification;
use ramag_app::RedisService;
use ramag_domain::entities::{ConnectionConfig, RedisValue};
use ramag_domain::error::Result;
use ramag_ui::{open_confirm, open_prompt};

use super::{DeletedScope, KeyTreeEvent, KeyTreePanel};

/// 单轮 SCAN 收集上限：删完一轮再扫下一轮，内存有界
const SCAN_BATCH: usize = 10_000;
/// 单条 DEL 携带的 key 数上限：避免超长命令阻塞服务端
const DEL_CHUNK: usize = 500;

// ===== 右键菜单构造（render.rs 调用） =====

/// key / 命名空间行右键菜单。两种身份兼具的节点（`user` 同时是 key 和前缀）两项都给
pub(super) fn node_context_menu(
    menu: PopupMenu,
    entity: Entity<KeyTreePanel>,
    full_path: String,
    is_leaf: bool,
    is_namespace: bool,
    db: u8,
) -> PopupMenu {
    let mut menu = menu;
    if is_leaf {
        let (key, ent) = (full_path.clone(), entity.clone());
        menu = menu.item(
            PopupMenuItem::new("重命名 key…").on_click(move |_, window, app| {
                let (key, ent) = (key.clone(), ent.clone());
                open_prompt(
                    "重命名 Key",
                    format!("输入「{}」的新名称", truncate_label(&key, 60)),
                    &key.clone(),
                    "重命名",
                    move |new_name, _, app| {
                        ent.update(app, |this, cx| this.rename_key_op(key, new_name, cx));
                    },
                    window,
                    app,
                );
            }),
        );
        let (key, ent) = (full_path.clone(), entity.clone());
        menu = menu.item(
            PopupMenuItem::new("删除 key…").on_click(move |_, window, app| {
                let (key, ent) = (key.clone(), ent.clone());
                open_confirm(
                    "删除 Key",
                    format!(
                        "将永久删除 key「{}」，此操作不可恢复。",
                        truncate_label(&key, 60)
                    ),
                    "删除",
                    true,
                    move |_, app| {
                        ent.update(app, |this, cx| this.delete_key_op(key, cx));
                    },
                    window,
                    app,
                );
            }),
        );
    }
    if is_namespace {
        let (prefix, ent) = (full_path.clone(), entity.clone());
        menu = menu.item(
            PopupMenuItem::new("删除该前缀下全部 key…").on_click(move |_, window, app| {
                let (prefix, ent) = (prefix.clone(), ent.clone());
                open_confirm(
                    "删除前缀下全部 Key",
                    format!(
                        "将删除匹配「{}:*」的全部 key（按服务端实际扫描，含未加载部分），此操作不可恢复。",
                        truncate_label(&prefix, 60)
                    ),
                    "删除",
                    true,
                    move |_, app| {
                        ent.update(app, |this, cx| this.delete_prefix_op(prefix, cx));
                    },
                    window,
                    app,
                );
            }),
        );
    }
    let ent = entity;
    menu.separator()
        .item(
            PopupMenuItem::new(format!("清空当前 DB {db}…")).on_click(move |_, window, app| {
                let ent = ent.clone();
                open_confirm(
                    "清空当前 DB",
                    format!("将删除 DB {db} 的全部 key（FLUSHDB），此操作不可恢复。"),
                    "清空",
                    true,
                    move |_, app| {
                        ent.update(app, |this, cx| this.flush_db_op(cx));
                    },
                    window,
                    app,
                );
            }),
        )
}

// ===== 删除 / 重命名执行 =====

impl KeyTreePanel {
    /// RENAMENX：目标 key 已存在则返回 0 不覆盖，避免静默吞掉别人的数据
    pub(super) fn rename_key_op(&mut self, old: String, new: String, cx: &mut Context<Self>) {
        if new == old {
            return;
        }
        let Some(config) = self.config.clone() else {
            return;
        };
        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let argv = vec!["RENAMENX".to_string(), old.clone(), new.clone()];
            let r = svc.execute_command(&config, db, argv).await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(RedisValue::Int(1)) => {
                        if let Some(k) = this.keys.iter_mut().find(|k| k.key == old) {
                            k.key = new.clone();
                        }
                        this.rebuild_tree();
                        if this.selected.as_deref() == Some(old.as_str()) {
                            this.selected = Some(new.clone());
                            // 让详情面板切到新 key
                            cx.emit(KeyTreeEvent::Selected(new.clone()));
                        }
                        this.pending_notification = Some(
                            Notification::success(format!(
                                "已重命名为 {}",
                                truncate_label(&new, 60)
                            ))
                            .autohide(true),
                        );
                    }
                    Ok(RedisValue::Int(_)) => {
                        this.pending_notification = Some(
                            Notification::error("目标 key 已存在，未执行重命名").autohide(true),
                        );
                    }
                    Ok(other) => {
                        tracing::error!(?other, "renamenx unexpected reply");
                        this.pending_notification =
                            Some(Notification::error("重命名失败：服务端应答异常").autohide(true));
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "rename key failed");
                        this.pending_notification =
                            Some(Notification::error(format!("重命名失败：{e}")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn delete_key_op(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let r = svc.delete_key(&config, db, &key).await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(_) => {
                        // 本地移除即可，无需整库重扫
                        this.keys.retain(|k| k.key != key);
                        this.rebuild_tree();
                        if this.selected.as_deref() == Some(key.as_str()) {
                            this.selected = None;
                        }
                        this.pending_notification = Some(
                            Notification::success(format!(
                                "已删除 key {}",
                                truncate_label(&key, 60)
                            ))
                            .autohide(true),
                        );
                        cx.emit(KeyTreeEvent::KeysDeleted(DeletedScope::Key(key.clone())));
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "delete key from tree failed");
                        this.pending_notification =
                            Some(Notification::error(format!("删除失败：{e}")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn delete_prefix_op(&mut self, prefix: String, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let svc = self.service.clone();
        let db = self.db;
        let pattern = format!("{}:*", escape_glob(&prefix));
        cx.spawn(async move |this, cx| {
            let result = delete_by_pattern(&svc, &config, db, &pattern).await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(n) => {
                        let sub_prefix = format!("{prefix}:");
                        if this
                            .selected
                            .as_deref()
                            .is_some_and(|s| s.starts_with(&sub_prefix))
                        {
                            this.selected = None;
                        }
                        this.pending_notification = Some(
                            Notification::success(format!(
                                "已删除前缀 {} 下 {n} 个 key",
                                truncate_label(&prefix, 60)
                            ))
                            .autohide(true),
                        );
                        cx.emit(KeyTreeEvent::KeysDeleted(DeletedScope::Prefix(
                            prefix.clone(),
                        )));
                        this.refresh(cx);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, pattern = %pattern, "delete by prefix failed");
                        this.pending_notification =
                            Some(Notification::error(format!("删除失败：{e}")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn flush_db_op(&mut self, cx: &mut Context<Self>) {
        let Some(config) = self.config.clone() else {
            return;
        };
        let svc = self.service.clone();
        let db = self.db;
        cx.spawn(async move |this, cx| {
            let r = svc
                .execute_command(&config, db, vec!["FLUSHDB".to_string()])
                .await;
            let _ = this.update(cx, |this, cx| {
                match r {
                    Ok(_) => {
                        this.selected = None;
                        this.pending_notification =
                            Some(Notification::success(format!("已清空 DB {db}")).autohide(true));
                        cx.emit(KeyTreeEvent::KeysDeleted(DeletedScope::Db));
                        this.refresh(cx);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, db, "flushdb failed");
                        this.pending_notification =
                            Some(Notification::error(format!("清空失败：{e}")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

/// 循环「SCAN 收集一轮 → 分批 DEL」直到该 pattern 再无匹配；返回实际删除数。
/// 不一次性收集全部 key，内存上界 = SCAN_BATCH 个 key 名
async fn delete_by_pattern(
    svc: &RedisService,
    config: &ConnectionConfig,
    db: u8,
    pattern: &str,
) -> Result<u64> {
    let mut total = 0u64;
    loop {
        let batch = svc
            .scan_all(config, db, Some(pattern), None, SCAN_BATCH)
            .await?;
        if batch.is_empty() {
            break;
        }
        let got = batch.len();
        for chunk in batch.chunks(DEL_CHUNK) {
            let mut argv = Vec::with_capacity(chunk.len() + 1);
            argv.push("DEL".to_string());
            argv.extend(chunk.iter().map(|k| k.key.clone()));
            if let RedisValue::Int(n) = svc.execute_command(config, db, argv).await? {
                total += n.max(0) as u64;
            }
        }
        // 单轮不足上限说明已扫到尾，无需再来一轮空扫
        if got < SCAN_BATCH {
            break;
        }
    }
    Ok(total)
}

/// 转义 Redis MATCH glob 特殊字符，避免前缀里的 `*?[]\` 误匹配别人的 key
fn escape_glob(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// 确认弹窗 / toast 里的 key 名截断，防超长 key 撑爆对话框
fn truncate_label(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let head: String = s.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_glob_specials() {
        assert_eq!(escape_glob("user"), "user");
        assert_eq!(escape_glob("a*b"), "a\\*b");
        assert_eq!(escape_glob("a?[c]"), "a\\?\\[c\\]");
        assert_eq!(escape_glob("a\\b"), "a\\\\b");
    }

    #[test]
    fn truncate_label_keeps_short_and_cuts_long() {
        assert_eq!(truncate_label("short", 10), "short");
        assert_eq!(truncate_label("数据库连接池", 3), "数据库…");
    }
}
