# 第 40 章 项目二：手写 JSON 解析器

> **本章目标**
>
> - 把 RFC 8259 定义的六种 JSON 值翻译成一个 Rust 枚举，并为每个变体选对载荷类型
> - 手写词法分析器与递归下降解析器，让错误报告精确到字节偏移
> - 用 `Result<T, ParseError>` 把「哪里错了」一路带到调用者面前
> - 解释为什么不能把所有数字都塞进 `f64`，以及 `JsonNumber` 的三个变体各管什么
> - 把解析树写回 JSON 文本，正确处理引号、反斜杠、换行与 `\u` 转义
> - 用单元测试覆盖正常与异常路径，并看清递归深度与所有权这两个真实陷阱

前面的项目处理的是「行」和「字节」。这个项目处理的是**结构**：一段文本进去，一棵树出来，再变回文本。

JSON 只定义了六种值，文法小到可以印在一页纸上。正因为它小，适合拿来练三件事：用枚举给数据建模、把错误位置一路传出去、以及在没有第三方 crate 的情况下处理 UTF-8 和数字精度。写完这一章再看 serde_json 的源码，你会认出每一个部件。

## 40.1 需求与 JSON 语法

先定验收标准。RFC 8259 规定，一个 JSON 文本就是下面六种值之一：

```text
value   = null | true | false | number | string | array | object
object  = "{" [ member { "," member } ] "}"
member  = string ":" value
array   = "[" [ value { "," value } ] "]"
string  = '"' { 普通字符 | escape } '"'
escape  = "\" ( '"' | "\" | "/" | "b" | "f" | "n" | "r" | "t" | "u" 4位十六进制 )
number  = [ "-" ] int [ frac ] [ exp ]
int     = "0" | 非零数字 { 数字 }
frac    = "." 1*数字
exp     = ("e" | "E") [ "+" | "-" ] 1*数字
空白    = 空格 | 制表符 | 换行 | 回车
```

把要求写成两条可执行的验收：

```console
$ cargo run -- '{"name":"Ana","tags":["rust","json"],"score":9.5}'
{"name":"Ana","tags":["rust","json"],"score":9.5}

$ cargo run -- '{"a": 1,}'
解析失败：字节偏移 8：对象的键必须是字符串
```

第一条叫往返（round trip）：把同一份 JSON 解析再序列化，结构、键值关系与字符串内容保持一致。它**不是逐字节还原**——空白会被重排，数字会被规范化（`1e3` 输出成 `1000.0`，`-0` 输出成 `0`），各类 JSON 库都如此。第二条要求失败时给出**字节偏移**，而不是一句「解析失败」。配置文件动辄几百 KB，没有位置信息的报错等于没报。

项目只用标准库，`Cargo.toml` 里连 `[dependencies]` 都不需要：

```toml
[package]
name = "jsonp"
version = "0.1.0"
edition = "2021"
```

## 40.2 用 enum 建模

六种值，六个变体，一一对应：

```rust
use std::fmt;

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

fn main() {
    let value = JsonValue::Object(vec![
        ("name".to_string(), JsonValue::String("Ana".to_string())),
        ("age".to_string(), JsonValue::Number(JsonNumber::Int(30))),
        (
            "tags".to_string(),
            JsonValue::Array(vec![JsonValue::String("rust".to_string())]),
        ),
    ]);
    println!("{:?}", value);
    println!("JsonValue 占 {} 字节", std::mem::size_of::<JsonValue>());
}
```

几个决定值得解释。

**对象为什么用 `Vec<(String, JsonValue)>`**：JSON 规范不要求键唯一，也不保证顺序，但真实世界的配置文件往往希望「读进来再写回去」时保持原顺序，测试也好写。`Vec` 保留顺序、允许重复键，代价是查找退化成线性扫描；要 O(1) 查找就换 `HashMap`，但那会丢掉顺序。本项目选 `Vec`。

**为什么需要 `Clone` 与 `PartialEq`**：测试要比较两棵树是否相等，所以需要 `PartialEq`；解析器有时要从 token 里取出字符串，`Clone` 让复制值这件事显式且便宜。`Debug` 用于打印。

**`JsonValue` 有多大**：`String` 与 `Vec` 在 64 位平台上都是 24 字节，加上 8 字节的判别式，这个枚举占 32 字节。枚举的大小由**最大**变体决定，所以只装一个 `bool` 的 `JsonValue::Bool` 也要占满 32 字节。这是用枚举建模的内存代价，换来的是「一个值只能是六种之一」的类型保证。

