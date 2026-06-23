//! ResultPanel DML：行内编辑触发的 INSERT / UPDATE / DELETE

use gpui::Context;
use gpui_component::notification::Notification;
use ramag_domain::entities::{Query, Value};
use tracing::error;

use super::ResultPanel;
use super::ResultState;
use super::helpers::{
    build_new_value, build_pk_where, dml_row_limit, escape_new_value_for_old, find_pk_idx,
};

impl ResultPanel {
    /// 删除前的预览数据：(row_idx, "列=值" 简短文案)；调用方拿去给 confirm dialog 用
    /// 优先用主键列做预览，没主键用第一列
    pub(crate) fn delete_preview(&self) -> Option<(usize, String)> {
        let (ri, _) = self.selected_cell?;
        let ResultState::Ok(result) = &self.state else {
            return None;
        };
        let row = result.rows.get(ri)?;
        let idx = find_pk_idx(result).unwrap_or(0);
        let col = result.columns.get(idx)?.clone();
        let val = row
            .values
            .get(idx)
            .map(|v| v.display_preview(60))
            .unwrap_or_default();
        Some((ri, format!("{col} = {val}")))
    }

    /// 批量删除前的预览：返回 (排序去重后的 indices, "N 行预览" 文案)
    pub(crate) fn delete_preview_multi(&self) -> Option<(Vec<usize>, String)> {
        if self.selected_rows.is_empty() {
            return None;
        }
        let ResultState::Ok(result) = &self.state else {
            return None;
        };
        let mut indices: Vec<usize> = self
            .selected_rows
            .iter()
            .copied()
            .filter(|i| *i < result.rows.len())
            .collect();
        indices.sort();
        indices.dedup();
        if indices.is_empty() {
            return None;
        }
        let pk_or_first = find_pk_idx(result).unwrap_or(0);
        let preview_col = result.columns.get(pk_or_first).cloned().unwrap_or_default();
        let mut samples: Vec<String> = indices
            .iter()
            .take(3)
            .filter_map(|&ri| {
                let row = result.rows.get(ri)?;
                let val = row
                    .values
                    .get(pk_or_first)
                    .map(|v| v.display_preview(40))
                    .unwrap_or_default();
                Some(format!("{preview_col} = {val}"))
            })
            .collect();
        if indices.len() > 3 {
            samples.push(format!("…还有 {} 行", indices.len() - 3));
        }
        let summary = format!("将删除 {} 行：{}", indices.len(), samples.join(" / "));
        Some((indices, summary))
    }

