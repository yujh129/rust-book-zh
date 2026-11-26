# 第 4 章 复合类型：元组、数组、切片与字符串

> **本章目标**
> - 用元组把不同类型的值打包，并通过位置或解构取出
> - 掌握 `[T; N]` 的定长语义、初始化方式与越界行为
> - 会用 `&arr[..]` 取切片，并说清切片为什么占两个机器字
> - 分清 `String`、`&str` 与 `str`，知道函数签名该写哪个
> - 从内存布局解释数组、元组与 `String` 的字节构成
> - 认出 5 个复合类型相关的典型编译错误

第 3 章的函数只能返回一个值，一个变量也只能装一个标量。真实程序里，"一个坐标""一张固定长度的查找表""一段用户输入"都是常见单位，需要语法层面的容器把它们装在一起。

Rust 提供三种最基础的组合方式。元组（tuple）把数量固定、类型可以不同的值按位置排在一起；数组（array）把类型相同、长度固定的值连续存放；字符串是"字节缓冲 + UTF-8 编码约定"的组合。元组和数组直接内联存放元素，元素本身仍可拥有堆内存，最后一种的所有权形态需要单独讨论。

结构体要到第 8 章才讲，所以本章的容器都带着明显的局限：元组的字段没有名字，数组的长度不能改。先用它们表达"临时组合"与"定长表格"，等结构体和 `Vec` 登场时，你才有依据判断什么时候该换更重的工具。

## 4.1 三种复合类型的定位

先看一个把三者都用上的例子：

```rust
fn main() {
    let point = (3, 4);                       // 元组：两个 i32
    let names = ["Alice", "Bob", "Carol"];    // 数组：三个 &str
    let greeting = String::from("hello");     // 字符串：拥有堆上的缓冲

    println!("{point:?}");
    println!("{names:?}");
    println!("{greeting}");
}
```

三者在"长度何时确定"和"元素类型是否必须相同"上完全不同，这决定了它们各自的用途：

| 类型 | 元素类型 | 长度确定时机 | 访问方式 | 典型用途 |
| --- | --- | --- | --- | --- |
| 元组 (T1, T2) | 可以不同 | 编译期 | t.0 或解构 | 多返回值、临时组合 |
| 数组 [T; N] | 相同 | 编译期 | a[i] | 定长表、缓冲区 |
| 切片 &[T] | 相同 | 运行期 | s[i] | 借用一段连续元素 |
| String 与 &str | u8（UTF-8） | 运行期 | 字节或字符迭代 | 文本 |

元组和数组都是"值类型"：赋值或传参时整体复制（元素实现 `Copy` 时），没有隐藏的堆指针。这一性质在 4.6 节会从字节层面展开。

## 4.2 元组：按位置打包

元组的类型写成括号里列出每个位置的类型：`(i32, f64, char)`。长度和每个位置的类型都是类型的一部分，`(i32, i32)` 与 `(i32, i32, i32)` 是两个不同的类型。

```rust
fn main() {
    let t: (i32, f64, char) = (500, 6.4, 'x');

    println!("{} {} {}", t.0, t.1, t.2);

    let (code, ratio, letter) = t;      // 解构：一次取出全部
    println!("{code} {ratio} {letter}");

    let (first, ..) = t;                // 只取第一个，其余忽略
    println!("{first}");
}
```

访问用 `t.0`、`t.1`，点号后面必须是字面量整数，不能是变量：`t[i]` 与 `t.i` 都不合法。这是元组与数组最直观的区别，也是本章案例 3 的来源。

解构（destructuring）是元组最常用的能力。`let (a, b) = pair;` 把两个位置分别绑定到新变量；`..` 表示"剩下的不管"，可以放在模式的开头、中间或结尾。

元组也可以嵌套，类型照写：`((i32, i32), &str)`。取内层用 `t.0.1` 这样的连续投影。

### 单元类型

只有一个值的元组 `()` 叫单元类型（unit type）。它同时是类型和值：

```rust
fn log(msg: &str) {
    println!("{msg}");
}

fn main() {
    let nothing: () = log("done");
    println!("{nothing:?}");
}
```

没有返回值的函数其实返回 `()`。`if`、`while`、`for`、赋值表达式、`println!` 的值也都是 `()`。所以 `let x = println!("hi");` 合法，`x` 的类型是 `()`。这条规则让"每个表达式都有类型"在语言层面成立，第 9 章讲 `match` 各分支类型必须一致时会再用到。

