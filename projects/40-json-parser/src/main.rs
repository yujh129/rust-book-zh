//! 第 40 章项目二：手写 JSON 解析器
//!
//! 按《rust-book》src/part7/ch40-json-parser.md 正文的演进顺序组装，只用标准库，
//! 无任何第三方依赖。章节末尾「典型错误案例」中的 5 段代码是故意写错的示例，
//! 不属于最终实现，未收录。

use std::fmt;

// ---------------------------------------------------------------------------
// 40.2 用 enum 建模
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum JsonValue {
    Null,
    Bool(bool),
    Number(JsonNumber),
    String(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

#[derive(Debug, Clone, PartialEq)]
enum JsonNumber {
    Int(i64),
    Float(f64),
    Raw(String),
}

impl fmt::Display for JsonNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonNumber::Int(i) => write!(f, "{}", i),
            JsonNumber::Raw(s) => f.write_str(s),
            JsonNumber::Float(x) => {
                let s = format!("{}", x);
                if s.contains(['.', 'e', 'E']) {
                    f.write_str(&s)
                } else {
                    write!(f, "{}.0", s)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 40.3 手写词法分析器
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
struct ParseError {
    pos: usize,
    kind: ErrorKind,
}

#[derive(Debug, Clone, PartialEq)]
enum ErrorKind {
    UnexpectedEof,
    UnexpectedByte(u8),
    ExpectedValue,
    ExpectedKey,
    ExpectedColon,
    ExpectedCommaOrEnd,
    InvalidNumber,
    NumberOutOfRange,
    InvalidEscape,
    InvalidUnicode,
    ControlCharInString,
    TrailingCharacters,
    DepthLimit,
}

impl fmt::Display for ErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorKind::UnexpectedEof => f.write_str("输入意外结束"),
            ErrorKind::UnexpectedByte(b) => write!(f, "偏移处有不认识的字节 0x{:02x}", b),
            ErrorKind::ExpectedValue => f.write_str("这里应该是一个值"),
            ErrorKind::ExpectedKey => f.write_str("对象的键必须是字符串"),
            ErrorKind::ExpectedColon => f.write_str("键后面缺少冒号"),
            ErrorKind::ExpectedCommaOrEnd => f.write_str("缺少逗号或右括号"),
            ErrorKind::InvalidNumber => f.write_str("数字格式不合法"),
            ErrorKind::NumberOutOfRange => f.write_str("数字超出 f64 能表示的范围"),
            ErrorKind::InvalidEscape => f.write_str("非法的转义序列"),
            ErrorKind::InvalidUnicode => f.write_str("非法的 \\u 转义"),
            ErrorKind::ControlCharInString => f.write_str("字符串里有未转义的控制字符"),
            ErrorKind::TrailingCharacters => f.write_str("值后面还有多余内容"),
            ErrorKind::DepthLimit => f.write_str("嵌套层数超过上限"),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "字节偏移 {}：{}", self.pos, self.kind)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    Null,
    True,
    False,
    Number(&'a str),
    Str(&'a str),
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Colon,
    Comma,
    Eof,
}

#[derive(Debug, Clone, Copy)]
struct Spanned<'a> {
    token: Token<'a>,
    pos: usize,
}

struct Tokenizer<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(src: &'a str) -> Self {
        Self { src, bytes: src.as_bytes(), pos: 0 }
    }

    fn skip_ws(&mut self) {
        while let Some(b' ' | b'\t' | b'\n' | b'\r') = self.bytes.get(self.pos) {
            self.pos += 1;
        }
    }

    fn next(&mut self) -> Result<Spanned<'a>, ParseError> {
        self.skip_ws();
        let start = self.pos;
        let b = match self.bytes.get(self.pos) {
            Some(&b) => b,
            None => return Ok(Spanned { token: Token::Eof, pos: start }),
        };
        self.pos += 1;
        let token = match b {
            b'{' => Token::LBrace,
            b'}' => Token::RBrace,
            b'[' => Token::LBracket,
            b']' => Token::RBracket,
            b':' => Token::Colon,
            b',' => Token::Comma,
            b'"' => Token::Str(self.scan_string(start)?),
            b'-' | b'0'..=b'9' => Token::Number(self.scan_number(start)?),
            b'n' => {
                self.expect_word(b"ull", start)?;
                Token::Null
            }
            b't' => {
                self.expect_word(b"rue", start)?;
                Token::True
            }
            b'f' => {
                self.expect_word(b"alse", start)?;
                Token::False
            }
            other => {
                return Err(ParseError { pos: start, kind: ErrorKind::UnexpectedByte(other) })
            }
        };
        Ok(Spanned { token, pos: start })
    }

    fn expect_word(&mut self, rest: &[u8], start: usize) -> Result<(), ParseError> {
        if self.bytes[self.pos..].starts_with(rest) {
            self.pos += rest.len();
            Ok(())
        } else {
            Err(ParseError { pos: start, kind: ErrorKind::UnexpectedByte(self.bytes[start]) })
        }
    }

    fn scan_string(&mut self, start: usize) -> Result<&'a str, ParseError> {
        self.pos = start + 1;
        loop {
            match self.bytes.get(self.pos) {
                None => {
                    return Err(ParseError { pos: start, kind: ErrorKind::UnexpectedEof })
                }
                Some(b'"') => {
                    let text = &self.src[start + 1..self.pos];
                    self.pos += 1;
                    return Ok(text);
                }
                Some(b'\\') => self.pos += 2,
                Some(&b) if b < 0x20 => {
                    return Err(ParseError {
                        pos: self.pos,
                        kind: ErrorKind::ControlCharInString,
                    })
                }
                Some(_) => self.pos += 1,
            }
        }
    }

    fn scan_number(&mut self, start: usize) -> Result<&'a str, ParseError> {
        self.pos = start;
        while let Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') =
            self.bytes.get(self.pos)
        {
            self.pos += 1;
        }
        let lit = &self.src[start..self.pos];
        if is_valid_number(lit) {
            Ok(lit)
        } else {
            Err(ParseError { pos: start, kind: ErrorKind::InvalidNumber })
        }
    }
}