`JsonNumber` 的三个变体留到 40.5 展开。现在只需要知道：整数、浮点数、放不进 `i64` 的大整数，走三条不同的路。

## 40.3 手写词法分析器

解析器分两层。词法分析（lexer，也叫 tokenizer）把字节流切成 token，语法分析（parser）在 token 序列上做递归下降。分层的意义是每一层只操心一件事：词法层回答「`"ab\\"c"` 是一个字符串」，语法层回答「这个字符串该出现在哪里」。

先定义两层共用的错误类型：

```rust,noplayground
use std::fmt;

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
```

`pos` 是出错处的字节偏移，`kind` 是机器可判定的类别。为什么 `kind` 不直接存 `String`？因为测试要断言 `err.kind == ErrorKind::ExpectedKey`，比较枚举比匹配字符串稳得多；至于中文句子，交给 `Display` 翻译一次就够了，报错、日志与测试断言三处共用。

`impl std::error::Error for ParseError` 一行方法都不用写。它的价值在于让 `ParseError` 满足 `?` 需要的错误接口，也能被 `Box<dyn Error>` 装下。

token 的定义里带一个生命周期参数：

```rust,noplayground
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
```

`Number(&'a str)` 与 `Str(&'a str)` 指向输入里的切片，词法分析阶段一次分配都没有。`Token` 能派生出 `Copy`，因为引用是 `Copy` 的，于是 `self.current.token` 可以直接读出来比较，不会把 token 移走。

`Spanned` 把 token 和它的起始字节偏移绑在一起。parser 要报错时就读 `current.pos`，它指向这个 token 的第一个字节。

主循环如下：

```rust,noplayground
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
}
```

`next` 返回 `Result<Spanned, ParseError>`，输入耗尽时给 `Token::Eof` 而不是 `None`。多一个哨兵 token，parser 就不用到处处理 `Option`：`Eof` 与别的 token 一样参与相等比较，`bump()` 也不必区分「还有下一个」和「没有了」。

`expect_word` 检查关键字的后半截：读到 `n` 之后还要 `ull`。不匹配时，报错位置用的是 `n` 的偏移而不是 `u` 的偏移，因为用户需要知道这个词**从哪开始**不对。`self.bytes[self.pos..]` 直接切片而不用 `get`，是因为 `starts_with` 自己会处理越界：短于 `rest` 的切片只会返回 `false`，不会 panic。

空白只在 token 之间跳。`skip_ws` 只认四个字符，RFC 8259 的空白就是这四个；垂直制表符 `\x0b` 会让解析失败，这是规范要求的。

### 字节偏移不是字符下标

`pos` 是字节偏移。这一点要单独强调，因为你迟早会写 `&src[..err.pos]`，想给报错附一段上下文，然后发现它在运行期炸掉：

```rust,noplayground
fn main() {
    let src = "你好";
    let pos = 1;
    let head = &src[..pos];
    println!("{}", head);
}
```

```text
thread 'main' (34948) panicked at src/main.rs:4:20:
end byte index 1 is not a char boundary; it is inside '你' (bytes 0..3 of string)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

「你」占 3 个字节，`pos = 1` 落在它中间，Rust 拒绝切出一个非法的 UTF-8 字符串，于是 panic。想安全地取上下文，就先用 `char_indices()` 找到最近的边界，或者用 `str::is_char_boundary` 先判断再切；错误信息里已经带了字节偏移，调试时不必切字符串也能定位。

### 字符串与数字的扫描

字符串扫描只负责找到结尾引号，转义序列的原样保留：

```rust,noplayground
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
```

遇到反斜杠就跳过下一个字节，因为 `"a\\"b"` 里那个被转义的引号不是字符串的结尾；`\\u0041` 的四位十六进制也顺带被跳过。返回值是字符串的**原始内容**，转义还留在里面，解码是 parser 的事。小于 `0x20` 的字节直接拒绝：RFC 8259 要求控制字符必须转义。

细看 `self.pos += 2` 有一个小瑕疵。如果输入以反斜杠结尾，`pos` 会越过 `len()` 变成 `len + 1`，下一轮 `self.bytes.get(self.pos)` 返回 `None`，报 `UnexpectedEof`。结果是对的，过程不优雅；要写得严谨就先检查 `self.bytes.get(self.pos + 1)` 是否存在。能跑对但靠边界兜底的地方，注释里应当写明白。

数字扫描反过来：先把所有可能属于数字的字节全部吃进来，再整体验证：