    /// 批量执行 DELETE：每行独立 SQL（DELETE ... WHERE ... LIMIT 1），串行 await
    pub(crate) fn execute_delete_rows_async(
        &mut self,
        indices: Vec<usize>,
        cx: &mut Context<Self>,
    ) {
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再删除")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        let by_pk = find_pk_idx(result).is_some();
        let strategy = if by_pk {
            "按主键"
        } else {
            "按全列等值"
        };

        let driver = conn.driver;
        let limit_clause = dml_row_limit(driver);
        let plans: Vec<(usize, String)> = indices
            .iter()
            .filter_map(|&ri| {
                let row = result.rows.get(ri)?;
                let where_clause = build_pk_where(result, row, driver);
                let sql = format!("DELETE FROM {table_ref} WHERE {where_clause}{limit_clause};");
                Some((ri, sql))
            })
            .collect();
        if plans.is_empty() {
            return;
        }

        cx.spawn(async move |this, cx| {
            let mut deleted: Vec<usize> = Vec::new();
            let mut last_err: Option<ramag_domain::error::DomainError> = None;
            for (ri, sql) in plans {
                let q = Query::new(sql);
                match svc.execute_with_history(&conn, &q).await {
                    Ok(qr) if qr.affected_rows > 0 => deleted.push(ri),
                    Ok(_) => {}
                    Err(e) => {
                        error!(error = %e, "delete row failed (in batch)");
                        last_err = Some(e);
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                if let ResultState::Ok(r) = &mut this.state {
                    let mut to_remove = deleted.clone();
                    to_remove.sort_by(|a, b| b.cmp(a));
                    for ri in to_remove {
                        if ri < r.rows.len() {
                            r.rows.remove(ri);
                        }
                    }
                }
                this.selected_rows.clear();
                this.selected_cell = None;
                this.pending_notification = Some(if let Some(e) = last_err {
                    Notification::error(e.write_hint(&format!("已删除 {} 行后出错", deleted.len())))
                        .autohide(true)
                } else {
                    Notification::success(format!("已删除 {} 行（{strategy}匹配）", deleted.len()))
                        .autohide(true)
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// 新增行弹框确认后调用：异步执行 INSERT，成功后本地 rows.push
    pub(crate) fn apply_insert_async(
        &mut self,
        values: Vec<(String, Value)>,
        cx: &mut Context<Self>,
    ) {
        if values.is_empty() {
            return;
        }
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法新增").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法新增").autohide(true));
            cx.notify();
            return;
        };
        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再新增")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        let driver = conn.driver;
        let cols_sql = values
            .iter()
            .map(|(c, _)| driver.quote_identifier(c))
            .collect::<Vec<_>>()
            .join(", ");
        let vals_sql = values
            .iter()
            .map(|(_, v)| v.to_sql_literal())
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("INSERT INTO {table_ref} ({cols_sql}) VALUES ({vals_sql});");
        let q = Query::new(sql);
        let new_row_values: Option<Vec<Value>> = match &self.state {
            ResultState::Ok(r) => Some(
                r.columns
                    .iter()
                    .map(|c| {
                        values
                            .iter()
                            .find(|(name, _)| name.eq_ignore_ascii_case(c))
                            .map(|(_, v)| v.clone())
                            .unwrap_or(Value::Null)
                    })
                    .collect(),
            ),
            _ => None,
        };

        cx.spawn(async move |this, cx| {
            let outcome = svc.execute_with_history(&conn, &q).await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(qr) => {
                        if qr.affected_rows == 0 {
                            this.pending_notification = Some(
                                Notification::warning("INSERT 未影响任何行（请检查约束）")
                                    .autohide(true),
                            );
                        } else {
                            if let (ResultState::Ok(r), Some(vs)) =
                                (&mut this.state, new_row_values)
                            {
                                r.rows.push(ramag_domain::entities::Row { values: vs });
                            }
                            this.pending_notification = Some(
                                Notification::success(format!("已新增 {} 行", qr.affected_rows))
                                    .autohide(true),
                            );
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "insert row failed");
                        this.pending_notification =
                            Some(Notification::error(e.write_hint("新增失败")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 二次确认后真执行 DELETE：异步发到 DB，成功后本地移除该行
    pub(crate) fn execute_delete_row_async(&mut self, ri: usize, cx: &mut Context<Self>) {
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法删除").autohide(true));
            cx.notify();
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(row) = result.rows.get(ri).cloned() else {
            return;
        };

        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再删除")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        let by_pk = find_pk_idx(result).is_some();
        let strategy = if by_pk {
            "按主键"
        } else {
            "按全列等值"
        };
        let where_clause = build_pk_where(result, &row, conn.driver);
        let limit_clause = dml_row_limit(conn.driver);
        let sql = format!("DELETE FROM {table_ref} WHERE {where_clause}{limit_clause};");
        let q = Query::new(sql);

        cx.spawn(async move |this, cx| {
            let outcome = svc.execute_with_history(&conn, &q).await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(qr) => {
                        if qr.affected_rows == 0 {
                            this.pending_notification = Some(
                                Notification::warning(
                                    "DELETE 未匹配到记录（请检查主键或行已被改动）",
                                )
                                .autohide(true),
                            );
                        } else {
                            if let ResultState::Ok(r) = &mut this.state
                                && ri < r.rows.len()
                            {
                                r.rows.remove(ri);
                            }
                            this.selected_cell = None;
                            this.pending_notification = Some(
                                Notification::success(format!(
                                    "已删除 {} 行（{strategy}匹配）",
                                    qr.affected_rows
                                ))
                                .autohide(true),
                            );
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "delete row failed");
                        this.pending_notification =
                            Some(Notification::error(e.write_hint("删除失败")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// 单元格编辑弹框「确认修改」：异步执行 UPDATE，成功后同步本地 cell
    pub(crate) fn apply_cell_update_async(
        &mut self,
        ri: usize,
        ci: usize,
        new_text: String,
        cx: &mut Context<Self>,
    ) {
        let Some(svc) = self.service.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法修改").autohide(true));
            cx.notify();
            return;
        };
        let Some(conn) = self.connection.clone() else {
            self.pending_notification =
                Some(Notification::warning("当前未注入连接，无法修改").autohide(true));
            cx.notify();
            return;
        };
        let ResultState::Ok(result) = &self.state else {
            return;
        };
        let Some(row) = result.rows.get(ri).cloned() else {
            return;
        };
        let Some(col_name) = result.columns.get(ci).cloned() else {
            return;
        };
        let Some(cell_val) = row.values.get(ci).cloned() else {
            return;
        };

        let table_ref = match self.current_table_ref() {
            Some(t) => t,
            None => {
                self.pending_notification = Some(
                    Notification::error("无法识别目标表，请先用 SELECT 单表查询后再编辑")
                        .autohide(true),
                );
                cx.notify();
                return;
            }
        };

        let by_pk = find_pk_idx(result).is_some();
        let strategy = if by_pk {
            "按主键"
        } else {
            "按全列等值"
        };

        let driver = conn.driver;
        let where_clause = build_pk_where(result, &row, driver);
        let new_literal = escape_new_value_for_old(&cell_val, &new_text);
        let limit_clause = dml_row_limit(driver);
        let sql = format!(
            "UPDATE {table_ref} SET {} = {new_literal} WHERE {where_clause}{limit_clause};",
            driver.quote_identifier(&col_name),
        );
        let new_cell_val = build_new_value(&cell_val, &new_text);
        let q = Query::new(sql);

        cx.spawn(async move |this, cx| {
            let outcome = svc.execute_with_history(&conn, &q).await;
            let _ = this.update(cx, |this, cx| {
                match outcome {
                    Ok(qr) => {
                        if qr.affected_rows == 0 {
                            this.pending_notification = Some(
                                Notification::warning("UPDATE 未匹配到记录（请检查主键）")
                                    .autohide(true),
                            );
                        } else {
                            if let ResultState::Ok(r) = &mut this.state
                                && let Some(row) = r.rows.get_mut(ri)
                                && let Some(slot) = row.values.get_mut(ci)
                            {
                                *slot = new_cell_val;
                            }
                            this.pending_notification = Some(
                                Notification::success(format!(
                                    "已更新 {} 行（{strategy}匹配）",
                                    qr.affected_rows
                                ))
                                .autohide(true),
                            );
                        }
                    }
                    Err(e) => {
                        error!(error = %e, "apply cell update failed");
                        this.pending_notification =
                            Some(Notification::error(e.write_hint("更新失败")).autohide(true));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}