fn is_valid_number(lit: &str) -> bool {
    let b = lit.as_bytes();
    let mut i = 0;
    if b.get(i) == Some(&b'-') {
        i += 1;
    }
    match b.get(i) {
        Some(b'0') => i += 1,
        Some(b'1'..=b'9') => {
            i += 1;
            while matches!(b.get(i), Some(b'0'..=b'9')) {
                i += 1;
            }
        }
        _ => return false,
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        if !matches!(b.get(i), Some(b'0'..=b'9')) {
            return false;
        }
        while matches!(b.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
    }
    if matches!(b.get(i), Some(b'e') | Some(b'E')) {
        i += 1;
        if matches!(b.get(i), Some(b'+') | Some(b'-')) {
            i += 1;
        }
        if !matches!(b.get(i), Some(b'0'..=b'9')) {
            return false;
        }
        while matches!(b.get(i), Some(b'0'..=b'9')) {
            i += 1;
        }
    }
    i == b.len()
}

// ---------------------------------------------------------------------------
// 40.4 递归下降解析器
// ---------------------------------------------------------------------------

const MAX_DEPTH: usize = 128;

struct Parser<'a> {
    lexer: Tokenizer<'a>,
    current: Spanned<'a>,
    depth: usize,
}

impl<'a> Parser<'a> {
    fn new(src: &'a str) -> Result<Self, ParseError> {
        let mut lexer = Tokenizer::new(src);
        let current = lexer.next()?;
        Ok(Self { lexer, current, depth: 0 })
    }

    fn bump(&mut self) -> Result<(), ParseError> {
        self.current = self.lexer.next()?;
        Ok(())
    }