```rust,noplayground
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
```

先吃后验的顺序很重要。如果一碰到不属于数字的字符就停，`1.2.3` 会被切成 `1.2` 和 `.3` 两个 token，报错位置跑到第二个点后面，用户看到的上下文是错的。现在整个 `1.2.3` 被当成一个候选字面量，校验失败的位置就是数字的起点。

校验函数是 RFC 文法的逐条翻译：

```rust,noplayground
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
```

整数部分匹配到 `0` 就 `i += 1` 并立即离开 `match`，不进入循环，所以 `01` 在第一个 `0` 之后还剩 `1`，最后的 `i == b.len()` 为假，被拒绝。这就是 RFC 里 `int = "0" | 非零数字 { 数字 }` 的字面翻译。所有访问都走 `b.get(i)`：越界返回 `None`，`matches!` 得到 `false`，函数返回而不 panic。

## 40.4 递归下降解析器

递归下降（recursive descent）的做法是：每个非终结符对应一个函数，函数看一眼当前 token 就知道该走哪条产生式，然后调用别的函数。JSON 的文法只有一层嵌套（值里可以再出现数组和对象），所以函数之间会互相递归。

解析器持有三样东西：词法分析器、预读到的当前 token、以及嵌套深度。

```rust,noplayground
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
}
```

预读一个 token（lookahead 1）就够用了：每个函数先看 `self.current.token`，匹配上就干活，干完用一个 `bump` 前进。`Token` 是 `Copy`，所以 `self.current.token` 能被直接复制出来匹配，不会因为借用 `self` 而和 `self.bump()` 打架——这一点在 40.9 会详细说。

`parse` 接收 `self` 而不是 `&mut self`，读完值之后必须检查是不是 `Eof`。`{"a":1} garbage` 应当在偏移 8 处报 `TrailingCharacters`，而不是静默返回前面那个对象。很多手写解析器的 bug 都出在这里：只解析了前缀，却当作成功。

`parse_value` 是分发器，它决定了「这个位置可以是什么」：

```rust,noplayground
const MAX_DEPTH: usize = 128;

impl<'a> Parser<'a> {
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
```

前四个分支的形状完全一样：取出载荷、`bump` 前进、包装成 `JsonValue`。数字和字符串分支值得多看一眼，它们把原始切片和位置交给了自由函数 `parse_number` 与 `parse_string`，而不是写成 `self.parse_number()`。原因是这两个操作根本不需要解析器状态：输入是一个 `&str`，输出是解码后的值或一个错误。写成自由函数，借用关系更简单，测试也更容易单独调用。

`_` 分支收下所有不该出现在值位置上的 token：`Eof`、`RBrace`、`Comma` 等等，统一报 `ExpectedValue`。报错位置就是当前 token 的起点。

`enter` 每次进入数组或对象时把 `depth` 加一，超过 `MAX_DEPTH` 就返回 `DepthLimit`。上限不是性能优化，是安全边界：没有它，一层一层嵌套的 `[[[[` 会把调用栈耗尽。40.9 会给出真实的栈溢出输出。

数组的解析是这套结构里最典型的循环：

```rust,noplayground
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
}
```

进入时当前 token 一定是 `[`，先 `bump` 到第一个元素。空数组 `[]` 需要单独判断，因为通用循环假定「至少有一个值」；不特判的话，`parse_value` 会看到 `]` 并报 `ExpectedValue`，而这个位置其实完全合法。

循环体先解析一个值，再看分隔符。如果是逗号就前进，然后回到循环开头继续解析——注意这里**没有**「逗号后不能再有值」的特判，于是 `[1,]` 会在逗号之后让 `parse_value` 看到 `]`，报 `ExpectedValue`，位置正好在 `]` 上。错误信息自然地指向了多余的那个逗号。如果是右括号就收尾，其他情况（比如 `[1 2]`）报 `ExpectedCommaOrEnd`。

`depth -= 1` 出现两次，因为提前 return 的空数组分支绕过了循环之后的递减。这种「成对操作散落在多个出口」的写法容易漏，把这四条路径都写进测试是值得的。

对象比数组多两个约束：键必须是字符串，键和值之间必须有冒号。

```rust,noplayground
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
```

键也要走 `parse_string` 解码，因为 `{"a\\nb":1}` 里的 `\\n` 是换行转义，键必须还原成真实的换行符再存进 `Vec`。重复的键在这里被原样保留，两个同名键 `("a", 1)` 与 `("a", 2)` 会先后进 `Vec`；要不要去重是使用者的策略，解析器不擅自决定。