单元类型还有实际用途：泛型参数占位（`Result<(), Error>` 表示"成功时没有值"）、以及作为不关心内容的 map 值。第 10 章会大量出现 `Result<(), E>`。

### 元组作为多返回值

函数只能返回一个值，但返回一个元组就等于返回了多个：

```rust
fn min_max(xs: &[i32]) -> (i32, i32) {
    let mut lo = xs[0];
    let mut hi = xs[0];
    for &x in &xs[1..] {
        if x < lo { lo = x; }
        if x > hi { hi = x; }
    }
    (lo, hi)
}

fn main() {
    let (lo, hi) = min_max(&[3, 1, 4, 1, 5]);
    println!("最小 {lo}，最大 {hi}");
}
```

参数 `&[i32]` 是切片引用，数组和 `Vec` 都能传进来；4.4 节会解释它为什么比 `&[i32; 5]` 通用。

同样的功能在 Java 里要定义一个 `Pair` 类或返回 `int[]`，在 Go 里靠多返回值语法，在 Python 里返回 tuple。Rust 的元组是"匿名的、编译期展开的结构体"：没有运行时开销，但字段也没有名字——超过三个字段就该考虑结构体。

元组还能比较：元素类型都实现 `PartialOrd` 时，元组按字典序比较，先比第 0 个，相等再比第 1 个。

```rust
fn main() {
    println!("{}", (1, 5) < (2, 0));       // true
    println!("{}", ("a", 2) < ("a", 10));  // true
}
```

## 4.3 数组：长度是类型的一部分

数组类型写作 `[T; N]`：T 是元素类型，N 是长度，必须是编译期常量。`[i32; 5]` 与 `[i32; 6]` 不兼容，函数签名写死长度后也只能收这个长度。这是数组与 `Vec` 最根本的分歧：数组的长度信息在类型里，`Vec` 的在运行期的三个字段里。

数组的元素在内存里首尾相接，整个数组内联在持有它的值里——局部变量就在栈帧上。`[u8; 1024]` 就是 1 KiB 的栈空间，不是指向 1 KiB 的指针。

```rust
fn main() {
    let a: [i32; 5] = [1, 2, 3, 4, 5];
    let zeros = [0u8; 8];                 // 同一个值重复 N 次
    let grid = [[1, 2, 3], [4, 5, 6]];    // 二维数组：[[i32; 3]; 2]

    println!("{} {}", a.len(), zeros.len());
    println!("{grid:?}");
    println!("{}", grid[1][2]);
}
```

类型标注通常可以省略，编译器从初始化式推断。只有空数组不行：`let a = [];` 推不出元素类型，必须写 `let a: [i32; 0] = [];`。长度 0 的数组不是废设，泛型代码在边界情况下会用到它。

### 重复表达式与 Copy

`[expr; N]` 会把 expr 复制 N 份，所以 expr 必须满足二者之一：类型实现 `Copy`，或者 expr 是常量表达式。`[0u8; 1024]` 两者都满足。`[String::new(); 3]` 不满足：`String` 不是 `Copy`，`String::new()` 也不是常量，编译器不会替你克隆三次。

需要"每个元素独立构造"时用 `std::array::from_fn`：

```rust
fn main() {
    let squares: [i32; 8] = std::array::from_fn(|i| (i as i32) * (i as i32));
    let names: [String; 3] = std::array::from_fn(|i| format!("user{i}"));
    println!("{squares:?}");
    println!("{names:?}");
}
```

`from_fn` 的签名是 `pub fn from_fn<T, const N: usize, F>(cb: F) -> [T; N] where F: FnMut(usize) -> T`，按下标调用 N 次。注意 `array` 是模块名（`std::array`）而不是类型名；类型就叫数组。

### 索引与越界

`a[i]` 的 i 是 `usize`。索引表达式在运行时做边界检查：越界立即 panic，绝不会像 C 那样读到相邻内存。

```rust,noplayground
fn main() {
    let a = [1, 2, 3];
    let i: usize = std::env::args().nth(1).unwrap().parse().unwrap();
    println!("{}", a[i]);
}
```

```console
$ cargo run -- 3
thread 'main' panicked at src/main.rs:4:20:
index out of bounds: the len is 3 but the index is 3
```

下标改成字面量 `a[3]`，错误会提前到编译期：编译器能静态算出这次索引必然越界，于是直接拒绝编译。本章案例 2 会看到完整的报错。

