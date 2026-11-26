# 第 29 章 序列化与配置：serde 实战

> **本章目标**
>
> - 理解 serde 的「数据模型」抽象：为什么一份 `#[derive]` 能同时支持 JSON、TOML、YAML、MessagePack
> - 熟练使用 serde 的容器属性与字段属性，写出既能读旧配置、又能写新配置的结构体
> - 掌握枚举的四种表示方式，知道各自的 JSON 形态与踩坑点
> - 能对不听话的外部格式手写 `Serialize` / `Deserialize`
> - 建立配置文件的演进策略：未知字段、缺字段、默认值、版本号

## 29.1 serde 解决的是 N×M 问题

假设你的程序里有 N 个类型要落盘、要发网络、要读配置，而世界上有 M 种格式（JSON、TOML、YAML、MessagePack、CBOR、CSV、bincode……）。
如果每种类型为每种格式各写一份转换代码，工作量是 N×M，而且每加一种格式就要把 N 个类型全部重写一遍。

serde 的做法是在中间插一层**数据模型**：类型只需要描述「我由哪些字段组成、每个字段是什么形状」，格式只需要描述「怎么把这个形状写成字节」。
于是工作量变成 N + M，而且两边互不认识。

这一层数据模型是有限的一组形状，serde 把它们定义成 `Serializer` trait 上的方法：

```text
bool  i8 i16 i32 i64 i128  u8 u16 u32 u64 u128  f32 f64  char  str  bytes
none  some  unit  unit_struct  unit_variant  newtype_struct  newtype_variant
seq  tuple  tuple_struct  tuple_variant  map  struct  struct_variant
```

你的类型实现 `Serialize`，做的是「把我的值翻译成这些形状之一」；`serde_json` 实现 `Serializer`，做的是「把这些形状写成 JSON 文本」。
`Deserialize` 反过来：格式把字节解析成形状，再通过一个 `Visitor` 回调把你的类型拼出来。

```rust,noplayground
pub trait Serialize {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer;
}

pub trait Deserialize<'de>: Sized {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>;
}
```

注意 `Deserialize<'de>` 上的生命周期：`'de` 是**输入数据的生命周期**。
它存在的唯一理由是允许零拷贝——`&'de str` 可以直接指向输入缓冲区里的一段字节，而不必分配 `String`。
这个设计代价会在 29.7 和错误案例 2 里反复出现。

常见的格式 crate：

| 格式 | crate | 典型用途 | 是否人类可读 |
| --- | --- | --- | --- |
| JSON | serde_json | HTTP API、日志、跨语言交换 | 是 |
| TOML | toml | Cargo.toml 这类配置文件 | 是 |
| YAML | serde_yaml（已归档，社区转向 serde_yml / serde_norway） | K8s 清单、CI 配置 | 是 |
| MessagePack | rmp-serde | 比 JSON 小的二进制协议 | 否 |
| CBOR | ciborium | 物联网、需要 schema 的二进制 | 否 |
| bincode | bincode | 进程间高速传输（不保证跨版本兼容） | 否 |
| CSV | csv | 表格数据 | 是 |

一个最小的端到端例子：

```rust,noplayground
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
struct Config {
    host: String,
    port: u16,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let c = Config { host: "localhost".into(), port: 8080 };

    let json = serde_json::to_string(&c)?;
    println!("{json}");

    let back: Config = serde_json::from_str(r#"{"host":"db","port":5432}"#)?;
    println!("{:?}", back);
    Ok(())
}
```

输出：

```text
{"host":"localhost","port":8080}
Config { host: "db", port: 5432 }
```

`#[derive(Serialize, Deserialize)]` 展开后是几百行手写 impl 的等价物（29.7 会看展开结果）。
本章剩下的内容，基本都是「怎么用属性控制这段展开代码的行为」。

## 29.2 属性速查：容器、字段、变体

serde 的属性分三层，作用域不同，写错层级是最常见的困惑来源。

### 容器属性（写在 struct / enum 上）

| 属性 | 作用 |
| --- | --- |
| rename = "X" | 把类型名改成 X（对 struct 名影响很小，对 enum 变体名影响大） |
| rename_all = "camelCase" | 批量重命名所有字段/变体，可选 snake_case、kebab-case、SCREAMING_SNAKE_CASE 等 |
| deny_unknown_fields | 遇到不认识的字段就报错，而不是静默忽略 |
| default | 反序列化时缺失的字段用 Default::default() 补 |
| default = "path" | 缺失字段用指定函数补 |
| tag = "type" | 内部标签枚举（29.3） |
| tag = "t", content = "c" | 相邻标签枚举 |
| untagged | 无标签枚举 |
| transparent | 单字段 newtype 完全透明，序列化结果就是内层值 |
| bound = "T: MyTrait" | 覆盖自动推导的 trait bound |
| crate = "path::to::serde" | 重新导出 serde 时用（写库会用到） |

### 字段属性（写在字段上）

