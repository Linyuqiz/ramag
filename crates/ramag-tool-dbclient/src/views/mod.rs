//! DB Client 工具视图集合
//!
//! 视图层次：
//! - [`DbClientView`]：根视图，三栏布局
//! - [`connection_list::ConnectionListPanel`]：左栏连接列表
//! - [`connection_form::ConnectionFormPanel`]：连接增/改表单
//! - [`table_tree::TableTreePanel`]：表树（schema → tables）

pub mod cell_edit_dialog;
pub mod connection_form;
pub mod connection_list;
pub mod connection_session;
pub mod dbclient_view;
pub mod history_panel;
pub mod query_panel;
pub mod query_tab;
pub mod result_panel;
pub mod result_table;
pub mod table_tree;
pub mod tree_helpers;

pub use dbclient_view::DbClientView;