回到 40.1 的验收例子 `{"a": 1,}`：逗号让循环回到开头，当前 token 是 `}`，键的 `match` 匹配到 `_`，于是报 `ExpectedKey`，位置是 `}` 的字节偏移 8。输出正是「解析失败：字节偏移 8：对象的键必须是字符串」。

数字的处理分三级：

```rust,noplayground
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
```

字面量里没有小数点也没有指数，就当整数用 `i64` 解析。解析失败只有一种可能：这个整数超出了 `i64` 的范围，于是它退化成 `Raw(String)`，原样保留——这就是 `JsonNumber` 需要第三个变体的原因。带小数点或指数的走 `f64`；`"1e999"` 解析出来是无穷大，`is_finite()` 为假，报 `NumberOutOfRange`。这些判断为什么重要，40.5 会给数字。

把碎片拼起来以后，`main` 只需要三行有效代码：

```rust,noplayground
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
```

没有命令行参数时用一个内嵌样例，这样 `cargo run` 立刻有输出，方便手工试；`--pretty` 走 40.8 的缩进输出，其余第一个参数当作待解析的 JSON 文本。`String` 拥有这段输入，`Parser` 借用它，生命周期关系正好满足。

字符串解码是本章唯一需要分配内存的地方，所以它单独成函数：

```rust,noplayground
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
```

入参 `raw` 是扫描器给出的字符串内容，两端引号已经去掉。没有反斜杠就直接 `to_string`，这是绝大多数真实数据的路径，一次分配、零次逐字符判断。

有转义时逐字节前进。遇到普通字节，用 `raw[i..].chars().next()` 取一个完整字符，再按 `len_utf8()` 前进，所以 `i` 永远落在字符边界上——这正是 40.3 那个 panic 的反面教材要提醒的事。`expect` 不是给运行时兜底的，它是写给读代码的人看的断言：这里不可能切歪。

`base = pos + 1` 把「字符串内容内的偏移」换算成「整个文档的字节偏移」，那个 `+ 1` 跳过开头的引号。这种偏移换算最容易被忽略，忘了以后所有字符串内部的报错都会差一个字节。

`\u` 转义需要两个辅助函数。先是读四位十六进制：

```rust,noplayground
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
```

`to_digit(16)` 同时接受 `0-9`、`a-f`、`A-F`，省掉手写查表；返回 `None` 就说明不是十六进制字符。四个字节凑成一个 `u16`，最大 `0xFFFF`，不会溢出。

然后处理代理对（surrogate pair）。JSON 的 `\uD83D\uDE00` 用两个 16 位码元表示一个 BMP 之外的字符：

```rust,noplayground
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
```

高代理（`0xD800..=0xDBFF`）后面必须紧跟 `\u` 和低代理（`0xDC00..=0xDFFF`），公式是 `0x10000 + (高 - 0xD800) * 0x400 + (低 - 0xDC00)`。孤立的低代理，或者高代理后面接了个不是低代理的码元，都报 `InvalidUnicode`。`char::from_u32` 会拒绝 `0xD800..=0xDFFF` 这一段，所以直接写 `\uD800`（不带配对）也不会构造出非法字符。

返回值是 `(char, 下一个位置)`，而不是只返回 `char`：调用者必须知道这次转义吃掉了 6 个字节还是 10 个字节，否则 `i` 会错位。

## 40.5 数字精度陷阱

如果所有数字都存成 `f64`，下面这段程序会告诉你代价：

```rust
fn main() {
    let big: f64 = "9007199254740993".parse().unwrap();
    println!("{}", big);
    println!("{}", 0.1 + 0.2);
    println!("{}", ((1u64 << 53) | 1) as f64 == (1u64 << 53) as f64);
}
```

```console
$ cargo run
9007199254740992
0.30000000000000004
true
```

第一行：`9007199254740993` 读进来变成了 `9007199254740992`。`f64` 有 53 位尾数，到 `2^53` 为止每个非负整数都能精确表示，再往上只有部分整数可以精确表示。第三行更直接：`2^53` 与 `2^53 + 1` 作为 `u64` 是两个数，转成 `f64` 之后变得相等。第二行是经典的 `0.1 + 0.2`，二进制浮点无法精确表示十进制小数。

这意味着两类数据不能用 `f64` 装：超过 `2^53` 的整数 ID（数据库主键、雪花 ID），以及金额这类十进制小数。本项目的策略是分三级，对应 `JsonNumber` 的三个变体：