| 属性 | 作用 |
| --- | --- |
| rename / rename(serialize = ..., deserialize = ...) | 单字段改名，两个方向可以不同 |
| alias = "old_name" | 反序列化时额外接受的旧名字（可写多个） |
| default / default = "path" | 该字段缺失时的兜底值 |
| skip | 两个方向都跳过（反序列化时必须能 default） |
| skip_serializing / skip_deserializing | 只跳一个方向 |
| skip_serializing_if = "Option::is_none" | 条件跳过，常用于省略空字段 |
| flatten | 把内层结构体的字段摊平到外层（29.4） |
| with = "module" | 用模块里的 serialize/deserialize 函数替换 |
| serialize_with / deserialize_with | 只替换一个方向 |
| borrow | 明确要求零拷贝借用（29.7） |

### 变体属性（写在 enum 变体上）

| 属性 | 作用 |
| --- | --- |
| rename / alias | 单独改名 |
| skip | 该变体不参与序列化/反序列化 |
| other | 兜底变体，接收所有不认识的字符串 |
| untagged | 单个变体无标签（与容器级 untagged 配合） |

一个把这些属性用起来的配置结构：

```rust,noplayground
use serde::{Deserialize, Serialize};

fn default_port() -> u16 { 8080 }

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
struct ServerConfig {
    host: String,
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls: Option<TlsConfig>,
    #[serde(alias = "workers", default = "default_workers")]
    worker_count: usize,
}

#[derive(Debug, Serialize, Deserialize)]
struct TlsConfig {
    cert: String,
    key: String,
}

fn default_workers() -> usize { 4 }

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg: ServerConfig = serde_json::from_str(
        r#"{"host":"0.0.0.0","workers":8}"#,
    )?;
    println!("{:#?}", cfg);
    println!("{}", serde_json::to_string(&cfg)?);
    Ok(())
}
```

输出：

```text
ServerConfig {
    host: "0.0.0.0",
    port: 8080,
    tls: None,
    worker_count: 8,
}
{"host":"0.0.0.0","port":8080,"worker-count":8}
```

这里同时发生了四件事：`host` 原样保留；`port` 缺失，用函数补成 8080；`tls` 缺失变成 `None` 且序列化时被省略；`worker_count` 通过 alias 读到了旧名字 `workers`，输出时又按 `rename_all` 写成 `worker-count`。
这就是「读旧写新」的基本手法——迁移配置格式时不必写两套结构体。

## 29.3 枚举的四种表示方式

枚举是 serde 里最容易踩坑的部分，因为「一个枚举值」要编码成 JSON 时，既要记下是哪个变体，又要记下变体里的数据。
这个「记下变体」的标签怎么放，就是四种表示方式的区别。

### 默认：外部标签（externally tagged）

标签作为外层对象唯一的键，数据作为值。四种变体形状的实测结果：

```rust,noplayground
#[derive(Debug, Serialize, Deserialize)]
enum Ext {
    Quit,
    Text(String),
    Click { x: i32, y: i32 },
    Move(i32, i32),
}
```

```text
Ext::Quit          -> "Quit"
Ext::Text("hi")    -> {"Text":"hi"}
Ext::Click{x:1,y:2} -> {"Click":{"x":1,"y":2}}
Ext::Move(3, 4)     -> {"Move":[3,4]}
```

外部标签不需要任何属性，任何变体形状都能用，是兼容性最好的选择。
缺点是在 JSON 里多嵌套一层，前端写 `obj.Click.x` 很难看。

### 内部标签（internally tagged）

```rust,noplayground
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Event {
    Click { x: i32 },
    Text(String),
}
```

结构体变体看起来很舒服：`Event::Click { x: 3 }` 变成 `{"type":"Click","x":3}`。
但 newtype 变体（`Text(String)`）**不能序列化**，会在运行时报错：

```text
err: cannot serialize tagged newtype variant Event::Text containing a string
```

原因：内部标签要求变体的数据必须能写成「键值对集合」，这样才能和 `type` 字段平级合并。
`String` 是标量，没有字段可以平级，于是失败。
同理，tuple 变体和 unit 变体也不受支持（unit 变体合法但序列化成 `{"type":"Quit"}`）。

如果你的协议需要内部标签，就把所有变体都写成结构体变体，或者用相邻标签。

### 相邻标签（adjacently tagged）

```rust,noplayground
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", content = "c")]
enum Adj {
    Text(String),
    Click { x: i32 },
}
```

```text
Adj::Text("hi")  -> {"t":"Text","c":"hi"}
Adj::Click{x:1}  -> {"t":"Click","c":{"x":1}}
```

标签和数据各占一个键，任何变体形状都能用，且数据部分保持原样。
代价是 JSON 多两个字段，且反序列化时 `t` 和 `c` 的键名不能与你的字段冲突。

### 无标签（untagged）

```rust,noplayground
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum Un {
    Click { x: i32 },
    Text(String),
}
```

```text
Un::Click{x:1}  -> {"x":1}
Un::Text("hi")  -> "hi"
```

没有标签，反序列化时按**声明的顺序**逐个尝试，第一个成功的胜出：

