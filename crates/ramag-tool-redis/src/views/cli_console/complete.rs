//! 命令行补全：实现 gpui-component CompletionProvider，补全 Redis 命令名 + 语法提示。
//! 仅在「行首命令位」触发；参数位不补（subcommand 暂不展开，保持 KISS）。

use std::rc::Rc;

use anyhow::Result;
use gpui::{Context, Task, Window};
use gpui_component::RopeExt;
use gpui_component::input::{CompletionProvider, InputState};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionItemKind, CompletionResponse, CompletionTextEdit,
    Documentation, InsertReplaceEdit, MarkupContent, MarkupKind,
};
use ropey::Rope;

/// 单条命令元数据：名、语法、一句话说明
struct CmdMeta {
    name: &'static str,
    syntax: &'static str,
    summary: &'static str,
}

/// 常用 Redis 命令表（按类别）。覆盖高频命令，非穷尽 240+ 全集。
#[rustfmt::skip]
const COMMANDS: &[CmdMeta] = &[
    // 通用 / Key
    CmdMeta { name: "DEL", syntax: "DEL key [key ...]", summary: "删除 key" },
    CmdMeta { name: "EXISTS", syntax: "EXISTS key [key ...]", summary: "key 是否存在" },
    CmdMeta { name: "EXPIRE", syntax: "EXPIRE key seconds", summary: "设置过期秒数" },
    CmdMeta { name: "PEXPIRE", syntax: "PEXPIRE key ms", summary: "设置过期毫秒数" },
    CmdMeta { name: "TTL", syntax: "TTL key", summary: "剩余过期秒数" },
    CmdMeta { name: "PTTL", syntax: "PTTL key", summary: "剩余过期毫秒数" },
    CmdMeta { name: "PERSIST", syntax: "PERSIST key", summary: "移除过期时间" },
    CmdMeta { name: "TYPE", syntax: "TYPE key", summary: "key 的数据类型" },
    CmdMeta { name: "RENAME", syntax: "RENAME key newkey", summary: "重命名 key" },
    CmdMeta { name: "KEYS", syntax: "KEYS pattern", summary: "匹配 key（生产慎用）" },
    CmdMeta { name: "SCAN", syntax: "SCAN cursor [MATCH pat] [COUNT n] [TYPE t]", summary: "游标遍历 key" },
    CmdMeta { name: "RANDOMKEY", syntax: "RANDOMKEY", summary: "随机返回一个 key" },
    CmdMeta { name: "DUMP", syntax: "DUMP key", summary: "序列化 key" },
    CmdMeta { name: "OBJECT", syntax: "OBJECT ENCODING|IDLETIME|REFCOUNT key", summary: "查看 key 内部信息" },
    // String
    CmdMeta { name: "GET", syntax: "GET key", summary: "取 String 值" },
    CmdMeta { name: "SET", syntax: "SET key value [EX s|PX ms] [NX|XX]", summary: "设置 String 值" },
    CmdMeta { name: "SETEX", syntax: "SETEX key seconds value", summary: "设值 + 过期秒数" },
    CmdMeta { name: "SETNX", syntax: "SETNX key value", summary: "不存在才设值" },
    CmdMeta { name: "GETSET", syntax: "GETSET key value", summary: "设新值返旧值" },
    CmdMeta { name: "GETDEL", syntax: "GETDEL key", summary: "取值并删除" },
    CmdMeta { name: "APPEND", syntax: "APPEND key value", summary: "追加到 String 末尾" },
    CmdMeta { name: "STRLEN", syntax: "STRLEN key", summary: "String 长度" },
    CmdMeta { name: "INCR", syntax: "INCR key", summary: "自增 1" },
    CmdMeta { name: "DECR", syntax: "DECR key", summary: "自减 1" },
    CmdMeta { name: "INCRBY", syntax: "INCRBY key increment", summary: "按整数自增" },
    CmdMeta { name: "INCRBYFLOAT", syntax: "INCRBYFLOAT key increment", summary: "按浮点自增" },
    CmdMeta { name: "MGET", syntax: "MGET key [key ...]", summary: "批量取值" },
    CmdMeta { name: "MSET", syntax: "MSET key value [key value ...]", summary: "批量设值" },
    // Hash
    CmdMeta { name: "HGET", syntax: "HGET key field", summary: "取 Hash 字段值" },
    CmdMeta { name: "HSET", syntax: "HSET key field value [field value ...]", summary: "设 Hash 字段" },
    CmdMeta { name: "HMGET", syntax: "HMGET key field [field ...]", summary: "批量取字段" },
    CmdMeta { name: "HGETALL", syntax: "HGETALL key", summary: "取全部字段与值" },
    CmdMeta { name: "HDEL", syntax: "HDEL key field [field ...]", summary: "删字段" },
    CmdMeta { name: "HEXISTS", syntax: "HEXISTS key field", summary: "字段是否存在" },
    CmdMeta { name: "HKEYS", syntax: "HKEYS key", summary: "全部字段名" },
    CmdMeta { name: "HVALS", syntax: "HVALS key", summary: "全部字段值" },
    CmdMeta { name: "HLEN", syntax: "HLEN key", summary: "字段数量" },
    CmdMeta { name: "HINCRBY", syntax: "HINCRBY key field increment", summary: "字段整数自增" },
    CmdMeta { name: "HSCAN", syntax: "HSCAN key cursor [MATCH pat] [COUNT n]", summary: "游标遍历字段" },
    // List
    CmdMeta { name: "LPUSH", syntax: "LPUSH key element [element ...]", summary: "左侧入列" },
    CmdMeta { name: "RPUSH", syntax: "RPUSH key element [element ...]", summary: "右侧入列" },
    CmdMeta { name: "LPOP", syntax: "LPOP key [count]", summary: "左侧出列" },
    CmdMeta { name: "RPOP", syntax: "RPOP key [count]", summary: "右侧出列" },
    CmdMeta { name: "LRANGE", syntax: "LRANGE key start stop", summary: "区间取元素" },
    CmdMeta { name: "LLEN", syntax: "LLEN key", summary: "列表长度" },
    CmdMeta { name: "LINDEX", syntax: "LINDEX key index", summary: "按下标取元素" },
    CmdMeta { name: "LSET", syntax: "LSET key index element", summary: "按下标设元素" },
    CmdMeta { name: "LREM", syntax: "LREM key count element", summary: "按值删元素" },
    CmdMeta { name: "LTRIM", syntax: "LTRIM key start stop", summary: "裁剪保留区间" },
    // Set
    CmdMeta { name: "SADD", syntax: "SADD key member [member ...]", summary: "加成员" },
    CmdMeta { name: "SREM", syntax: "SREM key member [member ...]", summary: "删成员" },
    CmdMeta { name: "SMEMBERS", syntax: "SMEMBERS key", summary: "全部成员" },
    CmdMeta { name: "SISMEMBER", syntax: "SISMEMBER key member", summary: "是否为成员" },
    CmdMeta { name: "SCARD", syntax: "SCARD key", summary: "成员数量" },
    CmdMeta { name: "SPOP", syntax: "SPOP key [count]", summary: "随机弹出成员" },
    CmdMeta { name: "SINTER", syntax: "SINTER key [key ...]", summary: "交集" },
    CmdMeta { name: "SUNION", syntax: "SUNION key [key ...]", summary: "并集" },
    CmdMeta { name: "SDIFF", syntax: "SDIFF key [key ...]", summary: "差集" },
    CmdMeta { name: "SSCAN", syntax: "SSCAN key cursor [MATCH pat] [COUNT n]", summary: "游标遍历成员" },
    // Sorted Set
    CmdMeta { name: "ZADD", syntax: "ZADD key [NX|XX] score member [score member ...]", summary: "加带分成员" },
    CmdMeta { name: "ZREM", syntax: "ZREM key member [member ...]", summary: "删成员" },
    CmdMeta { name: "ZSCORE", syntax: "ZSCORE key member", summary: "取成员分数" },
    CmdMeta { name: "ZRANGE", syntax: "ZRANGE key start stop [WITHSCORES]", summary: "按排名取区间" },
    CmdMeta { name: "ZREVRANGE", syntax: "ZREVRANGE key start stop [WITHSCORES]", summary: "倒序取区间" },
    CmdMeta { name: "ZRANGEBYSCORE", syntax: "ZRANGEBYSCORE key min max", summary: "按分数取区间" },
    CmdMeta { name: "ZRANK", syntax: "ZRANK key member", summary: "成员排名" },
    CmdMeta { name: "ZCARD", syntax: "ZCARD key", summary: "成员数量" },
    CmdMeta { name: "ZINCRBY", syntax: "ZINCRBY key increment member", summary: "成员分数自增" },
    CmdMeta { name: "ZSCAN", syntax: "ZSCAN key cursor [MATCH pat] [COUNT n]", summary: "游标遍历成员" },
    // Stream
    CmdMeta { name: "XADD", syntax: "XADD key ID field value [field value ...]", summary: "追加流条目" },
    CmdMeta { name: "XLEN", syntax: "XLEN key", summary: "流长度" },
    CmdMeta { name: "XRANGE", syntax: "XRANGE key start end [COUNT n]", summary: "区间取条目" },
    CmdMeta { name: "XREVRANGE", syntax: "XREVRANGE key end start [COUNT n]", summary: "倒序取条目" },
    CmdMeta { name: "XDEL", syntax: "XDEL key ID [ID ...]", summary: "删条目" },
    CmdMeta { name: "XREAD", syntax: "XREAD [COUNT n] STREAMS key ID", summary: "读取流" },
    // Server / Connection
    CmdMeta { name: "PING", syntax: "PING [message]", summary: "心跳探测" },
    CmdMeta { name: "ECHO", syntax: "ECHO message", summary: "回显消息" },
    CmdMeta { name: "SELECT", syntax: "SELECT index", summary: "切换 DB" },
    CmdMeta { name: "DBSIZE", syntax: "DBSIZE", summary: "当前 DB key 数" },
    CmdMeta { name: "INFO", syntax: "INFO [section]", summary: "服务器信息" },
    CmdMeta { name: "CONFIG", syntax: "CONFIG GET|SET parameter [value]", summary: "读写配置" },
    CmdMeta { name: "CLIENT", syntax: "CLIENT LIST|INFO|GETNAME|SETNAME ...", summary: "客户端管理" },
    CmdMeta { name: "COMMAND", syntax: "COMMAND DOCS|COUNT|INFO ...", summary: "命令元信息" },
    CmdMeta { name: "MEMORY", syntax: "MEMORY USAGE key | DOCTOR | STATS", summary: "内存诊断" },
    CmdMeta { name: "FLUSHDB", syntax: "FLUSHDB [ASYNC]", summary: "清空当前 DB（危险）" },
    CmdMeta { name: "FLUSHALL", syntax: "FLUSHALL [ASYNC]", summary: "清空所有 DB（危险）" },
];

