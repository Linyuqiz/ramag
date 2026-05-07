//! SQL 类 driver 共享层。每个 driver impl [`SqlBackend`] + [`impl_driver_for!`] 宏即可获得 `Driver` 实现

pub mod backend;
pub mod errors;
pub mod macros;
pub mod pool;
pub mod runtime;
pub mod sql;

pub use backend::{
    SqlBackend, cancel_query_impl, execute_impl, list_columns_impl, list_foreign_keys_impl,
    list_indexes_impl, list_schemas_impl, list_tables_impl, server_version_impl,
    test_connection_impl,
};
pub use pool::PoolCache;
pub use runtime::run_in_tokio;
