//! 树节点破坏性操作：清空表 / 删除表（视图）/ 删除库。
//! 右键菜单 → open_confirm 二次确认 → 异步 DDL（走 execute_with_history 留痕）→ 刷新 + toast

use gpui::{Context, Entity};
use gpui_component::menu::{PopupMenu, PopupMenuItem};
use gpui_component::notification::Notification;
use ramag_domain::entities::{DriverKind, Query};
use ramag_ui::{open_confirm, open_prompt};

use super::TableTreePanel;

/// DDL 完成后的树刷新方式
enum AfterDdl {
    /// 树结构无变化（清空表）
    None,
    /// 重拉单个 schema 的表列表（删除表 / 视图）
    ReloadSchema(String),
    /// 整树重拉（删除库）
    FullRefresh,
}

// ===== 右键菜单构造（row.rs 调用） =====

/// 表 / 视图行右键菜单：查看 DDL + 清空表（仅基础表）+ 删除
pub(super) fn table_context_menu(
    menu: PopupMenu,
    entity: Entity<TableTreePanel>,
    schema: String,
    table: String,
    is_view: bool,
) -> PopupMenu {
    let ddl_label = if is_view {
        "查看视图定义"
    } else {
        "查看建表 SQL"
    };
    let (s, t, ent) = (schema.clone(), table.clone(), entity.clone());
    let menu = menu
        .item(PopupMenuItem::new(ddl_label).on_click(move |_, _, app| {
            let (s, t) = (s.clone(), t.clone());
            ent.update(app, |this, cx| this.handle_show_ddl(s, t, is_view, cx));
        }))
        .separator();

    let rename_title = if is_view {
        "重命名视图"
    } else {
        "重命名表"
    };
    let (s, t, ent) = (schema.clone(), table.clone(), entity.clone());
    let menu = menu.item(
        PopupMenuItem::new("重命名…").on_click(move |_, window, app| {
            let (s, t, ent) = (s.clone(), t.clone(), ent.clone());
            open_prompt(
                rename_title,
                format!("输入 {s}.{t} 的新名称"),
                &t.clone(),
                "重命名",
                move |new_name, _, app| {
                    ent.update(app, |this, cx| {
                        this.rename_table(s, t, new_name, is_view, cx)
                    });
                },
                window,
                app,
            );
        }),
    );

    let menu = if is_view {
        menu
    } else {
        let (s, t, ent) = (schema.clone(), table.clone(), entity.clone());
        menu.item(
            PopupMenuItem::new("清空表…").on_click(move |_, window, app| {
                let (s, t, ent) = (s.clone(), t.clone(), ent.clone());
                open_confirm(
                    "清空表",
                    format!("将删除 {s}.{t} 的全部数据（TRUNCATE TABLE），此操作不可恢复。"),
                    "清空",
                    true,
                    move |_, app| {
                        ent.update(app, |this, cx| this.truncate_table(s, t, cx));
                    },
                    window,
                    app,
                );
            }),
        )
    };

    let (label, title, desc) = if is_view {
        (
            "删除视图…",
            "删除视图",
            format!("将删除视图 {schema}.{table}（仅删除视图定义，不影响底层表数据）。"),
        )
    } else {
        (
            "删除表…",
            "删除表",
            format!("将永久删除表 {schema}.{table}（表结构与数据一并删除），此操作不可恢复。"),
        )
    };
    menu.item(PopupMenuItem::new(label).on_click(move |_, window, app| {
        let (s, t, ent) = (schema.clone(), table.clone(), entity.clone());
        open_confirm(
            title,
            desc.clone(),
            "删除",
            true,
            move |_, app| {
                ent.update(app, |this, cx| this.drop_table(s, t, is_view, cx));
            },
            window,
            app,
        );
    }))
}