```text
parse {"x":7} -> Click { x: 7 }
parse "7"    -> Text("7")
```

这种「结构匹配」的方式对使用者最友好，代价也很明确：

1. 必须把最具体的变体排在前面。上面若把 `Text` 放前面，`{"x":7}` 也能反序列化成 `Text("7")` 吗？不能——`String` 的 Visitor 会拒绝 map，所以这里侥幸正确。但只要两个变体都能接受同一种形状，就会静默选错。
2. untagged 反序列化必须把输入**缓存成中间表示**（serde 的 `Content` 类型）再逐个尝试，因为可能试多次。大数据量下这会带来额外的内存分配与复制。
3. 错误信息会退化成「data did not match any variant of untagged enum Un」，定位困难。

选型口诀：协议由你定就用内部标签（变体全用结构体变体）；对接已有协议就照抄它的形状——外部标签最省事，相邻标签最百搭，untagged 只在真的需要「同一个位置可能有两种形状」时用。

## 29.4 配置文件兼容：缺字段、多字段、改名

程序的配置结构体会随版本演进，而用户磁盘上的旧配置不会自动更新。
serde 的三个默认行为决定了迁移策略：

**默认一：不认识的字段被静默忽略。**

```rust,noplayground
#[derive(Debug, Deserialize)]
struct Config { host: String, port: u16 }

let r: Result<Config, _> = serde_json::from_str(r#"{"host":"localhost","port":8080,"prot":9090}"#);
println!("{:?}", r.err());
```

```text
None
```

`None` 表示没有错误——拼错的 `prot` 被丢掉了，程序照常启动。
这对向前兼容是好事（新版本加字段，旧版本还能读），对排错是坏事（用户以为改了配置，其实没生效）。
加 `#[serde(deny_unknown_fields)]` 可以让它报错：

```text
Some(Error("unknown field `prot`, expected `host`", line: 1, column: 18))
```

**默认二：缺字段会报错。**

```text
Some(Error("missing field `port`", line: 1, column: 20))
```

注意列号指向的是对象的收尾 `}`，不是字段位置——这是 serde 的报错风格，别被误导。
补默认值有三种写法：字段类型用 `Option<T>`（缺失变 `None`）、字段加 `#[serde(default)]`（用 `Default::default()`）、字段加 `#[serde(default = "函数名")]`（用自定义函数，函数签名必须是 `fn() -> T`）。

**默认三：类型不对会报错，且信息相当具体。**

```text
Some(Error("invalid type: string \"8080\", expected u16", line: 1, column: 33))
```

这解释了为什么 TOML/JSON 里 `port = "8080"` 会失败：`u16` 的访问器只接受整数，不会替你解析字符串。
要接受两种类型，要么手写 `Deserialize`（29.5），要么用一个 untagged 枚举包一层。

把三者组合成一套演进策略：

```rust,noplayground
#[derive(Debug, Serialize, Deserialize)]
#[serde(default)]              // 整体兜底：任何字段缺失都用 Default
struct Config {
    version: u32,              // 显式版本号，便于写迁移逻辑
    host: String,
    port: u16,
    #[serde(alias = "worker", alias = "workers")]
    worker_count: usize,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    features: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            version: 1,
            host: "127.0.0.1".into(),
            port: 8080,
            worker_count: 4,
            features: Vec::new(),
        }
    }
}
```

`#[serde(default)]` 放在容器上等价于给每个字段都加 `#[serde(default)]`，但字段里有 `Option` 时行为不变（`Option` 本来就把缺失当 `None`）。
配合 `alias`，旧名字 `worker` / `workers` 都能读；配合 `skip_serializing_if`，空字段不会出现在写出的配置里。

注意 `#[serde(default)]` 的语义是「缺失才用默认值」，不是「解析失败就用默认值」——写了 `port = "abc"` 依然报错，这通常正是你想要的：配置错了就该大声失败，而不是悄悄退回默认值。

## 29.5 手写 Serialize / Deserialize

属性不够用时就得自己实现。典型场景：外部 API 把同一个字段既可能写成整数、又可能写成字符串。

```text
{ "name": "api", "port": 8080 }
{ "name": "api", "port": "8080" }
```

`u16` 只接受前者，`String` 只接受后者，属性解决不了「两种都行」。
先实现一个 newtype，把「怎么变成形状」和「怎么从形状变回来」分别写清楚：

```rust,noplayground
use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

#[derive(Debug, PartialEq)]
struct Port(u16);

// 序列化：永远写成 JSON 整数
impl Serialize for Port {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u16(self.0)
    }
}

// 反序列化：整数和字符串都收
impl<'de> Deserialize<'de> for Port {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct PortVisitor;

        impl<'de> Visitor<'de> for PortVisitor {
            type Value = Port;

            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a port number or a string containing one")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Port, E> {
                u16::try_from(v)
                    .map(Port)
                    .map_err(|_| E::custom(format!("port out of range: {v}")))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Port, E> {
                v.parse::<u16>().map(Port).map_err(E::custom)
            }
        }

        d.deserialize_any(PortVisitor)
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct Svc {
    name: String,
    port: Port,
}
```