    fn parse(mut self) -> Result<JsonValue, ParseError> {
        let value = self.parse_value()?;
        if self.current.token != Token::Eof {
            return Err(ParseError {
                pos: self.current.pos,
                kind: ErrorKind::TrailingCharacters,
            });
        }
        Ok(value)
    }

    fn parse_value(&mut self) -> Result<JsonValue, ParseError> {
        match self.current.token {
            Token::Null => {
                self.bump()?;
                Ok(JsonValue::Null)
            }
            Token::True => {
                self.bump()?;
                Ok(JsonValue::Bool(true))
            }
            Token::False => {
                self.bump()?;
                Ok(JsonValue::Bool(false))
            }
            Token::Number(lit) => {
                let n = parse_number(lit, self.current.pos)?;
                self.bump()?;
                Ok(JsonValue::Number(n))
            }
            Token::Str(raw) => {
                let s = parse_string(raw, self.current.pos)?;
                self.bump()?;
                Ok(JsonValue::String(s))
            }
            Token::LBracket => self.parse_array(),
            Token::LBrace => self.parse_object(),
            _ => Err(ParseError { pos: self.current.pos, kind: ErrorKind::ExpectedValue }),
        }
    }

    fn enter(&mut self) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(ParseError { pos: self.current.pos, kind: ErrorKind::DepthLimit });
        }
        Ok(())
    }

    fn parse_array(&mut self) -> Result<JsonValue, ParseError> {
        self.enter()?;
        self.bump()?;
        let mut items = Vec::new();
        if self.current.token == Token::RBracket {
            self.bump()?;
            self.depth -= 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            items.push(self.parse_value()?);
            match self.current.token {
                Token::Comma => self.bump()?,
                Token::RBracket => {
                    self.bump()?;
                    break;
                }
                _ => {
                    return Err(ParseError {
                        pos: self.current.pos,
                        kind: ErrorKind::ExpectedCommaOrEnd,
                    })
                }
            }
        }
        self.depth -= 1;
        Ok(JsonValue::Array(items))
    }

    fn parse_object(&mut self) -> Result<JsonValue, ParseError> {
        self.enter()?;
        self.bump()?;
        let mut fields = Vec::new();
        if self.current.token == Token::RBrace {
            self.bump()?;
            self.depth -= 1;
            return Ok(JsonValue::Object(fields));
        }
        loop {
            let key = match self.current.token {
                Token::Str(raw) => parse_string(raw, self.current.pos)?,
                _ => {
                    return Err(ParseError { pos: self.current.pos, kind: ErrorKind::ExpectedKey })
                }
            };
            self.bump()?;
            if self.current.token != Token::Colon {
                return Err(ParseError {
                    pos: self.current.pos,
                    kind: ErrorKind::ExpectedColon,
                });
            }
            self.bump()?;
            let value = self.parse_value()?;
            fields.push((key, value));
            match self.current.token {
                Token::Comma => self.bump()?,
                Token::RBrace => {
                    self.bump()?;
                    break;
                }
                _ => {
                    return Err(ParseError {
                        pos: self.current.pos,
                        kind: ErrorKind::ExpectedCommaOrEnd,
                    })
                }
            }
        }
        self.depth -= 1;
        Ok(JsonValue::Object(fields))
    }
}

fn parse_number(lit: &str, pos: usize) -> Result<JsonNumber, ParseError> {
    if !lit.contains(['.', 'e', 'E']) {
        return match lit.parse::<i64>() {
            Ok(i) => Ok(JsonNumber::Int(i)),
            Err(_) => Ok(JsonNumber::Raw(lit.to_string())),
        };
    }
    let f: f64 = lit.parse().map_err(|_| ParseError { pos, kind: ErrorKind::InvalidNumber })?;
    if !f.is_finite() {
        return Err(ParseError { pos, kind: ErrorKind::NumberOutOfRange });
    }
    Ok(JsonNumber::Float(f))
}