pub struct RedisCompletionProvider;

impl RedisCompletionProvider {
    pub fn new_rc() -> Rc<dyn CompletionProvider> {
        Rc::new(Self)
    }
}

impl CompletionProvider for RedisCompletionProvider {
    fn completions(
        &self,
        rope: &Rope,
        offset: usize,
        _trigger: CompletionContext,
        _window: &mut Window,
        _cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>> {
        let text = rope.to_string();
        let bytes = text.as_bytes();
        let real_offset = offset.min(bytes.len());

        // 光标前的命令字（字母/数字）
        let mut start = real_offset;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric()) {
            start -= 1;
        }
        let prefix = &text[start..real_offset];

        // 仅命令位补全：前缀之前须全是空白（否则是参数位，不补）
        if prefix.is_empty() || !text[..start].trim().is_empty() {
            return Task::ready(Ok(CompletionResponse::Array(vec![])));
        }

        let prefix_upper = prefix.to_ascii_uppercase();
        let replace_range = lsp_types::Range::new(
            rope.offset_to_position(start),
            rope.offset_to_position(real_offset),
        );

        let items: Vec<CompletionItem> = COMMANDS
            .iter()
            .filter(|c| c.name.starts_with(&prefix_upper))
            .map(|c| make_item(c, replace_range))
            .collect();

        Task::ready(Ok(CompletionResponse::Array(items)))
    }