- 不带 `.`、`e`、`E` 的字面量按 `i64` 解析，精确。
- 超出 `i64` 的整数退化为 `Raw(String)`，原样保留字面量，往返逐字节一致。测试里用 `18446744073709551615`（`u64` 最大值）验证这一点。
- 带小数点或指数的走 `f64`，与 serde_json 的默认行为一致；`serde_json` 打开 `arbitrary_precision` 特性后也改用字符串保存原始形态，`Raw` 就是同一思路的最小版本。

序列化时还有一个容易踩的坑：`f64` 的 `1.0` 用 `{}` 格式化会输出 `1`。读回来是 `Int(1)`，类型从浮点变成了整数，往返不再等价。`Display for JsonNumber` 里检查结果有没有 `.` 或 `e`，没有就补 `.0`，正是为了保住这一点。

## 40.6 序列化回字符串

写回比解析短得多。对象和数组只需要处理分隔符：

```rust,noplayground
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
}
```

传 `&mut String` 而不是返回 `String`，是因为嵌套结构会不断拼接；反复 `format!` 再相加会产生一串中间字符串。这种「一个出口缓冲、递归往里写」的写法在渲染与序列化里很常见。

字符串的转义是真正需要小心的地方：

```rust,noplayground
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
```

必须转义的只有三类：双引号、反斜杠、以及所有小于 `0x20` 的控制字符。斜杠 `/` 可以转义也可以不转义，这里选择不转，输出更短。`\u{0008}` 与 `\u{000c}` 是退格和换页，它们有专用的两字符转义 `\b` 与 `\f`；其他控制字符没有专用写法，统一用 `\u{:04x}`。大于 `0x1F` 的字符一律原样输出——UTF-8 文本本身就是合法 JSON，中文和 emoji 都不需要转义成 `\uXXXX`。

跑一遍组装好的项目，下面这段带引号与换行的字符串被正确转义：

```console
$ cargo run
{"msg":"hi\n\"rust\"","n":1.0}
```

对象内的键也走同一个 `write_json_string`，所以 `{"a\nb":1}` 这类键写回去时不会破坏格式。

## 40.7 单元测试

每个测试盯住前面提到的一个陷阱，而不是泛泛地「能解析」：

```rust,noplayground
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
```

六个测试各自对应一个论点：标量与空白、往返一致性、转义与代理对、大整数精确性、错误位置、深度上限。测试名写成一句断言而不是 `test1`，失败时 `cargo test` 的输出自己会说话。

```console
$ cargo test
running 6 tests
test tests::decodes_escapes_and_surrogates ... ok
test tests::keeps_big_integers_exact ... ok
test tests::parses_nested_and_round_trips ... ok
test tests::parses_scalars ... ok
test tests::reports_byte_offset ... ok
test tests::rejects_deep_nesting ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

## 40.8 加一个 pretty-print

紧凑输出方便传输，给人看却难受。加一个缩进版本只需要一个 `indent` 参数：

```rust,noplayground
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

