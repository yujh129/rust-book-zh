# 第 8 章 结构体：把数据组织起来

> **本章目标**
>
> - 会给一组相关数据定义一个 `struct`，并理解"可变性属于整个实例、不属于单个字段"
> - 掌握字段初始化简写和结构体更新语法 `..base`，以及后者引起的部分移动
> - 分清具名结构体、元组结构体、单元结构体各自的使用场景
> - 会写 `impl` 块，说清 `&self`、`&mut self`、`self` 三种接收者的区别
> - 理解结构体在内存中的对齐与填充，知道 `#[repr(C)]` 解决什么问题

到第 7 章为止，我们手里的"数据"都还是零散的：一个 `i32`、一个 `String`、一个切片。真实程序里，数据总是成组出现的——一个矩形有宽和高，一个用户有名字和邮箱，一个 TCP 连接有本地地址、远端地址和超时。把这些字段捏成一个类型，是 Rust 从"能写脚本"走向"能写系统"的第一步。

## 8.1 为什么要结构体：三个平行变量的问题

先看一段不用结构体的代码：

```rust
fn area(width: u32, height: u32) -> u32 {
    width * height
}

fn main() {
    let width = 30;
    let height = 50;
    println!("面积 = {}", area(width, height));
}
```

这段代码能跑，但它把"一个矩形"拆成了两个互不相干的变量。麻烦从三个角度出现：

1. **调用点会写错顺序**。`area(h, w)` 编译器不报错，结果悄悄错了。
2. **函数签名不可读**。看 `fn draw(x1: i32, y1: i32, x2: i32, y2: i32, color: u32)`，你得去读文档才知道参数含义。
3. **无法给"矩形"挂方法**。`area(w, h)` 是自由函数，"计算矩形面积"这件事没法归到矩形自己身上。

结构体一次解决这三个问题：

```rust
struct Rectangle {
    width: u32,
    height: u32,
}

impl Rectangle {
    fn area(&self) -> u32 {
        self.width * self.height
    }
}

fn main() {
    let rect = Rectangle { width: 30, height: 50 };
    println!("面积 = {}", rect.area());
}
```

定义用 `struct 名字 { 字段: 类型, ... }`，**没有分号**。字段之间用逗号分隔，最后一行也可以留逗号——这是 Rust 的惯例写法，因为加字段时不用改上一行。

## 8.2 实例化、字段访问与可变性

实例化的语法是 `类型名 { 字段: 值, ... }`，字段顺序不必和定义一致：

```rust
struct Rectangle {
    width: u32,
    height: u32,
}

fn main() {
    let a = Rectangle { width: 30, height: 50 };
    let b = Rectangle { height: 50, width: 30 };   // 顺序无关
    println!("{} {}", a.width, b.height);
}
```

**关键细节：Rust 没有"字段级可变性"。** 一个实例要么整体可变，要么整体不可变：

```rust
struct Rectangle {
    width: u32,
    height: u32,
}

fn main() {
    let mut rect = Rectangle { width: 30, height: 50 };
    rect.width = 31;      // 可以：实例是 mut
    rect.height += 1;     // 可以
    println!("{} {}", rect.width, rect.height);

    let fixed = Rectangle { width: 1, height: 1 };
    // fixed.width = 2;   // 注释掉：error[E0596]，fixed 没有声明 mut
    println!("{}", fixed.width);
}
```

这一点和 C# 的 `readonly` 字段、TypeScript 的 `readonly` 修饰符都不一样：Rust 把"能不能改"这个属性放在**绑定**上，而不是放在类型或字段上。好处是借用检查器只需要看一处就能判断，坏处是你不能写"这个结构体前两个字段可写、后两个只读"——想那样做，就得把只读部分拆成另一个类型（第 36 章会讲怎么用类型系统表达这个约束）。

### 字段初始化简写

当变量名和字段名相同时，可以省略 `字段:`：

```rust
struct User {
    name: String,
    age: u8,
}

fn build_user(name: String, age: u8) -> User {
    User { name, age }     // 等价于 User { name: name, age: age }
}

fn main() {
    let u = build_user(String::from("ana"), 30);
    println!("{} {}", u.name, u.age);
}
```

### 结构体更新语法 `..`

想基于一个已有实例改几个字段、其余照抄，可以写：