    fn is_completion_trigger(
        &self,
        _offset: usize,
        new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        // 字母 / 数字触发（命令名只含字母数字）
        new_text.chars().all(|c| c.is_ascii_alphanumeric())
    }
}

/// 命令名 → CompletionItem。语法只放 documentation（选中时在侧边文档面板显示，可读）；
/// 不设内联 detail——选中项底色是实色 accent，灰字 detail 压上去看不清
fn make_item(cmd: &CmdMeta, range: lsp_types::Range) -> CompletionItem {
    CompletionItem {
        label: cmd.name.to_string(),
        kind: Some(CompletionItemKind::FUNCTION),
        detail: None,
        documentation: Some(Documentation::MarkupContent(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("**{}**\n\n`{}`\n\n{}", cmd.name, cmd.syntax, cmd.summary),
        })),
        text_edit: Some(CompletionTextEdit::InsertAndReplace(InsertReplaceEdit {
            new_text: cmd.name.to_string(),
            insert: range,
            replace: range,
        })),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commands_uppercase_and_sorted_within_use() {
        // 名称应全大写（补全插入大写命令名）
        for c in COMMANDS {
            assert_eq!(
                c.name,
                c.name.to_ascii_uppercase(),
                "命令名须大写: {}",
                c.name
            );
        }
    }

    #[test]
    fn has_common_commands() {
        let names: Vec<&str> = COMMANDS.iter().map(|c| c.name).collect();
        for must in ["GET", "SET", "HGETALL", "KEYS", "CONFIG", "PING"] {
            assert!(names.contains(&must), "缺常用命令: {must}");
        }
    }
}