fn pad(level: usize, out: &mut String) {
    for _ in 0..level * 2 { out.push(' '); }
}
```

再给 `JsonValue` 加一个返回 `String` 的包装，`main` 里就能一行调用（同一个类型可以有多个 `impl` 块）：

```rust,noplayground
impl JsonValue {
    fn to_pretty(&self) -> String {
        let mut out = String::new();
        self.write_pretty(0, &mut out);
        out
    }
}
```

空数组与空对象用守卫分支单独处理，输出 `[]` 与 `{}`，不换行也不缩进——这是 pretty 打印的通行约定，`serde_json::to_string_pretty` 也是这么做的。每层缩进两个空格，所以 `pad` 里乘 2：

```console
$ cargo run -- --pretty
{
  "name": "Ana",
  "age": 30,
  "tags": [
    "rust",
    "json"
  ],
  "score": 9.5
}
```

再往前走一步就是查询：给 `JsonValue` 加一个 `fn get(&self, path: &str) -> Option<&JsonValue>`，支持 `user.tags[0]` 这样的写法，返回值借用而不是克隆。怎么拆词、怎么处理数组下标、越界返回什么，是下一个练习。

## 40.9 深入：三个真实边界

### 递归深度与栈溢出

把 40.4 的深度上限拿掉，喂给解析器 10 万个左方括号，会发生什么？下面这段独立的递归下降片段省略了解析器其余部分，只保留「每遇见一个 `[` 就递归一次」这个形状：

```rust,noplayground
struct Rec {
    src: Vec<u8>,
    pos: usize,
}

impl Rec {
    fn value(&mut self) -> u64 {
        match self.src.get(self.pos) {
            Some(b'[') => {
                self.pos += 1;
                let mut n = 0;
                loop {
                    n += self.value();
                    match self.src.get(self.pos) {
                        Some(b',') => self.pos += 1,
                        Some(b']') => { self.pos += 1; return n; }
                        _ => return n,
                    }
                }
            }
            Some(b'0') => { self.pos += 1; 1 }
            _ => 0,
        }
    }
}

fn main() {
    let mut rec = Rec { src: vec![b'['; 100_000], pos: 0 };
    println!("{}", rec.value());
}
```

```text
thread 'main' (36628) has overflowed its stack
```

每一个 `[` 都换来一次 `value` 递归，调用栈被一层层吃掉。Windows 主线程默认只有 1 MB 栈空间（Linux 通常是 8 MB），10 万层就已经很危险。注意输出里没有 `panicked` 字样：栈溢出在 Rust 里是**进程终止**，不是可以被 `catch_unwind` 捕获的 panic，退出码是 `0xC00000FD`（`-1073741571`）。

所以 `MAX_DEPTH = 128` 不是为了性能，而是一道安全边界：解析器处理的输入往往来自外部，一条 `[[[[` 就能让你的服务进程消失。上限取多少是权衡——太小会拒绝合法的深嵌套数据，太大挡不住攻击。测试里用 `"[".repeat(200)` 稳定地触发 `DepthLimit`。

真想支持更深的嵌套，可以用 `std::thread::Builder::new().stack_size(64 * 1024 * 1024)` 起一个大栈线程，或者把递归改写成显式栈的迭代解析。前者只是把上限抬高，后者才是根治。

### 零拷贝借用与所有权

词法层的 `Token<'a>` 借用输入，语法层的 `JsonValue` 拥有 `String` 和 `Vec`。为什么不全借用？让 `JsonValue` 的字符串变体写成 `&'a str`，等于把整棵树的寿命绑死在输入上：

```rust,noplayground
enum JsonValue<'a> {
    String(&'a str),
    Object(Vec<(&'a str, JsonValue<'a>)>),
    // 其余变体略
}
```

这样确实能省掉全部字符串分配，但有两个问题。第一，带转义的字符串没法借用——`"a\\nb"` 解码后是 `a` 加换行加 `b`，输入里根本没有这段连续字节，必须分配；折中方案是 `Cow<'a, str>`，无转义时借、有转义时拥有。第二，解析结果不能比输入活得更久。函数里 `let text = read_file()?;` 解析出的树想返回给调用者，编译器就会拒绝：`text` 在函数结束时被释放，树里还挂着它的切片。

本项目的分界很常见：**解析期间零拷贝，解析结果拥有数据**。serde 把选择的自由交给用户——`Deserialize<'de>` 允许字段借用 `&'de str`，代价是结果的生命周期必须短于输入。

### serde_json 的 Visitor 模式

本章的 `parse_value` 总是先造一棵 `JsonValue` 树。如果目标类型是已知的 `struct`，这棵树纯属多余。serde_json 的做法是：`Deserializer` 一边扫描文本，一边把遇到的每个值直接交给一个 `Visitor`：

```rust,noplayground
impl<'de> Visitor<'de> for NameVisitor {
    type Value = String;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("一个字符串")
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
        Ok(v.to_uppercase())
    }
}
```

`visit_str` 的职责与本章的 `parse_string` 完全对应：拿到一段已经解码的文本，决定怎么用。区别只在于谁接收结果——本章把值塞进树，Visitor 直接构造用户类型。没有中间树，就省掉了一次分配与一次遍历，还能把 `&'de str` 借给结构体字段；代价是每个类型都要实现（或派生）一个 Visitor。看懂这两个模式的对应关系，再读 serde 的源码就不会迷路。

## 典型错误案例
### 案例 1：函数声明返回 String，却交回一个借用片段——E0308
```rust,editable
fn parse_string(src: &str, start: usize) -> String {
    &src[start..start + 2]
}
fn main() {
    println!("{}", parse_string("hi", 0));
}
```
```text
error[E0308]: mismatched types
 --> src/main.rs:2:5
  |
2 |     &src[start..start + 2]
  |     ^^^^^^^^^^^^^^^^^^^^^^ expected `String`, found `&str`
help: try using a conversion method
  |
2 |     (&src[start..start + 2]).to_string()
```
**原因**：函数签名把返回值定为 `String`，而 `src[a..b]` 的类型是 `&str`；函数体末尾表达式的类型必须与签名一致，借用不会自动变成所有权。同一个错误码也出现在 `match` 的分支分别返回整数与 `bool` 时，那时的报错标题是 `match` arms have incompatible types。**修复**：在切片后面接一次 `to_string()`，显式写出这次分配。
```rust,editable
fn parse_string(src: &str, start: usize) -> String {
    src[start..start + 2].to_string()
}
fn main() {
    println!("{}", parse_string("hi", 0));
}
```

### 案例 2：在 &self 方法里搬走字段——E0507
```rust,editable
struct Doc { text: String }
impl Doc {
    fn take(&self) -> String {
        self.text
    }
}
fn main() {
    let d = Doc { text: String::from("hi") };
    println!("{}", d.take());
}
```
```text
error[E0507]: cannot move out of `self.text` which is behind a shared reference
 --> src/main.rs:4:9
  |
4 |         self.text
  |         ^^^^^^^^^ move occurs because `self.text` has type `String`
  |
help: consider cloning the value if the performance cost is acceptable
  |
4 |         self.text.clone()
```
**原因**：`&self` 只借到共享引用，`self.text` 的所有权还留在调用方手里；`String` 没有实现 `Copy`，编译器不允许把字段从借用里搬出来。**修复**：克隆一份返回，或把方法改为接收 `self`；确实要清空字段时用 `std::mem::take(&mut self.text)` 换取所有权。
```rust,editable
struct Doc { text: String }
impl Doc {
    fn take(&self) -> String {
        self.text.clone()
    }
}
fn main() {
    let d = Doc { text: String::from("hi") };
    println!("{}", d.take());
}
```

### 案例 3：对枚举调用 to_string 却没有 Display——E0599
```rust,editable
enum JsonValue { String(String) }
fn main() {
    let v = JsonValue::String(String::from("hi"));
    let text = v.to_string();
    println!("{}", text);
}
```
```text
error[E0599]: `JsonValue` doesn't implement `std::fmt::Display`
 --> src/main.rs:4:18
  |
4 |     let text = v.to_string();
  |                  ^^^^^^^^^ method cannot be called on `JsonValue` due to unsatisfied trait bounds
  |
  = note: the following trait bounds were not satisfied:
          `JsonValue: std::fmt::Display`
          which is required by `JsonValue: ToString`
```
**原因**：`to_string` 来自 `ToString`，而 `ToString` 由 `Display` 的兜底实现自动提供；`enum` 默认只实现 `Debug`，没有 `Display` 就没有 `to_string`。**修复**：为枚举实现 `Display`，在 `fmt` 里按变体分派。
```rust,editable
use std::fmt;
enum JsonValue { String(String) }
impl fmt::Display for JsonValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JsonValue::String(s) => write!(f, "{}", s),
        }
    }
}
fn main() {
    let v = JsonValue::String(String::from("hi"));
    println!("{}", v.to_string());
}
```

### 案例 4：把一个值交给函数两次——E0382
```rust,editable
struct Doc { text: String }
fn dump(d: Doc) -> String {
    d.text
}
fn main() {
    let d = Doc { text: String::from("hi") };
    let a = dump(d);
    let b = dump(d);
    println!("{} {}", a, b);
}
```
```text
error[E0382]: use of moved value: `d`
 --> src/main.rs:8:18
  |
7 |     let a = dump(d);
  |                  - value moved here
8 |     let b = dump(d);
  |                  ^ value used here after move
note: consider changing this parameter type in function `dump` to borrow instead
```
**原因**：`dump(d)` 按值接收参数，取得 `d` 的所有权；第一次调用后 `d` 已被移动，第二次再用就是使用已移动的值。**修复**：把参数改成借用 `&Doc`，调用处写 `dump(&d)`；只有在结构体实现了 `Clone` 且确实要保留副本时才克隆。
```rust,editable
struct Doc { text: String }
fn dump(d: &Doc) -> String {
    d.text.clone()
}
fn main() {
    let d = Doc { text: String::from("hi") };
    let a = dump(&d);
    let b = dump(&d);
    println!("{} {}", a, b);
}
```

### 案例 5：问号运算符不会凭空转换错误类型——E0277
```rust,editable
use std::fs;
struct ParseError { pos: usize }
fn load(path: &str) -> Result<String, ParseError> {
    Ok(fs::read_to_string(path)?)
}
fn main() {
    let _ = load("data.json");
}
```
```text
error[E0277]: `?` couldn't convert the error to `ParseError`
 --> src/main.rs:4:32
  |
4 |     Ok(fs::read_to_string(path)?)
  |        ------------------------^ the trait `From<std::io::Error>` is not implemented for `ParseError`
note: `ParseError` needs to implement `From<std::io::Error>`
  |
2 | struct ParseError { pos: usize }
  | ^^^^^^^^^^^^^^^^^
```
**原因**：`?` 在提前返回之前会对错误值调用一次 `From` 转换；`read_to_string` 报的是 `std::io::Error`，函数的错误类型是 `ParseError`，两者之间没有 `From` 实现。**修复**：为一个错误类型实现 `From<std::io::Error>`，解析器就能把读文件的失败也收进同一条错误通道。
```rust,editable
use std::fs;
use std::io;
struct ParseError { pos: usize }
impl From<io::Error> for ParseError {
    fn from(_: io::Error) -> Self { ParseError { pos: 0 } }
}
fn load(path: &str) -> Result<String, ParseError> {
    Ok(fs::read_to_string(path)?)
}
fn main() {
    let _ = load("data.json");
}
```

## 完整源码

本章的完整实现在 `projects/40-json-parser/`（<https://github.com/yujh129/rust-book-zh/tree/main/projects/40-json-parser>）：单文件 690 行，零依赖。

```console
$ cd projects/40-json-parser
$ cargo run --offline -- --pretty
{
  "name": "Ana",
  "age": 30,
  "tags": [
    "rust",
    "json"
  ],
  "score": 9.5
}
$ cargo run --offline -- '{"a": 1,}'
解析失败：字节偏移 8：对象的键必须是字符串
```

| 文件 | 内容 |
| --- | --- |
| `src/main.rs` | 按 40.2–40.8 的顺序组织：`JsonValue`/`JsonNumber`、词法分析器、递归下降解析器（深度上限）、`to_json`/`to_pretty`、`main`，末尾是 6 个单元测试 |

`cargo test --offline` 跑 6 个测试。正文「典型错误案例」里的五段代码是故意写错的示例，不属于实现，项目里只收正确版本，所以测试是全绿的。

## 动手练习

1. 给 `Tokenizer` 增加 `peek(&self) -> Token`，不消费地看一眼下一个 token，并用它重写 `parse_array` 的结束判断；现有 6 个测试必须全绿。
2. 让解析器接受尾随逗号（`[1, 2, ]`）与 `//` 行注释，各写一个测试，并说明这两种放宽为什么不属于 RFC 8259。
3. 把 `JsonValue` 改成带参数的 `JsonValue<'a>`，字符串用 `&'a str` 借用，只在出现转义时才分配 `String`；记录需要改动的函数，并比较 1 MB 输入的解析耗时。
4. 给命令行加 `--pretty` 开关，输出两空格缩进，并保证 `[]` 与 `{}` 保持单行。
5. 实现 `get(&self, path: &str) -> Option<&JsonValue>`，支持 `a.b[0].c` 这样的子集查询，路径不存在时返回 `None`。

## 小结

1. JSON 只有六种值：`null`、`bool`、`number`、`string`、`array`、`object`，用带 variant 的 `enum` 建模最直接。
2. 对象用 `Vec<(String, JsonValue)>` 而不是 `HashMap`，可以保留键的顺序与重复键，代价是查找变慢。
3. 词法分析器只做一件事：把字节流切成带位置的 token；错误位置用字节偏移记录，比字符下标便宜，也能直接用于切片。
4. 数字不能统一用 `f64`：超过 2^53 后，相邻整数不能再全部精确表示，所以整数走 `i64`、浮点走 `f64`、超出 `i64` 的纯整数字面量保留原文，非有限的浮点结果报错。
5. 递归下降的每一步都只负责一种语法结构，并假定调用者已经消费到它该开始的位置，这样每个函数都能单独读懂。
6. 深度限制是解析器的一部分：没有 `MAX_DEPTH`，恶意输入就能打穿调用栈，而栈溢出不是 `Result` 能捕获的错误。
7. 序列化要为每个字符负责：双引号、反斜杠、控制字符都必须转义，浮点要保留 `.0` 才能与整数区分并原样往返。
8. 生产级实现的两个方向是零拷贝借用与错误类型分层，`serde` 的 `Visitor` 模式则把「解码文本」与「构造类型」彻底拆开。

