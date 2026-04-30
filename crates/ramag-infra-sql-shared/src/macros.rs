//! `impl_driver_for!` 宏：driver crate 一行获得 [`ramag_domain::traits::Driver`] 实现
//!
//! 因 Rust orphan rule（Driver 在 ramag-domain，shared 无法对泛型 T 提供 blanket impl），
//! 改用宏展开方式：driver crate 调一次 `impl_driver_for!(MysqlBackend);` 即获得 Driver。
//!
//! # 要求
//!
//! 调宏的 driver 类型必须实现：
//! - [`crate::SqlBackend`]
//! - [`Clone`]：宏内部用 `self.clone()` 把 owned 副本送进 `run_in_tokio`（跨 runtime 派发要求 'static）；
//!   driver 内部状态都是 `Arc<...>`，Clone 是 O(1) 引用计数 +1
//! - `Send + Sync + 'static`（SqlBackend supertrait 已要求）

/// 把 SqlBackend 实现一行展开成 Driver 实现
///
/// 用法：
/// ```ignore
/// #[derive(Clone)]
/// pub struct MysqlBackend { /* ... */ }
/// impl ramag_infra_sql_shared::SqlBackend for MysqlBackend { /* ... */ }
/// ramag_infra_sql_shared::impl_driver_for!(MysqlBackend);
/// ```
#[macro_export]
macro_rules! impl_driver_for {
    ($ty:ty) => {
        #[::async_trait::async_trait]
        impl ::ramag_domain::traits::Driver for $ty {
            fn name(&self) -> &'static str {
                <$ty as $crate::SqlBackend>::name(self)
            }

            async fn test_connection(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
            ) -> ::ramag_domain::error::Result<()> {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                $crate::run_in_tokio(async move {
                    $crate::test_connection_impl(&this, &config).await
                })
                .await
            }

            async fn server_version(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
            ) -> ::ramag_domain::error::Result<::std::string::String> {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                $crate::run_in_tokio(async move {
                    $crate::server_version_impl(&this, &config).await
                })
                .await
            }

            async fn execute(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
                query: &::ramag_domain::entities::Query,
            ) -> ::ramag_domain::error::Result<::ramag_domain::entities::QueryResult> {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                let query = query.clone();
                $crate::run_in_tokio(async move {
                    $crate::execute_impl(&this, &config, &query, None).await
                })
                .await
            }

            async fn execute_cancellable(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
                query: &::ramag_domain::entities::Query,
                handle: ::ramag_domain::traits::CancelHandle,
            ) -> ::ramag_domain::error::Result<::ramag_domain::entities::QueryResult> {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                let query = query.clone();
                $crate::run_in_tokio(async move {
                    $crate::execute_impl(&this, &config, &query, ::std::option::Option::Some(handle))
                        .await
                })
                .await
            }

            async fn cancel_query(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
                thread_id: u64,
            ) -> ::ramag_domain::error::Result<()> {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                $crate::run_in_tokio(async move {
                    $crate::cancel_query_impl(&this, &config, thread_id).await
                })
                .await
            }

            async fn list_schemas(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
            ) -> ::ramag_domain::error::Result<::std::vec::Vec<::ramag_domain::entities::Schema>>
            {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                $crate::run_in_tokio(async move {
                    $crate::list_schemas_impl(&this, &config).await
                })
                .await
            }

            async fn list_tables(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
                schema: &str,
            ) -> ::ramag_domain::error::Result<::std::vec::Vec<::ramag_domain::entities::Table>>
            {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                let schema = schema.to_string();
                $crate::run_in_tokio(async move {
                    $crate::list_tables_impl(&this, &config, &schema).await
                })
                .await
            }

            async fn list_columns(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
                schema: &str,
                table: &str,
            ) -> ::ramag_domain::error::Result<::std::vec::Vec<::ramag_domain::entities::Column>>
            {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                let schema = schema.to_string();
                let table = table.to_string();
                $crate::run_in_tokio(async move {
                    $crate::list_columns_impl(&this, &config, &schema, &table).await
                })
                .await
            }

            async fn list_indexes(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
                schema: &str,
                table: &str,
            ) -> ::ramag_domain::error::Result<::std::vec::Vec<::ramag_domain::entities::Index>>
            {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                let schema = schema.to_string();
                let table = table.to_string();
                $crate::run_in_tokio(async move {
                    $crate::list_indexes_impl(&this, &config, &schema, &table).await
                })
                .await
            }

            async fn list_foreign_keys(
                &self,
                config: &::ramag_domain::entities::ConnectionConfig,
                schema: &str,
                table: &str,
            ) -> ::ramag_domain::error::Result<
                ::std::vec::Vec<::ramag_domain::entities::ForeignKey>,
            > {
                let this = <$ty as ::std::clone::Clone>::clone(self);
                let config = config.clone();
                let schema = schema.to_string();
                let table = table.to_string();
                $crate::run_in_tokio(async move {
                    $crate::list_foreign_keys_impl(&this, &config, &schema, &table).await
                })
                .await
            }
        }
    };
}