fn parse_string(raw: &str, pos: usize) -> Result<String, ParseError> {
    let base = pos + 1;
    if !raw.contains('\\') {
        return Ok(raw.to_string());
    }
    let bytes = raw.as_bytes();
    let mut out = String::with_capacity(raw.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'\\' {
            let ch = raw[i..].chars().next().expect("i 在字符边界上");
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        i += 1;
        let esc = *bytes
            .get(i)
            .ok_or(ParseError { pos: base + i, kind: ErrorKind::InvalidEscape })?;
        i += 1;
        match esc {
            b'"' => out.push('"'),
            b'\\' => out.push('\\'),
            b'/' => out.push('/'),
            b'b' => out.push('\u{0008}'),
            b'f' => out.push('\u{000c}'),
            b'n' => out.push('\n'),
            b'r' => out.push('\r'),
            b't' => out.push('\t'),
            b'u' => {
                let (ch, next) = read_unicode_escape(bytes, i, base)?;
                out.push(ch);
                i = next;
            }
            _ => return Err(ParseError { pos: base + i - 1, kind: ErrorKind::InvalidEscape }),
        }
    }
    Ok(out)
}

fn read_hex4(bytes: &[u8], at: usize, base: usize) -> Result<u16, ParseError> {
    let mut value: u16 = 0;
    for k in 0..4 {
        let b = *bytes
            .get(at + k)
            .ok_or(ParseError { pos: base + at + k, kind: ErrorKind::UnexpectedEof })?;
        let digit = (b as char).to_digit(16).ok_or(ParseError {
            pos: base + at + k,
            kind: ErrorKind::InvalidUnicode,
        })?;
        value = value * 16 + digit as u16;
    }
    Ok(value)
}

fn read_unicode_escape(bytes: &[u8], at: usize, base: usize) -> Result<(char, usize), ParseError> {
    let bad = |pos: usize| ParseError { pos, kind: ErrorKind::InvalidUnicode };
    let first = read_hex4(bytes, at, base)?;
    match first {
        0xD800..=0xDBFF => {
            if bytes.get(at + 4) != Some(&b'\\') || bytes.get(at + 5) != Some(&b'u') {
                return Err(bad(base + at));
            }
            let second = read_hex4(bytes, at + 6, base)?;
            if !(0xDC00..=0xDFFF).contains(&second) {
                return Err(bad(base + at + 6));
            }
            let code = 0x10000 + ((first as u32) - 0xD800) * 0x400 + ((second as u32) - 0xDC00);
            let ch = char::from_u32(code).ok_or_else(|| bad(base + at))?;
            Ok((ch, at + 10))
        }
        0xDC00..=0xDFFF => Err(bad(base + at)),
        _ => {
            let ch = char::from_u32(first as u32).ok_or_else(|| bad(base + at))?;
            Ok((ch, at + 4))
        }
    }
}

// ---------------------------------------------------------------------------
// 40.6 序列化回字符串
// ---------------------------------------------------------------------------

impl JsonValue {
    fn to_json(&self) -> String {
        let mut out = String::new();
        self.write_json(&mut out);
        out
    }

    fn write_json(&self, out: &mut String) {
        match self {
            JsonValue::Null => out.push_str("null"),
            JsonValue::Bool(true) => out.push_str("true"),
            JsonValue::Bool(false) => out.push_str("false"),
            JsonValue::Number(n) => out.push_str(&n.to_string()),
            JsonValue::String(s) => write_json_string(s, out),
            JsonValue::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_json(out);
                }
                out.push(']');
            }
            JsonValue::Object(fields) => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_json_string(key, out);
                    out.push(':');
                    value.write_json(out);
                }
                out.push('}');
            }
        }
    }

    // 40.8：pretty 输出的入口（章节只给了 write_pretty，这里补一个返回 String 的包装）。
    fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write_pretty(0, &mut out);
        out
    }

    fn write_pretty(&self, indent: usize, out: &mut String) {
        match self {
            JsonValue::Array(items) if !items.is_empty() => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    out.push('\n');
                    pad(indent + 1, out);
                    item.write_pretty(indent + 1, out);
                }
                out.push('\n');
                pad(indent, out);
                out.push(']');
            }
            JsonValue::Object(fields) if !fields.is_empty() => {
                out.push('{');
                for (i, (key, value)) in fields.iter().enumerate() {
                    if i > 0 { out.push(','); }
                    out.push('\n');
                    pad(indent + 1, out);
                    write_json_string(key, out);
                    out.push_str(": ");
                    value.write_pretty(indent + 1, out);
                }
                out.push('\n');
                pad(indent, out);
                out.push('}');
            }
            JsonValue::Array(_) => out.push_str("[]"),
            JsonValue::Object(_) => out.push_str("{}"),
            JsonValue::Null => out.push_str("null"),
            JsonValue::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            JsonValue::Number(x) => out.push_str(&x.to_string()),
            JsonValue::String(s) => write_json_string(s, out),
        }
    }
}

