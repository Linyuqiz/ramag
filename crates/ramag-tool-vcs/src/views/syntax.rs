//! 代码行语法高亮：按文件扩展名选 tree-sitter 语言，逐行切成多色片段。
//! diff 内容与 Project Files 内容共用；不支持的扩展名退化为纯文本单色渲染。

use gpui::{AnyElement, Hsla, IntoElement, ParentElement, SharedString, Styled, div, prelude::*};
use gpui_component::{ActiveTheme, h_flex, highlighter::SyntaxHighlighter};
use ropey::Rope;

use super::vcs_view::VcsView;

/// 文件路径扩展名 → tree-sitter 语言名（均为 gpui-component 内置语言）。
///
/// 无扩展名 / 不在表内（Cargo.lock、.gitignore 等）→ None，调用方走纯文本渲染。
pub(super) fn lang_for_path(path: &str) -> Option<&'static str> {
    // 仅取最后一段文件名的扩展名，避免目录名里的点干扰
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e)?;
    let lang = match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "go" => "go",
        "py" => "python",
        "json" => "json",
        "js" | "jsx" | "mjs" => "javascript",
        "ts" | "tsx" => "typescript",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "sql" => "sql",
        "md" => "markdown",
        "sh" | "bash" => "bash",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "java" => "java",
        _ => return None,
    };
    Some(lang)
}

/// 渲染一行代码内容为内联元素。
///
/// - `lang = None` 或文本为空 → 单个 div（颜色 `fg`），与未高亮时完全一致。
/// - 否则用 `SyntaxHighlighter` 逐行解析，按 tree-sitter 给出的字节区间切片着色；
///   片段无颜色时回退 `fg`。区间是字节偏移，切片用 `str::get` 保证非 ASCII 不 panic。
///
/// 字号 `text_xs` + 等宽字体 + `whitespace_nowrap`，与原渲染保持一致，仅多了着色。
pub(super) fn render_code_line(
    text: &str,
    lang: Option<&str>,
    fg: Hsla,
    mono: SharedString,
    cx: &mut Context<VcsView>,
) -> AnyElement {
    let Some(lang) = lang.filter(|_| !text.is_empty()) else {
        return plain_line(text, fg, mono);
    };

    let mut hl = SyntaxHighlighter::new(lang);
    hl.update(None, &Rope::from_str(text), None);
    let theme = cx.theme().highlight_theme.clone();
    let styles = hl.styles(&(0..text.len()), &theme);

    let mut row = h_flex().text_xs().font_family(mono).whitespace_nowrap();
    for (range, style) in styles {
        // tree-sitter 区间是字节偏移；非字符边界时跳过该段，宁可少一段也不 panic
        let Some(seg) = text.get(range) else {
            continue;
        };
        if seg.is_empty() {
            continue;
        }
        let color = style.color.unwrap_or(fg);
        row = row.child(div().text_color(color).child(seg.to_string()));
    }
    row.into_any_element()
}

/// 纯文本单色行（未高亮 / 空行 / 不支持语言）
fn plain_line(text: &str, fg: Hsla, mono: SharedString) -> AnyElement {
    div()
        .text_xs()
        .font_family(mono)
        .text_color(fg)
        .whitespace_nowrap()
        .child(text.to_string())
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::lang_for_path;

    #[test]
    fn maps_known_extensions() {
        assert_eq!(lang_for_path("src/main.rs"), Some("rust"));
        assert_eq!(lang_for_path("a/b/util.go"), Some("go"));
        assert_eq!(lang_for_path("script.py"), Some("python"));
        assert_eq!(lang_for_path("data.json"), Some("json"));
        assert_eq!(lang_for_path("app.tsx"), Some("typescript"));
        assert_eq!(lang_for_path("mod.mjs"), Some("javascript"));
        assert_eq!(lang_for_path("config.yml"), Some("yaml"));
        assert_eq!(lang_for_path("header.hpp"), Some("cpp"));
        assert_eq!(lang_for_path("Main.java"), Some("java"));
    }

    #[test]
    fn case_insensitive_extension() {
        assert_eq!(lang_for_path("README.MD"), Some("markdown"));
        assert_eq!(lang_for_path("Build.SQL"), Some("sql"));
    }

    #[test]
    fn unknown_or_no_extension_is_none() {
        // 无扩展名 / 仅前缀点 / 不在表内 → 纯文本
        assert_eq!(lang_for_path("Cargo.lock"), None);
        assert_eq!(lang_for_path(".gitignore"), None);
        assert_eq!(lang_for_path("Makefile"), None);
        assert_eq!(lang_for_path("path/to/dir.with.dots/file"), None);
    }
}