```rust
#[derive(Debug)]
struct Config {
    host: String,
    port: u16,
    tls: bool,
}

fn main() {
    let dev = Config {
        host: String::from("127.0.0.1"),
        port: 8080,
        tls: false,
    };

    let prod = Config {
        host: String::from("example.com"),
        tls: true,
        ..dev
    };

    println!("{:?}", prod);
}
```

注意 `..dev` 放在**最后**，而且它会把 `dev` 中剩下的字段**移动**进 `prod`。也就是说 `dev.port` 是一个 `u16`（实现了 `Copy`），在这里被复制了；但如果剩下的字段里有 `String`，那个 `String` 就会被移动出去，`dev` 随之变成"部分移动"状态：

```rust
struct Config {
    host: String,
    port: u16,
}

fn main() {
    let dev = Config { host: String::from("127.0.0.1"), port: 8080 };
    let _prod = Config { host: String::from("example.com"), ..dev };
    // println!("{}", dev.port);   // 可以：port 是 Copy，没被移走
    // println!("{}", dev.host);   // 不行：host 已被移进 _prod
    println!("{}", _prod.port);
}
```

这是第 5 章"部分移动"规则在结构体上的直接体现：**每个字段各自有所有权，结构体本身没有额外的所有权语义**。

## 8.3 元组结构体与单元结构体

Rust 还有两种没有具名字段的结构体。

### 元组结构体：给元组一个有名字的类型

```rust
struct Color(u8, u8, u8);
struct Point(f64, f64);

fn main() {
    let black = Color(0, 0, 0);
    let origin = Point(0.0, 0.0);
    println!("{} {} {}", black.0, black.1, black.2);
    println!("({}, {})", origin.0, origin.1);
}
```

它的访问方式和小写 `tuple` 一样用 `.0`、`.1`，但它是**一个独立类型**。这一点在跨模块、跨 crate 的 API 里极其重要：

```rust
struct Meters(f64);
struct Seconds(f64);

fn main() {
    let d = Meters(100.0);
    let t = Seconds(9.58);
    // let v = d.0 / t.0;   // 取 .0 就回到了裸 f64，类型保护失效
    let v = d.0 / t.0;      // 这样写编译器不会拦你，所以别这么写
    println!("{}", v);
}
```

上面这段是"反面教材"：一旦取出 `.0`，单位信息就丢了。正确做法是给 `Meters` 实现方法，让运算发生在类型层面：

```rust
#[derive(Debug, Clone, Copy)]
struct Meters(f64);

impl Meters {
    fn per_second(self, seconds: f64) -> Meters {
        Meters(self.0 / seconds)
    }
}

fn main() {
    let distance = Meters(100.0);
    println!("{:?}", distance.per_second(9.58));
}
```

这种"只包一个字段的结构体"叫 **newtype**，是 Rust 里最常用的设计手法之一：给原始类型加一层类型标签，编译期就能阻止"把秒当米"的错误，运行期零开销（第 20 章会从内存布局角度再确认这一点）。

### 单元结构体：只有类型，没有数据

```rust
struct Marker;

trait Plugin {
    fn name(&self) -> &'static str;
}

impl Plugin for Marker {
    fn name(&self) -> &'static str {
        "marker"
    }
}

fn main() {
    println!("{}", Marker.name());
}
```

`struct Marker;` 不占任何字节（`size_of::<Marker>() == 0`），它的价值全在类型层面：用来实现 trait、作为泛型参数、或者作为状态机的"状态"标记（第 36 章类型状态模式会大量用到）。

## 8.4 impl 块：方法与关联函数

方法定义在 `impl` 块里，第一个参数是 `self` 的某种形式：

```rust
#[derive(Debug)]
struct Rectangle {
    width: u32,
    height: u32,
}

impl Rectangle {
    // 关联函数：没有 self，用 :: 调用
    fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    fn square(side: u32) -> Self {
        Self::new(side, side)
    }

    // &self：只读借用
    fn area(&self) -> u32 {
        self.width * self.height
    }

    // &mut self：可变借用
    fn scale(&mut self, factor: u32) {
        self.width *= factor;
        self.height *= factor;
    }

    // self：拿走所有权
    fn into_parts(self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn can_hold(&self, other: &Rectangle) -> bool {
        self.width >= other.width && self.height >= other.height
    }
}

fn main() {
    let mut r = Rectangle::new(4, 3);
    println!("面积 {}", r.area());
    r.scale(2);
    println!("放大后 {:?}", r);

    let s = Rectangle::square(5);
    println!("能放下吗：{}", r.can_hold(&s));

    let (w, h) = Rectangle::square(2).into_parts();
    println!("拆开：{} {}", w, h);
}
```