/// schema 行右键菜单：删除库（MySQL：DROP DATABASE；PG：DROP SCHEMA … CASCADE）
pub(super) fn schema_context_menu(
    menu: PopupMenu,
    entity: Entity<TableTreePanel>,
    schema: String,
    driver: DriverKind,
) -> PopupMenu {
    // schema 重命名仅 PG 支持（ALTER SCHEMA … RENAME TO）；MySQL 官方已移除 RENAME DATABASE
    let menu = if matches!(driver, DriverKind::Postgres) {
        let (s, ent) = (schema.clone(), entity.clone());
        menu.item(
            PopupMenuItem::new("重命名 Schema…").on_click(move |_, window, app| {
                let (s, ent) = (s.clone(), ent.clone());
                open_prompt(
                    "重命名 Schema",
                    format!("输入 schema {s} 的新名称"),
                    &s.clone(),
                    "重命名",
                    move |new_name, _, app| {
                        ent.update(app, |this, cx| this.rename_schema(s, new_name, cx));
                    },
                    window,
                    app,
                );
            }),
        )
    } else {
        menu
    };

    let (label, title, desc) = match driver {
        DriverKind::Postgres => (
            "删除 Schema…",
            "删除 Schema",
            format!(
                "将永久删除 schema {schema} 及其中全部对象（DROP SCHEMA … CASCADE），此操作不可恢复。"
            ),
        ),
        _ => (
            "删除数据库…",
            "删除数据库",
            format!("将永久删除数据库 {schema} 及其中全部表与数据，此操作不可恢复。"),
        ),
    };
    menu.item(PopupMenuItem::new(label).on_click(move |_, window, app| {
        let (schema, ent) = (schema.clone(), entity.clone());
        open_confirm(
            title,
            desc.clone(),
            "删除",
            true,
            move |_, app| {
                ent.update(app, |this, cx| this.drop_schema(schema, cx));
            },
            window,
            app,
        );
    }))
}

// ===== DDL 执行 =====

impl TableTreePanel {
    pub(super) fn truncate_table(&mut self, schema: String, table: String, cx: &mut Context<Self>) {
        let Some(driver) = self.connection.as_ref().map(|c| c.driver) else {
            return;
        };
        let sql = ddl_truncate_table(driver, &schema, &table);
        self.exec_ddl(
            sql,
            format!("已清空表 {schema}.{table}"),
            AfterDdl::None,
            cx,
        );
    }

    pub(super) fn drop_table(
        &mut self,
        schema: String,
        table: String,
        is_view: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(driver) = self.connection.as_ref().map(|c| c.driver) else {
            return;
        };
        if self.selected.as_ref() == Some(&(schema.clone(), table.clone())) {
            self.selected = None;
        }
        self.table_columns.remove(&format!("{schema}.{table}"));
        let sql = ddl_drop_table(driver, &schema, &table, is_view);
        let label = if is_view { "视图" } else { "表" };
        self.exec_ddl(
            sql,
            format!("已删除{label} {schema}.{table}"),
            AfterDdl::ReloadSchema(schema),
            cx,
        );
    }

    pub(super) fn rename_table(
        &mut self,
        schema: String,
        old: String,
        new: String,
        is_view: bool,
        cx: &mut Context<Self>,
    ) {
        if new == old {
            return;
        }
        let Some(driver) = self.connection.as_ref().map(|c| c.driver) else {
            return;
        };
        if self.selected.as_ref() == Some(&(schema.clone(), old.clone())) {
            self.selected = None;
        }
        self.table_columns.remove(&format!("{schema}.{old}"));
        let sql = ddl_rename_table(driver, &schema, &old, &new, is_view);
        self.exec_ddl(
            sql,
            format!("已重命名为 {schema}.{new}"),
            AfterDdl::ReloadSchema(schema),
            cx,
        );
    }

    /// 仅 PG（菜单层已限制）
    pub(super) fn rename_schema(&mut self, old: String, new: String, cx: &mut Context<Self>) {
        if new == old {
            return;
        }
        let Some(driver) = self.connection.as_ref().map(|c| c.driver) else {
            return;
        };
        if self.active_schema.as_deref() == Some(old.as_str()) {
            self.active_schema = None;
        }
        let sql = format!(
            "ALTER SCHEMA {} RENAME TO {}",
            driver.quote_identifier(&old),
            driver.quote_identifier(&new)
        );
        self.exec_ddl(sql, format!("已重命名为 {new}"), AfterDdl::FullRefresh, cx);
    }

    pub(super) fn drop_schema(&mut self, schema: String, cx: &mut Context<Self>) {
        let Some(driver) = self.connection.as_ref().map(|c| c.driver) else {
            return;
        };
        // 让 refresh 后的 load_schemas 自动激活默认库并广播
        if self.active_schema.as_deref() == Some(schema.as_str()) {
            self.active_schema = None;
        }
        let sql = ddl_drop_schema(driver, &schema);
        self.exec_ddl(sql, format!("已删除 {schema}"), AfterDdl::FullRefresh, cx);
    }