实测行为：

```text
{"name":"api","port":8080}   -> ok: Svc { name: "api", port: Port(8080) } -> {"name":"api","port":8080}
{"name":"api","port":"8080"} -> ok: Svc { name: "api", port: Port(8080) } -> {"name":"api","port":8080}
{"name":"api","port":99999}  -> err: port out of range: 99999 at line 1 column 26
{"name":"api","port":true}   -> err: invalid type: boolean `true`, expected a port number or a string containing one at line 1 column 25
```

几个值得记住的细节：

1. `Visitor` 的方法名对应数据模型里的形状：`visit_bool`、`visit_i64`、`visit_u64`、`visit_f64`、`visit_str`、`visit_string`、`visit_seq`、`visit_map`。只实现你关心的，剩下的会走默认实现并返回「invalid type」错误。
2. `visit_u64` 而不是 `visit_u16`：serde 的数据模型里整数统一是 i8/i16/i32/i64/u8/u16/u32/u64。JSON 数字进来时 serde_json 会根据大小选择最小的那个，小数字通常走 `visit_u64`。为了稳妥，很多库会同时实现 `visit_u64` 和 `visit_i64`。
3. `E::custom(msg)` 让错误信息带上你的上下文，最终会拼在位置信息前面，形成上面 `port out of range: 99999 at line 1 column 26` 的效果。位置指向的是出错的值，方便定位。
4. `deserialize_any` 是「让格式自己决定给我什么形状」，只有自描述格式（JSON、TOML、MessagePack）支持。bincode 这种紧凑格式必须在编译期就知道类型，调用 `deserialize_any` 会直接运行时报错。跨格式复用的代码要显式调用 `deserialize_u16` 等方法，或者在 Visitor 里对两种情形分别提供路径。

`expecting` 的信息直接决定用户看到的错误，写清楚一点，比如 `a port number between 1 and 65535` 就比 `u16` 有用得多。

手写 `Serialize` 也能省掉中间层。比如一个只用于展示的 `Id(u64)`，你希望它总是以字符串出现（避免 JS 的大整数精度问题）：

```rust,noplayground
impl Serialize for Id {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(&self.0)   // 等价于写成一个字符串
    }
}
```

`collect_str` 会调用 `Display` 实现；对 `&str` 用 `serialize_str`，对字节数组用 `serialize_bytes`。

## 29.6 serde_json 的日常用法

JSON 是最常用的格式，`serde_json` 的 API 面不大，值得一次记住。

### 强类型与弱类型之间

```rust,noplayground
use serde_json::json;

let v = json!({"b": 1, "a": 2, "c": [1, 2, 3]});
println!("{}", serde_json::to_string(&v).unwrap());
```

```text
{"a":2,"b":1,"c":[1,2,3]}
```

注意输出顺序和书写顺序**不一致**：默认配置下 `serde_json::Map` 是 `BTreeMap`，键会被排序。
想保序要开 `serde_json/preserve_order` feature（内部换成 IndexMap）。
对配置文件来说排序是好事（diff 稳定）；对需要复现客户端字段顺序的协议，就得开这个 feature。

`json!` 宏构造 `Value`，`to_value` 把强类型转成 `Value`，`from_value` 反向：

```rust,noplayground
let p = P { name: "db".into(), retries: 2 };
let val = serde_json::to_value(&p)?;      // Value::Object(...)
let back: P = serde_json::from_value(val)?;
```

`Value` 支持下标访问，缺失的键返回 `Value::Null` 而不是 panic：

```text
index: "db"
missing index: null
```

想区分「不存在」和「显式为 null」，用 `back.get("nope")` 拿 `Option<&Value>`。

### 读写文件与流式解析

`to_writer` / `from_reader` 直接对 `Write` / `Read` 工作，省掉中间 `String`：

```rust,noplayground
let file = std::fs::File::create("cfg.json")?;
serde_json::to_writer_pretty(file, &cfg)?;

let file = std::fs::File::open("cfg.json")?;
let cfg: Config = serde_json::from_reader(file)?;
```

注意 `from_reader` 是同步阻塞 IO，且 JSON 解析器不接受「尾部有多余内容」之外的异常——从网络流读时容易卡在等待更多字节上。
处理按行分隔的 JSON（NDJSON、JSON Lines，日志和流式 API 常见）用 `StreamDeserializer`：

```rust,noplayground
let text = "{\"id\":1}\n{\"id\":2}\n";
let de = serde_json::Deserializer::from_str(text);
for item in de.into_iter::<serde_json::Value>() {
    println!("{:?}", item?);
}
```

```text
ndjson: Object {"id": Number(1)}
ndjson: Object {"id": Number(2)}
```

`into_iter::<T>()` 每次只消费一行（确切地说是消费到一个完整 JSON 值），内存占用与文件大小无关，这是处理大日志的正确姿势。

### 把多余字段收进一个 map

有时你只关心协议里的几个字段，其余要保持原样透传。`#[serde(flatten)]` 收集到 `BTreeMap` 或 `Value`：