三种接收者的选择是本章最重要的一件事，它直接决定了调用方能不能继续使用这个值：

| 接收者 | 用法 | 调用后原值还能用吗 | 典型场景 |
|---|---|---|---|
| `&self` | 只读访问 | 能 | `area`、`len`、`to_string` |
| `&mut self` | 修改内部状态 | 能（但调用期间独占） | `push`、`scale`、`clear` |
| `self` | 消费掉自己 | 不能 | `into_parts`、`into_iter`、`build` |

一个经验规则：**默认写 `&self`，需要改就写 `&mut self`，只有"这个对象的使命就是变成别的东西"时才写 `self`**。Rust 标准库里的 `Into::into`、`Iterator::collect`、`String::into_bytes` 都属于第三类，命名也统一带 `into_` 前缀。

### 多个 impl 块

一个类型可以有任意多个 `impl` 块，编译器会把它们合并看待：

```rust
struct Config {
    host: String,
    port: u16,
}

impl Config {
    fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

impl Config {
    fn is_local(&self) -> bool {
        self.host == "127.0.0.1" || self.host == "localhost"
    }
}

fn main() {
    let c = Config { host: String::from("localhost"), port: 80 };
    println!("{} {}", c.addr(), c.is_local());
}
```

实际项目里常见的拆分方式是：一个 `impl` 放构造函数，一个放核心逻辑，再来一个 `impl Debug for ...`——按职责分块，而不是按编译器要求分块。

### 方法调用的自动引用

你写 `rect.area()`，而 `area` 的签名是 `fn area(&self)`，两者对不上，但编译器会自动补上 `&`：

```rust
struct Counter {
    n: u32,
}

impl Counter {
    fn get(&self) -> u32 { self.n }
    fn bump(&mut self) { self.n += 1; }
}

fn main() {
    let mut c = Counter { n: 0 };
    c.bump();              // 编译器补成 Counter::bump(&mut c)
    println!("{}", c.get());  // 编译器补成 Counter::get(&c)
}
```

这条规则叫 **自动引用/自动解引用**（auto-ref / auto-deref）。它让 `&`, `&mut`, 值三种形态在调用点看起来一样，但也意味着你**不能**靠"加不加 `&`"来改变调用的方法——方法解析先看类型再看接收者。如果你同时写了 `fn get(&self)` 和 `fn get(self)`，编译器会报"重复定义"。

## 8.5 深入：结构体在内存里到底是什么

这一节回答三个常被跳过的问题：字段之间有洞吗？字段顺序会影响大小吗？`Option<T>` 真的会多占一个字节吗？

### 对齐与填充

CPU 访问内存时，一个 `u32` 最好放在 4 的倍数地址上。为了让每个字段都落在"自然对齐"的位置，编译器会在字段之间插入**填充字节**（padding）：

```rust
use std::mem::{align_of, size_of};

struct Pair(u8, u32);

fn main() {
    println!("size = {}, align = {}", size_of::<Pair>(), align_of::<Pair>());
}
```

输出是 `size = 8, align = 4`：`u8` 占 1 字节，后面补 3 字节，再放 `u32`。那 3 个字节不存任何数据，纯粹为了对齐。

### 字段重排：`repr(Rust)` 的秘密

默认布局叫 `repr(Rust)`，它**不保证字段顺序**。编译器可以随意重排字段来减少填充：

```rust
use std::mem::size_of;

struct Mixed {
    a: u8,
    b: u32,
    c: u8,
}

fn main() {
    println!("{}", size_of::<Mixed>());
}
```

如果你按"源码顺序"心算：1 + 3补 + 4 + 1 + 3补 = 12。但实际输出是 **8**——编译器把 `b` 挪到了最前面，两个 `u8` 紧挨着放在后面（4 + 1 + 1 + 2补 = 8）。