不想 panic 就用 `get`，它返回 `Option<&T>`：

```rust
fn main() {
    let a = [1, 2, 3];
    let i = 5;
    println!("{:?}", a.get(1));     // Some(2)
    println!("{:?}", a.get(i));     // None
}
```

`get` 与 `[]` 的取舍是 Rust 的常见选择：确定的内部循环用 `[]`（失败即 bug，panic 合理），来自外部输入的下标用 `get`（越界是正常分支）。

### 遍历

```rust
fn main() {
    let a = [1, 2, 3];

    for x in &a {                 // 借用：x 是 &i32
        print!("{x} ");
    }
    println!();

    let doubled: Vec<i32> = a.into_iter().map(|x| x * 2).collect();
    println!("{doubled:?}");
}
```

Rust 2021 起，数组的 `into_iter()` 按值产出元素（`i32` 而不是 `&i32`），这是 edition 迁移时少数会改变行为的点之一。`a.iter()` 则总是产出 `&T`。

### 数组到切片

数组可以自动"退化"成切片：

```rust
fn sum(xs: &[i32]) -> i32 {
    xs.iter().sum()
}

fn main() {
    let a = [1, 2, 3];
    println!("{}", sum(&a));         // &[i32; 3] → &[i32]
    println!("{}", sum(&a[1..]));    // 只传后两个
}
```

`&a` 出现在需要 `&[i32]` 的位置时，编译器做的是无尺寸强制转换（unsized coercion）：把"指向数组的指针"换成"指针 + 长度"。数组没有被复制，也没有新分配。`Vec<i32>` 的 `&v` 同样能到 `&[i32]`，走的是另一条路（deref coercion），4.4 节会一并说明。

## 4.4 切片入门：借一段连续元素

切片（slice）类型写作 `[T]`，代表"一段长度在运行期才知道的连续元素"。`[T]` 没有固定大小，不能直接放进变量，只能通过引用使用：`&[T]` 或 `&mut [T]`。

```rust
fn main() {
    let a = [10, 20, 30, 40, 50];

    let all = &a[..];       // 全部
    let mid = &a[1..4];     // 下标 1、2、3
    let head = &a[..2];     // 前两个
    let tail = &a[2..];     // 从下标 2 到结尾

    println!("{all:?} {mid:?} {head:?} {tail:?}");
}
```

range 语法 `start..end` 是左闭右开：`1..4` 包含 1、2、3。省略的一端取边界，`..` 表示全长。下标类型是 `usize`，用变量也可以。

切片可以可变，只要原数组是可变的：

```rust
fn main() {
    let mut a = [1, 2, 3, 4, 5];
    let head = &mut a[..2];
    head[0] = 10;
    head[1] = 20;
    println!("{a:?}");
}
```

`&[T]` 是胖指针（fat pointer）：一个指向首元素的地址加一个 `usize` 长度。64 位平台上占 16 字节。索引 `s[i]` 先在运行期比较 i 与 len，再按 `i * size_of::<T>()` 偏移。

```rust
fn main() {
    let a = [1u64, 2, 3];
    let s: &[u64] = &a[1..];
    println!("{} {}", s.len(), std::mem::size_of::<&[u64]>());
}
```

range 越界同样 panic，而不是静默截断：对长度为 5 的数组取 `&a[2..9]`，程序会以 `range end index 9 out of range for slice of length 5` 终止。

函数参数写 `&[T]` 的理由就在这里：它接受数组、`Vec`、以及任意子切片，调用方不需要知道长度，也不需要为了适配而克隆。`&mut [T]` 同理。第 7 章会展开 `split_at`、`chunks`、`windows` 这些在切片上切分的工具。

## 4.5 字符串：str、&str 与 String

字符串是本章唯一涉及堆的类型，也是初学者最容易混淆的地方。先把三种形态分开：

- `str`：UTF-8 字节序列本身，无固定大小（unsized），只能通过指针使用。
- `&str`：借用的字符串切片，胖指针（地址 + 字节长度）。
- `String`：拥有所有权的可增长缓冲，缓冲区在堆上，栈上是三个字段。

字面量 `"hello"` 的类型是 `&'static str`：字节被编译进二进制的只读段，程序整个运行期都有效。`'static` 是生命周期标注，第 17 章细讲；现在只需知道"它不会失效"。

```rust
fn main() {
    let literal: &'static str = "hello";
    let owned: String = literal.to_string();
    let also: String = String::from("world");
    let borrowed: &str = &owned;
    println!("{literal} {owned} {also} {borrowed}");
}
```