    /// 统一执行入口：成功按 after 刷新树，失败 toast 错误；均写查询历史
    fn exec_ddl(
        &mut self,
        sql: String,
        success_msg: String,
        after: AfterDdl,
        cx: &mut Context<Self>,
    ) {
        let Some(conn) = self.connection.clone() else {
            return;
        };
        let svc = self.service.clone();
        cx.spawn(async move |this, cx| {
            let result = svc
                .execute_with_history(&conn, &Query::new(sql.clone()))
                .await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(_) => {
                        this.pending_notification =
                            Some(Notification::success(success_msg).autohide(true));
                        match after {
                            AfterDdl::None => {}
                            AfterDdl::ReloadSchema(s) => {
                                if this.expanded.contains_key(&s) {
                                    this.load_tables_for(s, cx);
                                }
                            }
                            AfterDdl::FullRefresh => this.refresh(cx),
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, sql = %sql, "tree ddl failed");
                        this.pending_notification =
                            Some(Notification::error(e.write_hint("执行失败")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

// ===== DDL 语句生成（纯函数） =====

fn ddl_truncate_table(driver: DriverKind, schema: &str, table: &str) -> String {
    format!(
        "TRUNCATE TABLE {}.{}",
        driver.quote_identifier(schema),
        driver.quote_identifier(table)
    )
}

fn ddl_drop_table(driver: DriverKind, schema: &str, table: &str, is_view: bool) -> String {
    let kind = if is_view { "VIEW" } else { "TABLE" };
    format!(
        "DROP {kind} {}.{}",
        driver.quote_identifier(schema),
        driver.quote_identifier(table)
    )
}

/// MySQL：RENAME TABLE（表 / 视图通用，新名带 schema）；PG：ALTER TABLE/VIEW … RENAME TO（新名不带 schema）
fn ddl_rename_table(
    driver: DriverKind,
    schema: &str,
    old: &str,
    new: &str,
    is_view: bool,
) -> String {
    let qs = driver.quote_identifier(schema);
    let qo = driver.quote_identifier(old);
    let qn = driver.quote_identifier(new);
    match driver {
        DriverKind::Postgres => {
            let kind = if is_view { "VIEW" } else { "TABLE" };
            format!("ALTER {kind} {qs}.{qo} RENAME TO {qn}")
        }
        _ => format!("RENAME TABLE {qs}.{qo} TO {qs}.{qn}"),
    }
}

/// MySQL 的 schema 即 database；PG 树展示的是 schema，加 CASCADE 才能删非空 schema
fn ddl_drop_schema(driver: DriverKind, schema: &str) -> String {
    let q = driver.quote_identifier(schema);
    match driver {
        DriverKind::Postgres => format!("DROP SCHEMA {q} CASCADE"),
        _ => format!("DROP DATABASE {q}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_quotes_by_dialect() {
        assert_eq!(
            ddl_truncate_table(DriverKind::Mysql, "shop", "order"),
            "TRUNCATE TABLE `shop`.`order`"
        );
        assert_eq!(
            ddl_truncate_table(DriverKind::Postgres, "public", "order"),
            "TRUNCATE TABLE \"public\".\"order\""
        );
    }

    #[test]
    fn drop_table_and_view() {
        assert_eq!(
            ddl_drop_table(DriverKind::Mysql, "shop", "t1", false),
            "DROP TABLE `shop`.`t1`"
        );
        assert_eq!(
            ddl_drop_table(DriverKind::Postgres, "public", "v1", true),
            "DROP VIEW \"public\".\"v1\""
        );
    }

    #[test]
    fn drop_schema_dialect_split() {
        assert_eq!(
            ddl_drop_schema(DriverKind::Mysql, "shop"),
            "DROP DATABASE `shop`"
        );
        assert_eq!(
            ddl_drop_schema(DriverKind::Postgres, "app"),
            "DROP SCHEMA \"app\" CASCADE"
        );
    }

    /// 标识符内引号必须转义，防注入式构造
    #[test]
    fn identifier_escaping() {
        assert_eq!(
            ddl_drop_schema(DriverKind::Mysql, "a`b"),
            "DROP DATABASE `a``b`"
        );
    }

    #[test]
    fn rename_table_dialect_split() {
        assert_eq!(
            ddl_rename_table(DriverKind::Mysql, "shop", "t1", "t2", false),
            "RENAME TABLE `shop`.`t1` TO `shop`.`t2`"
        );
        assert_eq!(
            ddl_rename_table(DriverKind::Postgres, "public", "t1", "t2", false),
            "ALTER TABLE \"public\".\"t1\" RENAME TO \"t2\""
        );
        assert_eq!(
            ddl_rename_table(DriverKind::Postgres, "public", "v1", "v2", true),
            "ALTER VIEW \"public\".\"v1\" RENAME TO \"v2\""
        );
    }
}