```rust,noplayground
#[derive(Debug, Serialize, Deserialize)]
struct Svc {
    name: String,
    port: Port,
    #[serde(flatten)]
    extra: std::collections::BTreeMap<String, String>,
}
```

```text
{"name":"api","port":8080,"region":"cn"} -> Svc { name: "api", port: Num(8080), extra: {"region": "cn"} }
```

反过来，`flatten` 也让「公共字段 + 具体字段」的层级结构可以出现在同一层 JSON 里。
代价是 flatten 必须先把输入解析成中间表示（和 untagged 一样），且它和非自描述格式（如 bincode）不兼容。

## 29.7 深入：derive 展开了什么，零拷贝又值多少

### derive 生成的是普通代码，不是反射

`#[derive(Serialize)]` 在编译期展开成手写风格的 impl。上面 `Config` 大致变成：

```rust,noplayground
impl serde::Serialize for Config {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("Config", 2)?;
        st.serialize_field("host", &self.host)?;
        st.serialize_field("port", &self.port)?;
        st.end()
    }
}
```

反序列化更绕一点：生成的代码先算一个字段名到索引的表（`FIELDS: &[&str]`），再实现一个 `Visitor` 的 `visit_map`，内部循环调用 `map.next_key()` 匹配字段名、`map.next_value()` 取值。
`deny_unknown_fields`、`alias`、`default` 全都是在这个循环里加分支实现的，没有额外机制。

这就是 serde 快的根本原因：**没有运行时类型信息，没有虚表分发，字段名在编译期就变成了常量字符串**，序列化一个小结构体基本就是若干次 append。
也是 serde 慢的原因（相对于手写解析器）：每个字段都要走一次泛型 `serialize_field`，编译器要花力气内联，跨 crate 泛型实例化还会让二进制膨胀。
想看确切展开结果，装 `cargo-expand` 后运行 `cargo expand --lib 你的模块`。

### 零拷贝：`&'de str` 与 `#[serde(borrow)]`

`Deserialize<'de>` 的生命周期参数就是为了这个。对比两种写法：

```rust,noplayground
#[derive(Debug, Deserialize)]
struct Owned {
    name: String,      // 拷贝一份，输入可以立刻释放
}

#[derive(Debug, Deserialize)]
struct Borrowed<'a> {
    name: &'a str,     // 指向输入缓冲区，零拷贝
}
```

`Borrowed` 的 derive 会生成 `impl<'de: 'a, 'a> Deserialize<'de> for Borrowed<'a>`，那个 `'de: 'a` 约束的意思是「输入必须活得比借用久」。
所以它能不能用，取决于输入的生命周期：

```rust,noplayground
let s = String::from(r#"{"name":"rust"}"#);
let v: Borrowed = serde_json::from_str(&s).unwrap();   // 可以：s 比 v 活得久
println!("{:?}", v);
```

```text
Borrowed { name: "rust" }
```

但一旦你想把它从函数里返回、而输入是函数内的局部变量，编译器立刻拦下来（见错误案例 2 的 E0515）。

还有一层坑：`#[serde(borrow)]` 只对特定类型生效。`&'a str` 和 `&'a [u8]` 天然借用；`Cow<'a, str>` 需要显式写 `#[serde(borrow)]`，否则 derive 会生成要求 `Cow<'static, str>` 的约束，读什么都失败；`&'a str` 放在 `Option` 里也要 `#[serde(borrow)]` 才能借到。

零拷贝的收益取决于字段大小：一个 20 字节的字符串，省下的是「一次分配 + 一次 memcpy」，在每秒几十万条消息的场景里很值；一次性读配置文件则毫无意义，只会给结构体加个生命周期参数、把代码搞复杂。

实际工程里常见的折中是 `Cow<'a, str>`：输入是 JSON 字符串且没有转义时借用，有转义（`\n`、`\uXXXX`）时分配。serde_json 的 `RawValue` 更进一步，连结构都不解析，直接把原始 JSON 片段借出来。

### untagged 与 flatten 的隐藏成本

这两个属性都要求「先试再决定」，而流式解析器是单向的，试错就没法回退。
于是 serde 的实现是：**先把输入完整解析成一个中间的 `Content` 枚举**（本质是一个带所有形状的通用树），再在这个树上逐个尝试。

后果有三个：

1. 内存翻倍：输入数据要被物化一遍，大消息（几 MB 的 JSON）会明显感觉到。
2. 速度下降：多了一次完整遍历与分配，通常比直接解析慢 1.5–3 倍。
3. 与非自描述格式不兼容：bincode 无法在没有类型信息的情况下产出 `Content`，会直接报错。

所以 `untagged` 适合「小消息、形状确实不定」的场合（webhook 事件、多态配置），不适合塞进高频热路径。

### 性能账：本机实测

以下是把同一个结构体（4 个字段、含 `Vec<String>`、序列化后 62 字节）处理 20 万次的结果（rustc 1.100.0、release 模式、serde_json 1.0.154、单机观测值，仅供量级参考）：