`to_string` 来自 `ToString` trait，任何实现 `Display` 的类型都有；`String::from` 来自 `From<&str>`。两者都分配一块新缓冲并复制字节。`&owned` 只是借出 `&str`，不复制。

转换关系可以记成一张小表：

| 方向 | 写法 | 代价 |
| --- | --- | --- |
| String 到 &str | &s 或 s.as_str() | 零成本，只借 |
| &str 到 String | to_string、String::from、to_owned | 分配并复制 |
| &str 到 &'static str | 只有字面量可以 | 无 |

反方向不成立：运行期从输入读到的 `String` 不可能变成 `&'static str`，因为它的生命期由变量决定。

### 拼接：+、format! 与 push_str

`+` 的底层是 `Add` trait。标准库为 `String` 实现了 `Add<&str>`，所以左操作数必须是 `String`，右操作数必须是 `&str`，返回新的 `String`。左操作数会被移动：

```rust
fn main() {
    let hello = String::from("Hello, ");
    let world = String::from("world!");
    let s = hello + &world;
    println!("{s}");
    // println!("{hello}");   // 取消注释：borrow of moved value
}
```

`&world` 是 `&String`，靠 deref coercion 变成 `&str`。`+` 会复用左值的堆缓冲，不够时才扩容，因此链式写法 `a + &b + &c` 的分配次数比"每一步都新建"少，但左值只能被消费一次。

`format!` 更常用：它接受任意实现 `Display` 的参数，返回 `String`，不移动任何东西。

```rust
fn main() {
    let name = "Rust";
    let s = format!("{name} 有 {} 个字符", name.chars().count());
    println!("{s}");
}
```

`push_str` 与 `push` 原地追加，返回 `()`，不产生新 `String`（可能扩容）：

```rust
fn main() {
    let mut s = String::from("foo");
    s.push_str("bar");
    s.push('!');
    println!("{s}");
}
```

### 长度是字节，不是字符

`String` 的内容是 UTF-8 字节。`len()` 返回字节数，`chars().count()` 统计 Unicode 标量值：

```rust
fn main() {
    let s = String::from("你好, Rust");
    println!("字节 {}，字符 {}", s.len(), s.chars().count());
}
```

一个汉字在 UTF-8 里占 3 字节，所以上面是 12 字节、8 个字符。

### 为什么不能写 s[0]

`String` 没有实现 `Index<usize>`。UTF-8 是变长编码，第 0 个"字符"可能占 1 到 4 字节：返回 `u8` 会误导，返回 `char` 又需要从头扫描。Rust 选择不提供这个运算符，而不是提供一个语义含糊的版本。

要字节用 `s.as_bytes()[0]`，要字符用 `s.chars().next()`，要子串用范围切片 `&s[0..4]`。范围切片按字节偏移，必须落在字符边界上，否则运行期 panic。

```rust
fn main() {
    let s = String::from("hello");
    let first = &s[0..1];
    let bytes = s.as_bytes();
    let ch = s.chars().next().unwrap();
    println!("{first} {} {ch}", bytes[0]);
}
```

`&s[0..1]` 对纯 ASCII 安全；对 `"你好"` 写 `&s[0..1]` 会 panic，因为 1 不是字符边界。第 7 章会给出按字符安全切分的做法。

## 4.6 深入：三种类型的字节布局

### 数组：连续、无间隙

```text
[i32; 4] 在栈上的布局（地址从左到右递增）

+--------+--------+--------+--------+
|   0    |   1    |   2    |   3    |
+--------+--------+--------+--------+
  a[0]     a[1]     a[2]     a[3]
```

数组的元素按声明顺序紧挨着排，中间不插入任何东西。因此 `size_of::<[T; N]>() == N * size_of::<T>()`，对齐要求等于 T 的对齐要求。

```rust
use std::mem::size_of;

fn main() {
    println!("[u8; 1024] = {}", size_of::<[u8; 1024]>());
    println!("[u32; 4]   = {}", size_of::<[u32; 4]>());
}
```

这也解释了按值传数组的代价：`fn f(a: [u8; 1024])` 每次调用都复制 1 KiB。大数组一律传 `&[u8]` 或 `&[u8; 1024]`。

### 元组：可能有填充

