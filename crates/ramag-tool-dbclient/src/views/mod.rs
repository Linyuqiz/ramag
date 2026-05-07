//! DB Client 视图集合：DbClientView 根视图 + 连接 / 表单 / 表树等子面板

pub mod cell_edit_dialog;
pub mod connection_form;
pub mod connection_list;
pub mod connection_session;
pub mod dbclient_view;
pub mod ddl;
pub mod history_panel;
pub mod query_panel;
pub mod query_tab;
pub mod result_panel;
pub mod result_table;
pub mod table_tree;
pub mod tree_helpers;

pub use dbclient_view::DbClientView;