这个优化是免费的，代价是**布局不再稳定**：同一份源码用不同版本的 rustc 编译，字段偏移可能不同。所以 Rust 明确告诉你：`repr(Rust)` 的布局只能自己用，不能拿去和 C 交互、不能用来做内存映射、不能靠它做序列化。

### `#[repr(C)]`：把布局钉死

需要稳定布局时（FFI、二进制协议、`memmap`），加上 `#[repr(C)]`：

```rust
use std::mem::size_of;

#[repr(C)]
struct Header {
    version: u8,
    length: u32,
    flags: u8,
}

fn main() {
    println!("{}", size_of::<Header>());   // 12：字段按声明顺序排，中间补 3+3 字节
}
```

输出是 12。`repr(C)` 保证字段按声明顺序排列、填充规则与 C 编译器一致，同时**关闭了重排优化**。所以 FFI 结构体要尽量把字段按"从大到小"排列，能省下不少填充。

还有一个 `#[repr(align(N))]` 用来强制对齐，典型用途是避免伪共享：

```rust
use std::mem::size_of;

#[repr(C, align(64))]
struct CacheLineAligned {
    counter: u64,
}

fn main() {
    println!("{}", size_of::<CacheLineAligned>());   // 64
}
```

两个线程各自改一个 `u64`，如果它们落在同一条 64 字节缓存行上，CPU 会不停地把这条缓存行在核心之间来回搬（false sharing）。把计数器按 64 字节对齐后，各占一行，性能能差好几倍。第 22 章讲 `AtomicUsize` 时还会回到这里。

### niche 优化：为什么 `Option<&T>` 不占额外空间

`Option<T>` 直觉上应该是"一个标记 + 一个 T"。对 `Option<i32>` 确实如此，但对引用类型不是：

```rust
use std::mem::size_of;

fn main() {
    println!("&u8        = {}", size_of::<&u8>());
    println!("Option<&u8>= {}", size_of::<Option<&u8>>());
    println!("Option<i32>= {}", size_of::<Option<i32>>());
}
```

输出是 `8 / 8 / 8`。原因是引用的合法值里**没有 0**（空指针不是合法引用），编译器就拿 `0` 当 `None`，于是 `Option<&u8>` 和 `&u8` 一样大。这个技巧叫 **niche 优化**，标准库把它用在 `Box<T>`、`Vec<T>`、`&str`、`NonZeroU32` 等类型上。这也是为什么 `Option<Box<T>>` 可以零成本地表达"可能没有"。

对 `i32` 就不行：`i32` 的所有位模式都是合法值，没有空位当 `None`，所以只能额外加一个标记（这里是 4 字节）。如果你确实需要一个"可以不存在的 u32"，用 `Option<NonZeroU32>`，它同样只占 4 字节。

## 8.6 derive：让编译器替你写样板代码

`#[derive(...)]` 是最常用的属性之一：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct User {
    name: String,
    age: u8,
    active: bool,
}

fn main() {
    let a = User { name: String::from("ana"), age: 30, active: true };
    let b = a.clone();
    println!("{:?}", a);
    println!("相等吗：{}", a == b);

    let empty = User::default();
    println!("{}", empty.age);
}
```

几个常用 derive 的实际含义：

| derive | 生成什么 | 注意点 |
|---|---|---|
| `Debug` | `{:?}` 的格式化实现 | 调试必备，几乎每个结构体都该加 |
| `Clone` | `.clone()` | 是深拷贝，成本可能很高 |
| `Copy` | 赋值/传参时按位复制 | 要求所有字段都是 `Copy`，且不能再有 `Drop` |
| `PartialEq` / `Eq` | `==` / 等价关系 | 有浮点字段时只能 `PartialEq` |
| `Hash` | 可作 `HashMap` 的键 | 必须和 `Eq` 的语义一致 |
| `Default` | `::default()` | 所有字段都要实现 `Default` |

`Copy` 和 `Clone` 的关系值得单独说一句：`Copy` 是 `Clone` 的子 trait，实现了 `Copy` 就自动获得 `clone()`，但反过来不成立。一个类型只要实现了 `Copy`，它的赋值就不再是"移动"，而是"复制"，原变量仍然可用：

```rust
#[derive(Debug, Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 2 };
    let q = p;          // Copy：p 没有失效
    println!("{:?} {:?}", p, q);
}
```

**经验规则：符合"小而纯的数据"就实现 `Copy`（坐标、颜色、ID），只要包含 `String`、`Vec`、`Box` 或需要 `Drop` 就只实现 `Clone`。** 给一个大结构体加 `Copy`，你会在某次 `match` 里莫名其妙地复制几百字节。

## 典型错误案例

下面五个错误是写结构体时最常撞上的，报错文本来自 rustc 1.100.0 的真实输出。

### 案例 1：漏写字段——E0063

```rust,editable
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1 };
    println!("{}", p.x);
}
```

```text
error[E0063]: missing field `y` in initializer of `Point`
 --> src/main.rs:7:13
  |