Rust 的默认布局（`repr(Rust)`）允许编译器重排字段以减少填充。`(u8, u32, u8)` 在当前 rustc 上占 8 字节：u32 放最前，两个 u8 紧随其后，尾部补 2 字节。若严格按声明顺序排，需要 12 字节。

字段顺序不属于语言保证，所以不要把元组的内存布局直接交给 C。需要稳定布局时，标准做法是定义一个 `#[repr(C)]` 的结构体，第 35 章会讲。

另一个与元组有关的硬限制：标准库为元组实现的 `Debug`、`Clone`、`PartialEq` 等 trait 只覆盖到 12 个元素。更多元素不会自动获得这些能力——这也是"超过几个字段就该用结构体"的实际理由。

### [T; N] 与 Vec 的取舍

| 维度 | [T; N] | Vec |
| --- | --- | --- |
| 长度 | 类型的一部分 | 运行时 len 字段 |
| 存储 | 内联在值里，通常在栈上 | 三字段随 `Vec` 值存放，分配的缓冲区在堆上 |
| 大小 | N 乘以 `size_of::<T>()` | 三个机器字 |
| 扩容 | 不能 | push 摊还 O(1) |
| 适用 | 长度固定的表、缓冲区 | 长度会变的序列 |

数组不是"更快的 Vec"：它的价值在于长度已知，编译器能做边界消除、能把它放进别的类型里而不多一次指针跳转。代价是长度进了类型，跨函数传递时要么固定长度，要么退化成切片。

两者互转很容易：

```rust
fn main() {
    let a = [1, 2, 3];
    let v = Vec::from(a);
    let back: [i32; 3] = v.clone().try_into().unwrap();
    println!("{} {back:?}", v.len());
}
```

`Vec::from` 接管数组的元素（这里 i32 是 Copy，相当于复制）；`v.try_into()` 在长度不符时返回 `Err`，所以必须处理失败分支，不能直接赋值。

### String 的三个字段

```text
String（64 位平台上 24 字节）

+---------+---------+---------+
|   ptr   |   cap   |   len   |
+---------+---------+---------+
     |
     v
   +---+---+---+---+---+
   | h | e | l | l | o |   堆上缓冲，容量 cap 字节
   +---+---+---+---+---+
```

`String` 本质是 `Vec<u8>` 的包装：ptr 指向堆缓冲，cap 是已分配容量，len 是当前字节数。32 位平台上三个字段各 4 字节，共 12 字节。`&str` 只有两个字：地址与字节长度；`str` 本身无固定大小，就是"被指向的那段字节"。

```rust
use std::mem::size_of;

fn main() {
    let n = size_of::<usize>();
    println!("usize  = {n}");
    println!("&str   = {}", size_of::<&str>());
    println!("String = {}", size_of::<String>());
}
```

移动一个 `String` 只复制这三个字，堆缓冲原地不动；`clone()` 才会深拷贝。所以"传 String 还是传 &str"的差别不只是风格：前者把三字和释放责任一起交出去，后者只借两字。

## 典型错误案例

### 案例 1：数组长度与类型标注不符

```rust,editable
fn main() {
    let a: [i32; 3] = [1, 2];
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:2:23
  |
2 |     let a: [i32; 3] = [1, 2];
  |            --------   ^^^^^^ expected an array with a size of 3, found one with a size of 2
  |
help: consider specifying the actual array length
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：N 是类型的一部分，`[i32; 2]` 与 `[i32; 3]` 是两个不同的类型。赋值处检查的是类型相等，所以报的是"类型不匹配"，而不是"长度检查失败"。
**修复**：把标注改成 `[i32; 2]`，或把元素补到三个。函数返回 `[i32; 3]` 却 `return [0; 2]` 时报的是同一个 E0308。

</details>

### 案例 2：常量下标越界，编译期就被拦下

```rust,editable
fn main() {
    let a = [1, 2, 3];
    println!("{}", a[5]);
}
```

```text
error: this operation will panic at runtime
 --> src/main.rs:3:20
  |
3 |     println!("{}", a[5]);
  |                    ^^^^ index out of bounds: the length is 3 but the index is 5
  |
  = note: `#[deny(unconditional_panic)]` on by default
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：下标是字面量，数组长度也已知，常量传播（const propagation）算出这次索引必然越界，触发默认 deny 的 `unconditional_panic` lint。注意它没有 E 编号，是 lint 而不是类型错误。下标来自运行期数据时，同一处代码会编译通过，改为运行期 panic。
**修复**：换成合法下标；下标是否越界取决于输入时，用 `a.get(i)` 返回 `Option` 并在 `None` 分支里处理。

