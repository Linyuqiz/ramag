//! `parse_search_query` 单元测试

#![allow(clippy::unwrap_used)]

use super::parse_search_query;

#[test]
fn parses_pure_keyword_into_grep() {
    let (g, a, s) = parse_search_query("bug fix");
    assert_eq!(g.as_deref(), Some("bug fix"));
    assert!(a.is_none());
    assert!(s.is_none());
}

#[test]
fn parses_author_prefix() {
    let (g, a, s) = parse_search_query("@alice");
    assert!(g.is_none());
    assert_eq!(a.as_deref(), Some("alice"));
    assert!(s.is_none());
}

#[test]
fn parses_relative_time() {
    let (g, a, s) = parse_search_query("7d");
    assert!(g.is_none());
    assert!(a.is_none());
    assert_eq!(s.as_deref(), Some("7 days ago"));
}

#[test]
fn mixes_three_kinds() {
    let (g, a, s) = parse_search_query("bug @alice 1w");
    assert_eq!(g.as_deref(), Some("bug"));
    assert_eq!(a.as_deref(), Some("alice"));
    assert_eq!(s.as_deref(), Some("1 weeks ago"));
}

#[test]
fn empty_returns_all_none() {
    let (g, a, s) = parse_search_query("");
    assert!(g.is_none());
    assert!(a.is_none());
    assert!(s.is_none());
}

#[test]
fn ignores_lone_at_sign() {
    // "@" 单字符不算 author（strip_prefix 后剩空串）→ 落到 grep
    let (g, a, _) = parse_search_query("@");
    assert_eq!(g.as_deref(), Some("@"));
    assert!(a.is_none());
}

#[test]
fn relative_time_supports_all_units() {
    for (input, expected) in [
        ("12h", "12 hours ago"),
        ("3d", "3 days ago"),
        ("2w", "2 weeks ago"),
        ("6m", "6 months ago"),
        ("1y", "1 years ago"),
    ] {
        let (_g, _a, s) = parse_search_query(input);
        assert_eq!(
            s.as_deref(),
            Some(expected),
            "input={input} should parse as {expected}"
        );
    }
}

#[test]
fn invalid_relative_time_falls_to_grep() {
    // "abc" 不是合法时间 → 当作 grep
    let (g, _, s) = parse_search_query("abc");
    assert_eq!(g.as_deref(), Some("abc"));
    assert!(s.is_none());
    // "5q"（无效单位）也该落到 grep
    let (g, _, s) = parse_search_query("5q");
    assert_eq!(g.as_deref(), Some("5q"));
    assert!(s.is_none());
}

#[test]
fn multi_word_grep_joined_by_space() {
    let (g, _, _) = parse_search_query("foo bar baz");
    assert_eq!(g.as_deref(), Some("foo bar baz"));
}

#[test]
fn author_only_picks_last_at_token() {
    // 现实使用：极少同时出现两个 @xxx；按行为我们覆盖第一个
    let (_g, a, _) = parse_search_query("@bob @alice");
    assert_eq!(a.as_deref(), Some("alice"));
}
