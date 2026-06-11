//! 纯函数：列表过滤 + 相对时间格式化（便于测试，不依赖 GPUI）

use chrono::{DateTime, Utc};
use ramag_domain::entities::{ClipItem, ClipKind};

/// 过滤 + 排序：钉住优先，组内按 last_used_at desc。
/// 搜索匹配 preview / text（大小写不敏感）；kind=None 不限类型
pub fn filter_items<'a>(
    items: &'a [ClipItem],
    query: &str,
    kind: Option<ClipKind>,
) -> Vec<&'a ClipItem> {
    let q = query.trim().to_lowercase();
    let mut out: Vec<&ClipItem> = items
        .iter()
        .filter(|i| kind.is_none_or(|k| i.kind == k))
        .filter(|i| q.is_empty() || matches_query(i, &q))
        .collect();
    out.sort_by(|a, b| {
        b.pinned
            .cmp(&a.pinned)
            .then(b.last_used_at.cmp(&a.last_used_at))
    });
    out
}

fn matches_query(item: &ClipItem, q_lower: &str) -> bool {
    if item.preview.to_lowercase().contains(q_lower) {
        return true;
    }
    item.text
        .as_deref()
        .is_some_and(|t| t.to_lowercase().contains(q_lower))
}

/// 相对时间：刚刚 / N 分钟前 / N 小时前 / N 天前 / 日期
pub fn relative_time(then: DateTime<Utc>, now: DateTime<Utc>) -> String {
    let secs = (now - then).num_seconds().max(0);
    match secs {
        0..=59 => "刚刚".to_string(),
        60..=3599 => format!("{} 分钟前", secs / 60),
        3600..=86399 => format!("{} 小时前", secs / 3600),
        86400..=604_799 => format!("{} 天前", secs / 86400),
        _ => then.format("%Y-%m-%d").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;
    use ramag_domain::entities::{ClipId, fnv1a_hash};

    fn clip(text: &str, kind: ClipKind, pinned: bool, age_secs: i64) -> ClipItem {
        let at = Utc::now() - Duration::seconds(age_secs);
        ClipItem {
            id: ClipId::new(),
            kind,
            text: Some(text.to_string()),
            rtf: None,
            image_path: None,
            thumb_path: None,
            image_dims: None,
            files: Vec::new(),
            preview: text.to_string(),
            source: None,
            byte_size: 0,
            pinned,
            content_hash: format!("{:016x}", fnv1a_hash(text.as_bytes())),
            created_at: at,
            last_used_at: at,
        }
    }

    #[test]
    fn pinned_first_then_recent() {
        let items = vec![
            clip("old", ClipKind::Text, false, 100),
            clip("new", ClipKind::Text, false, 1),
            clip("pinned-old", ClipKind::Text, true, 500),
        ];
        let out = filter_items(&items, "", None);
        assert_eq!(out[0].text.as_deref(), Some("pinned-old"));
        assert_eq!(out[1].text.as_deref(), Some("new"));
        assert_eq!(out[2].text.as_deref(), Some("old"));
    }

    #[test]
    fn filter_by_kind_and_query() {
        let items = vec![
            clip("hello world", ClipKind::Text, false, 1),
            clip("https://x.com", ClipKind::Link, false, 1),
        ];
        assert_eq!(filter_items(&items, "", Some(ClipKind::Link)).len(), 1);
        assert_eq!(filter_items(&items, "HELLO", None).len(), 1);
        assert_eq!(filter_items(&items, "zzz", None).len(), 0);
    }

    #[test]
    fn relative_time_buckets() {
        let now = Utc::now();
        assert_eq!(relative_time(now, now), "刚刚");
        assert_eq!(relative_time(now - Duration::minutes(5), now), "5 分钟前");
        assert_eq!(relative_time(now - Duration::hours(3), now), "3 小时前");
        assert_eq!(relative_time(now - Duration::days(2), now), "2 天前");
    }
}