7 |     let p = Point { x: 1 };
  |             ^^^^^ missing `y`

error: aborting due to 1 previous error
```

**原因**：结构体字面量必须给出**全部**字段。Rust 没有"字段默认值"，也没有可选字段语法。

**修复**：补上字段，或者实现 `Default` 后用 `..Default::default()` 让其余字段取默认值：

```rust,editable
#[derive(Default)]
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, ..Default::default() };
    println!("{} {}", p.x, p.y);
}
```

### 案例 2：字段名拼错——E0560

```rust,editable
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 2, z: 3 };
    println!("{}", p.x);
}
```

```text
error[E0560]: struct `Point` has no field named `z`
 --> src/main.rs:7:33
  |
7 |     let p = Point { x: 1, y: 2, z: 3 };
  |                                 ^ `Point` does not have this field
  |
  = note: all struct fields are already assigned

error: aborting due to 1 previous error
```

**原因**：字面量里出现了定义中没有的字段名。`note: all struct fields are already assigned` 这句话说明编译器已经识别出"该有的都有了，多出来一个"。

**修复**：删掉多余的字段，或者检查是不是该给结构体加这个字段。

### 案例 3：想改字段但绑定不可变——E0596

```rust,editable
struct Counter {
    count: u32,
}

impl Counter {
    fn incr(&mut self) {
        self.count += 1;
    }
}

fn main() {
    let c = Counter { count: 0 };
    c.incr();
    println!("{}", c.count);
}
```

```text
error[E0596]: cannot borrow `c` as mutable, as it is not declared as mutable
  --> src/main.rs:13:5
   |
13 |     c.incr();
   |     ^ cannot borrow as mutable
   |
help: consider changing this to be mutable
   |
12 |     let mut c = Counter { count: 0 };
   |         +++

error: aborting due to 1 previous error
```

**原因**：`incr(&mut self)` 需要一个可变借用，而 `c` 是 `let` 绑定的不可变变量。**错误发生在调用点，不是定义点**——所以初学者常常去改 `impl`，改不对。

**修复**：把绑定改成 `let mut c`。如果这个值在多个地方共享，就不能简单加 `mut`，得考虑 `Cell`/`RefCell`（第 19 章）或者重新设计接口。

### 案例 4：方法里想返回字段却没交出所有权——E0507

```rust,editable
struct User {
    name: String,
}

impl User {
    fn name(&self) -> String {
        self.name
    }
}

fn main() {
    let u = User { name: String::from("ana") };
    println!("{}", u.name());
}
```

```text
error[E0507]: cannot move out of `self.name` which is behind a shared reference
 --> src/main.rs:7:9
  |
7 |         self.name
  |         ^^^^^^^^^ move occurs because `self.name` has type `String`, which does not implement the `Copy` trait
  |
help: consider cloning the value if the performance cost is acceptable
  |
7 |         self.name.clone()
  |                  ++++++++

error: aborting due to 1 previous error
```

**原因**：`&self` 只是借用，借来的东西不能"搬走"。`String` 不是 `Copy`，所以 `self.name` 这个表达式想移动所有权，编译器不允许。

**修复**：三种合法写法，按场景选：

```rust,editable
struct User {
    name: String,
}

impl User {
    fn name(&self) -> &str {          // 方案一：返回引用，零成本
        &self.name
    }

    fn name_owned(&self) -> String {  // 方案二：显式克隆
        self.name.clone()
    }

    fn into_name(self) -> String {    // 方案三：消费 self，真搬走
        self.name
    }
}