| 路径 | 耗时 | 相对 to_string |
| --- | --- | --- |
| to_writer 到复用缓冲区 | 26.4 ms | 0.70× |
| to_string | 37.6 ms | 1.00× |
| to_string_pretty | 56.6 ms | 1.50× |
| to_value | 152.4 ms | 4.05× |
| from_str | 94.2 ms | — |
| from_value（含 Value 克隆） | 170.2 ms | — |

读法：

- `to_writer` 比 `to_string` 快约 30%，因为它写进你复用的 `Vec<u8>` / `BufWriter`，没有每行一次分配。批量写日志、写 socket 时应该用 `to_writer` 配 `BufWriter`，别拼接字符串。
- `to_string_pretty` 贵 50%，只该用在人看的配置文件与调试输出里。
- `to_value` 贵 4 倍：它把类型化的值先摊成通用树，每个字段都要装箱（`Box<str>`、`Vec<Value>`）。只有在你确实要动态改字段时才用 `Value`，热路径上别绕道。
- `from_str` 处理 62 字节约 470 ns（94.2 ms / 20 万），吞吐约 130 MB/s；同尺寸下 `from_value` 反而更慢，因为要先建树再取值。

这些数字会随编译器和机器变化，但比例关系相当稳定，可以作为选 API 的依据。

### `#[serde(with)]`：给第三方类型补实现

`chrono::DateTime<Utc>` 默认序列化成 RFC3339 字符串，但很多 API 要 Unix 时间戳。serde 不允许你给外部类型写 impl（孤儿规则），于是提供了字段级替换：

```rust,noplayground
use chrono::{DateTime, Utc};

#[derive(Debug, Serialize, Deserialize)]
struct Event {
    id: u64,
    #[serde(with = "chrono::serde::ts_seconds")]
    at: DateTime<Utc>,
}
```

`with` 指向一个模块，该模块必须导出签名匹配的 `serialize` 与 `deserialize` 两个函数。自己写也就十来行：

```rust,noplayground
mod as_millis {
    use chrono::{DateTime, TimeZone, Utc};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(t: &DateTime<Utc>, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_i64(t.timestamp_millis())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<DateTime<Utc>, D::Error> {
        let ms = i64::deserialize(d)?;
        Ok(Utc.timestamp_millis_opt(ms).single().ok_or_else(|| {
            serde::de::Error::custom("时间戳超出可表示范围")
        })?)
    }
}
```

`with` 模块必须同时提供 `serialize` 和 `deserialize` 两个函数——只写一个时，derive 会在反序列化方向找不到函数：

```rust,noplayground
mod as_millis {
    use serde::Serializer;
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(*v)
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Event {
    #[serde(with = "as_millis")]
    at: u64,
}
```

```text
error[E0425]: cannot find function `deserialize` in module `as_millis`
  --> src/main.rs:10:20
   |
10 |     #[serde(with = "as_millis")]
   |                    ^^^^^^^^^^^ not found in `as_millis`
   |
note: similarly named function `serialize` defined here
  --> src/main.rs:3:5
   |
 3 |     pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
   |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^
help: a function with a similar name exists
   |
10 -     #[serde(with = "as_millis")]
10 +     #[serde(with = serialize)]
```

报错指向属性而不是函数定义，第一眼容易懵。若这个类型只需要写、不需要读，把 derive 换成只 derive `Serialize`，或者用 `#[serde(serialize_with = "as_millis::serialize")]` 只替换写方向。

同理还有 `#[serde(serialize_with = "path")]` 和 `#[serde(deserialize_with = "path")]`，只替换一个方向时用它们。

## 29.8 典型错误案例

### 案例 1：忘了 derive，编译器让你补

```rust,noplayground
use serde::Serialize;

#[derive(Debug)]
struct Config {
    host: String,
    port: u16,
}

fn main() {
    let c = Config { host: "localhost".to_string(), port: 8080 };
    let s = serde_json::to_string(&c).unwrap();
    println!("{s}");
}
```

```text
error[E0277]: the trait bound `Config: serde::Serialize` is not satisfied
    --> src/main.rs:11:35
     |
  11 |     let s = serde_json::to_string(&c).unwrap();
     |             --------------------- ^^ unsatisfied trait bound
     |             |
     |             required by a bound introduced by this call
     |
help: the trait `Serialize` is not implemented for `Config`
    --> src/main.rs:4:1
     |
   4 | struct Config {
     | ^^^^^^^^^^^^^
     = note: for local types consider adding `#[derive(serde::Serialize)]` to your `Config` type
     = note: for types from other crates check whether the crate offers a `serde` feature flag
note: required by a bound in `serde_json::to_string`
     --> serde_json-1.0.154/src/ser.rs:2247:17
      |
 2247 |     T: ?Sized + Serialize,
      |                 ^^^^^^^^^ required by this bound in `to_string`
