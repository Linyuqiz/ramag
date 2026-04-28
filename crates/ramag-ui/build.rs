//! 让 cargo 跟踪 assets/icons 目录变化。
//! rust-embed 的 proc-macro 在 stable Rust 上不会自动注册 rerun-if-changed，
//! 没有这个 build.rs 新加的 svg 不会触发重编。
fn main() {
    println!("cargo:rerun-if-changed=assets/icons");
}
