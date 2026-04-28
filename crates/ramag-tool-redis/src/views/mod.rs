//! Redis 工具的视图组件
//!
//! 这些视图作为子组件由 dbclient 工具装载（dbclient 是统一连接管理入口，
//! 在新建连接表单内通过 driver 选择器决定走 SQL 还是 Redis 路径）。
//!
//! - [`connection_session`]：Redis 连接打开后的会话面板（DB 切换 + Key 树 + 详情）
//! - [`key_tree`]：SCAN 分批 + 命名空间分组
//! - [`key_detail`]：按类型 dispatch 渲染值

pub mod cli_panel;
pub mod connection_session;
pub mod hash_field_form;
pub mod key_create;
pub mod key_detail;
pub mod key_tree;
pub mod list_element_form;
pub mod monitor_panel;
pub mod pubsub_panel;
pub mod set_element_form;
pub mod stream_entry_form;
pub mod ttl_edit;
pub mod value_display;
pub mod value_edit;
pub mod zset_element_form;

pub use connection_session::RedisSessionPanel;