```

**原因**：`to_string<T>` 的约束是 `T: Serialize`，而 `Config` 只有 `Debug`。
注意错误不是指向 `use serde::Serialize;`——导入 trait 名和**为类型实现**这个 trait 是两回事。

**修复**：加 derive。若第 4 行的结构体确实无法改（比如来自别的 crate），就要用 `#[serde(remote = "...")]` 或手写 impl。

```rust,noplayground
#[derive(Debug, Serialize)]
struct Config {
    host: String,
    port: u16,
}
```

顺带一提，note 里那句「consider adding #[derive(serde::Serialize)]」是新版 rustc 里 serde 专门注册的诊断建议，非常省事——照着改基本一定对。

### 案例 2：借用了局部变量还想把它返回

```rust,noplayground
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Borrowed<'a> {
    name: &'a str,
}

fn parse() -> Borrowed<'static> {
    let input = String::from(r#"{"name":"rust"}"#);
    serde_json::from_str(&input).unwrap()
}

fn main() {
    println!("{:?}", parse().name);
}
```

```text
error[E0515]: cannot return value referencing local variable `input`
  --> src/main.rs:10:5
   |
10 |     serde_json::from_str(&input).unwrap()
   |     ^^^^^^^^^^^^^^^^^^^^^------^^^^^^^^^^
   |     |                    |
   |     |                    `input` is borrowed here
   |     returns a value referencing data owned by the current function
```

**原因**：`Borrowed<'a>` 里的 `&'a str` 直接指向输入缓冲区，而 `input` 是函数局部变量，函数返回时它就释放了。
这里还有第二处错：返回类型写的是 `Borrowed<'static>`，就算 `input` 是 `'static` 也未必对——`from_str` 推断出的生命周期由参数决定，不会自动延长。

**修复**：零拷贝的结构体只能借用在调用方手里活得够久的数据。三条路：

```rust,noplayground
// 1) 输入由调用方提供，生命周期自然对得上
fn parse(input: &str) -> Borrowed<'_> {
    serde_json::from_str(input).unwrap()
}

// 2) 用拥有所有权的字段
#[derive(Debug, Deserialize)]
struct Owned { name: String }

// 3) 用 Cow：能借就借，需要转义时才分配
#[derive(Debug, Deserialize)]
struct MaybeBorrowed<'a> {
    #[serde(borrow)]
    name: std::borrow::Cow<'a, str>,
}
```

### 案例 3：字段缺失时的运行期错误

```rust,noplayground
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Config {
    host: String,
    port: u16,
}

fn main() {
    let r: Result<Config, _> = serde_json::from_str(r#"{"host":"localhost"}"#);
    println!("{:?}", r.err());
}
```

```text
Some(Error("missing field `port`", line: 1, column: 20))
```

**原因**：`u16` 没有默认值，serde 认为缺字段是数据错误。列号 20 指向对象的收尾 `}`，而不是字段 `host` 的位置——这是 serde_json 报告「对象解析完了但字段不全」的方式，别按列号去找字段。

**修复**：给缺失的字段一个明确的默认值，三选一。

```rust,noplayground
#[derive(Debug, Deserialize)]
struct Config {
    host: String,
    #[serde(default = "default_port")]   // 缺失时调用函数
    port: u16,
}

fn default_port() -> u16 { 8080 }

// 或者：字段类型用 Option<u16>，缺失就是 None
// 或者：整个结构体加 #[serde(default)]，要求实现 Default
```

选哪种取决于业务语义：`port` 有公认默认值就用函数；「没配置」和「配置为空」有区别就用 `Option`。

### 案例 4：JSON 里的字符串转换不成整数

```rust,noplayground
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct Config {
    host: String,
    port: u16,
}

fn main() {
    let r: Result<Config, _> = serde_json::from_str(r#"{"host":"localhost","port":"8080"}"#);
    println!("{:?}", r.err());
}
```

```text
Some(Error("invalid type: string \"8080\", expected u16", line: 1, column: 33))
```

**原因**：serde 不做隐式类型转换。`u16` 的 Visitor 只实现整数访问器，遇到 JSON 字符串直接拒绝。
这和 TOML 里 `port = "8080"` 失败是同一回事，也是很多「配置文件明明有值却启动失败」的根因。
列号 33 指向字符串 `"8080"` 的右引号。

**修复**：要么改数据源（推荐），要么写一个既收整数又收字符串的类型。29.5 的 `Port` 就是完整答案；如果只有一处需要，用 `#[serde(deserialize_with = "...")]` 挂一个小函数更轻：

```rust,noplayground
fn de_u16_from_any<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw { Num(u16), Text(String) }
    match Raw::deserialize(d)? {
        Raw::Num(n) => Ok(n),
        Raw::Text(s) => s.parse().map_err(serde::de::Error::custom),
    }
}
```

### 案例 5：内部标签遇到 newtype 变体

```rust,noplayground
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Event {
    Click { x: i32 },
    Text(String),
}

fn main() {
    let e = Event::Text("hi".into());
    match serde_json::to_string(&e) {
        Ok(s) => println!("ok: {s}"),
        Err(err) => println!("err: {err}"),
    }
}
```

```text
err: cannot serialize tagged newtype variant Event::Text containing a string
```