fn pad(level: usize, out: &mut String) {
    for _ in 0..level * 2 { out.push(' '); }
}

fn write_json_string(s: &str, out: &mut String) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

// ---------------------------------------------------------------------------
// 40.4 的 main：章节正文的入口（附带 40.8/练习 4 的 --pretty 开关）
// ---------------------------------------------------------------------------

fn main() {
    let mut input: Option<String> = None;
    let mut pretty = false;
    for arg in std::env::args().skip(1) {
        if arg == "--pretty" {
            pretty = true;
        } else if input.is_none() {
            input = Some(arg);
        }
    }
    let src = input.unwrap_or_else(|| {
        String::from(r#"{"name":"Ana","age":30,"tags":["rust","json"],"score":9.5}"#)
    });
    match Parser::new(&src).and_then(|p| p.parse()) {
        Ok(value) => {
            if pretty {
                println!("{}", value.to_pretty());
            } else {
                println!("{}", value.to_json());
            }
        }
        Err(err) => println!("解析失败：{}", err),
    }
}

// ---------------------------------------------------------------------------
// 40.7 单元测试
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    fn parse(src: &str) -> Result<JsonValue, ParseError> {
        Parser::new(src)?.parse()
    }
    #[test]
    fn parses_scalars() {
        assert_eq!(parse("null").unwrap(), JsonValue::Null);
        assert_eq!(parse(" true ").unwrap(), JsonValue::Bool(true));
        assert_eq!(parse("-42").unwrap(), JsonValue::Number(JsonNumber::Int(-42)));
        assert_eq!(parse("\"hi\"").unwrap(), JsonValue::String("hi".to_string()));
    }
    #[test]
    fn parses_nested_and_round_trips() {
        let src = r#"{"a":[1,2,{"b":null}],"c":"x"}"#;
        let value = parse(src).unwrap();
        assert_eq!(value.to_json(), src);
    }
    #[test]
    fn decodes_escapes_and_surrogates() {
        let value = parse(r#""line\n\t\u4f60\u597d\uD83D\uDE00""#).unwrap();
        assert_eq!(value, JsonValue::String("line\n\t你好😀".to_string()));
    }
    #[test]
    fn keeps_big_integers_exact() {
        let value = parse("18446744073709551615").unwrap();
        assert_eq!(value.to_json(), "18446744073709551615");
    }
    #[test]
    fn reports_byte_offset() {
        let err = parse("{\"a\": 1,}").unwrap_err();
        assert_eq!(err.kind, ErrorKind::ExpectedKey);
        assert_eq!(err.pos, 8);
    }
    #[test]
    fn rejects_deep_nesting() {
        let src = "[".repeat(200) + &"]".repeat(200);
        let err = parse(&src).unwrap_err();
        assert_eq!(err.kind, ErrorKind::DepthLimit);
    }
}
