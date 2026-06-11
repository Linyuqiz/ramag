//! 图片解密缓存：图片落盘是密文，UI 不能直接 img(path)。
//! 渲染时同步查缓存；miss 则异步解密解码填充 + notify，下一帧显示。
//! 内部 RefCell，故 render（&self）可填发起加载

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use gpui::Image;

#[derive(Default)]
pub(crate) struct ImageCache {
    cache: RefCell<HashMap<String, Arc<Image>>>,
    loading: RefCell<HashSet<String>>,
}

impl ImageCache {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// 同步取已解密图片
    pub(crate) fn peek(&self, path: &str) -> Option<Arc<Image>> {
        self.cache.borrow().get(path).cloned()
    }

    /// 抢加载权：之前既未缓存也未在加载中才返回 true（防同路径重复 spawn）
    pub(crate) fn begin_load(&self, path: &str) -> bool {
        if self.cache.borrow().contains_key(path) {
            return false;
        }
        self.loading.borrow_mut().insert(path.to_string())
    }

    pub(crate) fn insert(&self, path: String, image: Arc<Image>) {
        self.loading.borrow_mut().remove(&path);
        self.cache.borrow_mut().insert(path, image);
    }

    /// 加载失败：清加载标记（不缓存，下次可重试）
    pub(crate) fn fail(&self, path: &str) {
        self.loading.borrow_mut().remove(path);
    }
}