**原因**：内部标签要求把变体的数据「合并进」同一层对象，所以数据必须是键值对（结构体变体）。`String` 是标量，无处安放，于是运行期报错——注意这是**运行期**错误，编译器不会阻止你，测试里不跑到这条分支就发现不了。

**修复**：三种改法，按协议要求选。

```rust,noplayground
// 1) 把 newtype 改成结构体变体
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum Event {
    Click { x: i32 },
    Text { body: String },   // {"type":"Text","body":"hi"}
}

// 2) 换成相邻标签，任何形状都能用
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type", content = "data")]
enum Adj {
    Click { x: i32 },
    Text(String),            // {"type":"Text","data":"hi"}
}

// 3) 回到默认的外部标签
#[derive(Debug, Serialize, Deserialize)]
enum Ext {
    Click { x: i32 },
    Text(String),            // {"Text":"hi"}
}
```

## 29.9 动手练习

**练习 1（格式无关）**：把 29.4 的 `Config` 做成既能从 JSON 读、也能从 TOML 读。
提示：`toml::from_str::<Config>` 与 `serde_json::from_str::<Config>` 都能用，因为 `Config` 只依赖 serde 的数据模型。写两个单测，同一个结构体、两份不同格式的输入，断言结果相等。

**练习 2（枚举表示）**：定义一个表示「任务状态」的枚举：`Pending`、`Running { pid: u32 }`、`Done { exit_code: i32 }`。
分别用外部标签、内部标签、相邻标签序列化一次，把三份 JSON 并排打出来。然后回答：`Pending` 在三种表示下分别长什么样？哪种表示能让 `Pending` 也带上字段（例如 `Pending { since: u64 }`）而不用改协议形状？

**练习 3（自定义解析）**：写一个 `RetryDelay` 类型，既能接受整数秒（`30`），也能接受带单位的字符串（`"30s"`、`"5m"`、`"2h"`）。
要求：`14.9` 这种非法输入要报出带上下文的错误；序列化时统一写成带单位的字符串。提示：29.5 的 `Port` 结构可以直接照搬，`visit_str` 里按后缀分支。

**练习 4（流式读写）**：写一个程序，用 `to_writer` 加 `BufWriter` 把 10 万条 `Row` 写成 NDJSON 文件，再用 `Deserializer::from_reader(...).into_iter::<Row>()` 读回来，断言条数、首尾元素，并打印两者耗时。
提示：`BufWriter` 的 `into_inner()` 或显式 `drop` 才能保证缓冲刷盘；读的时候用 `BufReader` 包一层，否则每次 `read` 只拿到几十字节。

**练习 5（配置演进）**：现有 v1 配置 `{"name": "svc"}`，你要引入 v2 增加 `retries`（默认 3）、把 `name` 改名为 `service`、并新增可选 `tags`。
要求：v1 文件能直接读进来；写出时是新格式；`retries: 0` 必须显式写出（提示：`skip_serializing_if` 只能跳过空集合，不能跳过 `0`）。最后写一个「读 v1 → 写 v2 → 再读 v2」的往返测试。

## 29.10 小结

1. serde 的价值是把 N×M 的转换问题拆成「类型 → 数据模型」和「数据模型 → 格式」两半，一份 derive 通吃 JSON/TOML/YAML/MessagePack。
2. 类型只实现 `Serialize` / `Deserialize`，格式只实现 `Serializer` / `Deserializer`；`Deserialize<'de>` 的生命周期参数是零拷贝的入口。
3. 属性分三层：容器（整个 struct/enum）、字段、变体。`rename_all`、`default`、`deny_unknown_fields` 在容器上；`alias`、`skip_serializing_if`、`flatten`、`with` 在字段上。
4. 枚举四种表示：默认外部标签最百搭；内部标签 `tag` 最像「类型字段」但只支持结构体变体；相邻标签 `tag + content` 形状任意；`untagged` 按结构匹配、错误信息最差。
5. 缺字段报 `missing field`，多字段默认**静默忽略**，类型不对报 `invalid type`——这三条默认行为决定了你的配置迁移策略。
6. `#[serde(default)]` 只对「缺失」生效，不对「解析失败」生效；配置写错就该失败，别用兜底掩盖。
7. 手写 `Deserialize` 的关键是 `Visitor`：按形状实现 `visit_u64`、`visit_str` 等方法，用 `E::custom` 补充上下文；`deserialize_any` 只适用于自描述格式。
8. `untagged` 与 `flatten` 都会先把输入物化成中间表示，带来内存与速度成本，并且与非自描述格式不兼容；不要放进热路径。
9. 本机实测（20 万条、62 字节/条）：`to_writer` 比 `to_string` 快约 30%，`to_value` 慢 4 倍，`from_str` 吞吐约 130 MB/s——批量输出用 `to_writer` + 缓冲，热路径别绕 `Value`。
10. `#[serde(with)]` 是给外部类型补实现的唯一干净办法，模块必须同时有 `serialize` 和 `deserialize`，只缺一个就会报 E0425。