fn main() {
    let u = User { name: String::from("ana") };
    println!("{}", u.name());
    println!("{}", u.name_owned());
    println!("{}", u.into_name());
}
```

### 案例 5：`{:?}` 但没有 `Debug`——E0277

```rust,editable
struct Point {
    x: i32,
    y: i32,
}

fn main() {
    let p = Point { x: 1, y: 2 };
    println!("{:?}", p);
}
```

```text
error[E0277]: `Point` doesn't implement `Debug`
 --> src/main.rs:8:22
  |
8 |     println!("{:?}", p);
  |               ----   ^ `Point` cannot be formatted using `{:?}` because it doesn't implement `Debug`
  |               |
  |               required by this formatting parameter
  |
  = help: the trait `Debug` is not implemented for `Point`
help: consider annotating `Point` with `#[derive(Debug)]`
  |
1 + #[derive(Debug)]
2 | struct Point {
  |

error: aborting due to 1 previous error
```

**原因**：`{:?}` 需要 `Debug` trait，而结构体默认不实现任何 trait。Rust 的原则是"不替你猜语义"，连打印格式都要显式声明。

**修复**：在定义上方加 `#[derive(Debug)]`。注意 `{}`（Display）**不能** derive，需要手写 `impl std::fmt::Display`——因为 Display 面向用户，格式属于业务决策。

## 动手练习

**练习 1**：给 `Rectangle` 加三个方法：`perimeter(&self) -> u32`、`is_square(&self) -> bool`、`shrink(&mut self, by: u32)`（宽高各减 `by`，但不能减到 0 以下，用 `saturating_sub`）。写完之后用 `cargo clippy` 看看有没有更好的写法。

**练习 2**：用 newtype 实现温度换算。要求 `Celsius` 和 `Fahrenheit` 都是独立类型，提供 `Celsius::to_fahrenheit(self) -> Fahrenheit` 和反向方法，并且**禁止**在 `main` 里直接对裸 `f64` 做换算。提示：`F = C * 9/5 + 32`。

**练习 3**：下面这个结构体占多少字节？试着只调整字段顺序（不删字段），把它压到最小：

```rust,noplayground
struct Packet {
    version: u8,
    timestamp: u64,
    kind: u8,
    length: u32,
    flags: u16,
}
```

写完用 `size_of::<Packet>()` 打印验证，再用 `#[repr(C)]` 版本对比一次，解释差异。

**练习 4**：实现一个泛型队列 `Queue<T>`，只允许用 `Vec<T>` 存储，提供 `push`、`pop`（FIFO）、`len`、`is_empty` 四个方法。要求 `pop` 的复杂度是 O(n) 可以接受，但不能用 `Vec::remove(0)` 之外的标准库集合类型。

**练习 5**：这段代码为什么编译不过？先自己猜错误码，再实际编译验证：

```rust,noplayground
#[derive(Clone, Copy)]
struct Badge {
    id: u32,
    label: String,
}
```

提示：`Copy` 要求所有字段都是 `Copy`，而 `String` 管理堆内存、实现了 `Drop`，无法按位复制。

## 小结

- `struct` 把相关数据捏成一个类型，解决"参数顺序写错""签名不可读""方法无处安放"三个问题
- 可变性属于绑定，不属于字段：要么整体 `mut`，要么整体不可变
- 字段初始化简写 `User { name, age }` 省掉同名重复；`..base` 会把剩余字段移动走，之后 `base` 处于部分移动状态
- 元组结构体给元组一个独立类型名，最常见的用法是 newtype：`struct Meters(f64)` 让"米"和"秒"在编译期就不可混用
- 单元结构体不占空间，只承担类型标记的职责
- `&self` 只读、`&mut self` 独占修改、`self` 消费；默认用 `&self`，命名 `into_` 前缀的函数才消费
- 默认布局 `repr(Rust)` 会重排字段以压缩体积，但不保证稳定；要让布局固定下来（FFI、二进制协议）就加 `#[repr(C)]`
- 对齐填充会让结构体比"字段大小之和"大；`#[repr(align(64))]` 用于避免伪共享，`Option<&T>` 借助 niche 优化不额外占空间
- `#[derive]` 只能生成"有唯一正确答案"的 trait；`Display` 需要手写，因为格式属于业务决策