</details>

### 案例 3：用方括号索引元组

```rust,editable
fn main() {
    let t: (i32, i32) = (1, 2);
    println!("{}", t[0]);
}
```

```text
error[E0608]: cannot index into a value of type `(i32, i32)`
 --> src/main.rs:3:21
  |
3 |     println!("{}", t[0]);
  |                     ^^^ help: to access tuple element `0`, use: `.0`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：元组每个位置的类型可以不同，无法用运行期下标计算偏移，所以索引只接受 `.0` 这种字面量写法。数组的 `[]` 依赖 `Index` trait，元组没有实现它。
**修复**：写 `t.0`、`t.1`；确实要按下标循环，先把同类型元素转成数组或 `Vec`。

</details>

### 案例 4：两个字面量直接用 + 拼接

```rust,editable
fn main() {
    let s = "hello " + "world";
    println!("{s}");
}
```

```text
error[E0369]: cannot add `&str` to `&str`
 --> src/main.rs:2:22
  |
2 |     let s = "hello " + "world";
  |             -------- ^ ------- &str
  |
  = note: string concatenation requires an owned `String` on the left
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`+` 是 `Add` trait 的语法糖，标准库只为 `String` 实现了 `Add<&str>`，没有 `&str + &str`。拼接必须产生一块新缓冲，而 `&str` 不拥有任何缓冲，没人能承担这块内存。
**修复**：左边先变成 `String`，如 `"hello ".to_owned() + "world"` 或 `String::from("hello ") + "world"`；或者直接用 `format!("{a}{b}")`。

</details>

### 案例 5：String 传给 &str 参数，忘了 &

```rust,editable
fn greet(name: &str) {
    println!("hello, {name}");
}

fn main() {
    let s = String::from("world");
    greet(s);
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:7:11
  |
7 |     greet(s);
  |     ----- ^ expected `&str`, found `String`
  |
help: consider borrowing here
  |
7 |     greet(&s);
  |           +
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`String` 与 `&str` 是不同的类型。deref coercion 只在已经有引用时生效：`&String` 能自动变成 `&str`，但 `String` 本身不会自动被借用——Rust 不做隐式借用，否则所有权转移会变得难以推断。
**修复**：写 `greet(&s)`。参数收 `&str` 是对的：字面量、`&String`、`String` 的引用都能传，调用方不必克隆。

</details>

## 动手练习

1. 写 `fn divmod(a: i32, b: i32) -> (i32, i32)` 返回商与余数，用解构接收；再写一个版本返回 `Option<(i32, i32)>`，b 为 0 时返回 `None`。
2. 用 `[0u8; 16]` 表示一张 4×4 棋盘，把两条对角线置 1，再逐行打印。
3. 写 `fn average(xs: &[f64]) -> f64`，分别用数组、`Vec`、数组的一部分调用它，确认三种实参都能编译。
4. 对 `String::from("你好, Rust")` 分别打印字节数、字符数、第一个字符，以及 `&s[..6]` 得到的子串；解释 `&s[..1]` 为什么 panic。
5. 把 `["2024", "07", "01"]` 拼成 `2024-07-01`，分别用 `+`、`format!`、`push_str` 实现，比较哪一版产生的临时 `String` 最少。

## 小结

- 元组按位置组合不同类型，用 `t.0` 访问、用 `let (a, b) = t` 解构；单元类型 `()` 表示"没有值"。
- 数组 `[T; N]` 的长度是类型的一部分，元素连续内联存放；`[expr; N]` 要求 `Copy` 或常量，否则用 `array::from_fn`。
- 索引越界在运行期 panic；下标为常量时 `unconditional_panic` 在编译期就拒绝，需要容错时改用 `get`。
- 切片 `&[T]` 是"地址 + 长度"的胖指针，`&arr[a..b]` 不复制数据；函数参数优先收 `&[T]`。
- `str` 是 UTF-8 字节序列，`&str` 借用它，`String` 拥有可增长的堆缓冲；字面量是 `&'static str`。
- `+` 只接受 `String + &str` 并消费左值；`format!` 与 `push_str` 是更常用的拼接方式。
- 字符串不能用整数下标，因为 UTF-8 变长；`len()` 是字节数，`chars().count()` 是 Unicode 标量值的个数。
- 布局：数组无间隙，元组可能有填充，`String` 占三个机器字，`&str` 占两个。

