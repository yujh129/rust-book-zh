# 第 9 章 枚举与模式匹配

> **本章目标**
>
> - 会用枚举表达"几种情况之一"，并让不同变体携带不同形状的数据
> - 理解 `Option<T>` 如何把"可能没有"变成类型的一部分，从而消灭 null
> - 写出带绑定、多模式、范围、守卫和 `@` 的 `match`，并说清穷尽性检查发生在哪一步
> - 在 `match`、`if let`、`let ... else`、`while let` 之间按场景做出选择
> - 能读懂枚举的内存布局，以及 `match` 被编译成跳转表还是比较链

一个 TCP 连接处于什么状态？初学者通常会写几个布尔字段：`connected`、`authenticated`、`failed`，再加一个 `error: Option<String>`。这几个字段能组合出十几种状态，其中大多数没有意义：`connected == false` 却 `authenticated == true`，`failed == true` 却没有错误信息。编译器对这些组合一无所知，你只能在每个函数入口重复检查，而且每新增一个状态，都要回头审一遍所有 `if`。

另一类问题是"没有值"。Java 用 null，Go 用 nil，C 用空指针，它们都表示"这里可能没有东西"，但类型系统不区分"一定有"和"可能有"。于是"字符串"和"可能为空的字符串"是同一个类型，忘记判空就是运行期的 NullPointerException 或段错误。

Rust 用同一个工具解决这两个问题：枚举（enum）。一个枚举值**恰好是**若干变体中的一个，每个变体可以携带不同类型的数据。`Option<T>` 本身就是一个枚举，它把"没有"从运行期意外变成必须处理的类型分支；`match` 则是配套的解构工具，强制你覆盖所有变体。

## 9.1 从"几个布尔标志"到枚举

先看不用枚举的写法：一个 `Connection` 结构体，三个 `bool` 字段 `connected`、`authenticated`、`failed`，再加一个 `error: Option<String>`。

这样的结构体编译得过，但字段之间的约束只存在于你的脑子里：`authenticated` 只在 `connected` 为真时才有意义，`error` 只在 `failed` 为真时才有值。任何一处赋值都可能破坏它，而编译器帮不上忙。

枚举把"几种情况之一"直接写进类型：

```rust
#[derive(Debug)]
enum Message {
    Quit,
    Move { x: i32, y: i32 },
    Write(String),
    ChangeColor(u8, u8, u8),
}

fn main() {
    let a = Message::Quit;
    let b = Message::Move { x: 10, y: 20 };
    let c = Message::Write(String::from("你好"));
    let d = Message::ChangeColor(255, 0, 0);
    println!("{:?} {:?} {:?} {:?}", a, b, c, d);
}
```

`enum` 后面是类型名，花括号里是变体（variant），用逗号分隔。变体有三种写法，对应第 8 章讲过的三种结构体：

- `Quit` 只有名字，不携带数据，叫单元变体（unit variant）。
- `Move { x, y }` 携带具名字段，形状像结构体，叫结构体变体（struct variant）。
- `Write(String)` 与 `ChangeColor(u8, u8, u8)` 携带位置字段，形状像元组结构体，叫元组变体（tuple variant）。

关键点在于：**同一个枚举的不同变体可以携带完全不同的数据**。`Message` 的每个值在内存里大小相同（由最大的变体决定），但类型系统始终知道当前是哪个变体、里面装的是什么。

这一点和 C 的 `enum` 完全不同。C 的枚举只是一组整型常量，想附带数据就得自己配一个 `union` 和一个手写的 tag 字段，而编译器不会检查 tag 与 union 成员是否对得上。Rust 的枚举把 tag 和 payload 合成一个类型，读写都绕不过检查。

和 Java 相比，Java 要用抽象类 `Message` 加四个子类来表达同样的东西，每个值都是堆上的对象，带一个指向虚函数表的指针。Rust 的枚举是**闭合**的：变体在定义处就列全了，不可能在别的模块里偷偷加一个。闭合带来两个直接好处——值可以放在栈上，分派可以在编译期算出来。9.6 节会展开这两点。

## 9.2 给枚举写方法

枚举也能有 `impl` 块，写法和结构体一样。沿用 9.1 的 `Message`：

```rust,noplayground
impl Message {
    fn describe(&self) -> String {
        match self {
            Message::Quit => String::from("退出"),
            Message::Move { x, y } => format!("移动到 ({}, {})", x, y),
            Message::Write(text) => format!("写入 {} 个字符", text.chars().count()),
            Message::ChangeColor(r, g, b) => format!("颜色 ({}, {}, {})", r, g, b),
        }
    }
}

fn main() {
    let m = Message::Write(String::from("你好"));
    println!("{}", m.describe());
    println!("{}", Message::Move { x: 1, y: 2 }.describe());
}
```

`match self` 里的 `self` 是 `&Message`，但模式写的是 `Message::Move { x, y }`，没有 `&` 也没有 `ref`。这是**默认绑定模式**（default binding modes，社区也叫 match ergonomics）：当被匹配的值是引用时，编译器自动把绑定变成引用，于是 `x` 的类型是 `&i32`、`text` 的类型是 `&String`。所以 `text.chars()` 能直接调用，不需要先解引用。

只判断变体、不取数据时，用标准库的 `matches!` 宏更省事：`matches!(m, Message::Quit)` 返回 `bool`，等价于只写一个 arm 的 `match`。它定义在 `std::matches`，展开后就是 `match` 表达式。

## 9.3 `Option<T>`：把 null 从类型里删掉

标准库对 `Option` 的定义只有两行：

```rust,noplayground
enum Option<T> {
    Some(T),
    None,
}
```

它就是一个普通枚举，没有任何编译器魔法。`Some` 和 `None` 被放进了 prelude，所以你能直接写名字，不用 `Option::Some`。`Option<T>` 对任意 `T` 都成立，这里的 `<T>` 是第 14 章要讲的泛型，现在只需知道"T 可以是任何类型"。

关键在于 `Option<i32>` 和 `i32` 是**两个不同的类型**：

```rust
fn half(n: i32) -> Option<i32> {
    if n % 2 == 0 {
        Some(n / 2)
    } else {
        None
    }
}

fn main() {
    let a = half(10);
    let b = half(7);
    println!("{:?} {:?}", a, b);

    // let x: i32 = a;   // 编译错误：expected `i32`, found `Option<i32>`
    let x = a.unwrap_or(0);
    println!("{}", x);

    let doubled = b.map(|n| n * 2).unwrap_or(-1);
    println!("{}", doubled);
}
```

输出是 `Some(5) None`、`5`、`-1`。真正重要的是那行注释：把 `Option<i32>` 直接赋给 `i32` 是类型错误，编译器不给你"偷偷当成 0"的机会。

对比一下另外三种语言：

- Java 的 `Integer` 可以是 null，`int` 不能，但两者在源码里长得几乎一样，拆箱时才可能抛 NullPointerException。
- Go 用 `nil` 表示"没有"，可空指针、nil 接口、map 里不存在的键都用它，而"接口里装着一个 nil 指针"至今仍是常见 bug。
- C 用空指针，解引用空指针是未定义行为。

`Option<T>` 的区别不是语法更漂亮，而是**"可能没有"进入了类型**：接收方要么用 `match` 处理两个分支，要么用 `unwrap_or` 一类的方法显式给出默认值，否则连编译都过不去。忘记判空这件事，在 Rust 里不存在。

几个常用方法，都定义在 `Option` 上：

- `map(f)`：有值时把值交给 `f` 变换，`None` 原样传下去。
- `and_then(f)`：`f` 也返回 `Option`，用来串联多步可能失败的操作，避免 `Option<Option<T>>`。
- `unwrap_or(v)`、`unwrap_or_else(f)`、`unwrap_or_default()`：取出值或给默认值。
- `ok_or(err)`：转成 `Result`，第 10 章会讲什么时候该转。
- `as_ref()`：借用里面的值，不消耗原来的 `Option`。
- `?` 运算符：在返回 `Option` 的函数里遇到 `None` 直接提前返回。

把字符串解析成数字，是 `Option` 最常见的出场场景：

```rust
fn main() {
    let nums: Vec<i32> = vec![10, 20, 30];
    println!("first = {:?}", nums.first());

    let ok: Option<i32> = "42".parse().ok();
    let bad: Option<i32> = "abc".parse().ok();
    println!("ok = {:?}, bad = {:?}", ok, bad);

    match bad {
        Some(n) => println!("解析出 {}", n),
        None => println!("不是数字"),
    }

    let total: i32 = nums.iter().copied().filter_map(|n| n.checked_mul(1000)).sum();
    println!("total = {}", total);
}
```

`nums.first()` 返回 `Option<&i32>`：空切片就没有第一个元素。`parse()` 返回 `Result`，`.ok()` 把错误丢掉只留成功值，所以得到 `Option<i32>`。最后一行用 `filter_map` 把 `Option` 流里的 `None` 全部过滤掉，只累加 `Some`——这是迭代器章节的常用手法，但它的前提就是 `Option` 是一个能被遍历、能参与组合的类型。

顺便区分两个容易混的类型：`Option<Option<T>>` 是有意义的，它有三个层次——"没有这个键"、"键存在但值为空"、"有值"。数据库查询和 JSON 处理里经常需要这三态，用一个 `Option` 表达不了。

## 9.4 match：穷尽、绑定与守卫

`match` 的形式是 `match 表达式 { 模式 => 结果, ... }`。它是表达式，整个 `match` 的值就是被选中那个 arm 的值，因此可以直接赋给变量或作为函数返回值。所有 arm 的结果必须是同一个类型。

### 穷尽性检查发生在编译期

9.2 的 `describe` 之所以能编译，是因为四个变体一个不少。少写一个，编译器直接拒绝（典型错误案例 1 会给出完整报错）。

这和 C、Java 的 `switch` 有本质区别：`switch` 的 `default` 是可选的，漏掉分支只会在运行时悄悄走进 `default`；Rust 没有隐式的 `default`，`_` 是你**主动写**的通配模式，写了就等于声明"剩下的情况我都不关心"。

穷尽性检查还有一个副产品：给枚举新增变体时，所有没写 `_` 的 `match` 都会编译失败，编译器把位置一一列出来。这是特性而不是麻烦——扩展类型时，它逼你走遍每一处需要更新的代码。

### 绑定与嵌套解构

模式不只是"比较"，它还能把变体里的数据取出来绑到名字上，并且可以一层层往下拆：

```rust
enum Shape {
    Circle { center: (f64, f64), radius: f64 },
    Rect { top_left: (f64, f64), size: (f64, f64) },
}

fn area(s: &Shape) -> f64 {
    match s {
        Shape::Circle { radius, .. } => 3.14159 * radius * radius,
        Shape::Rect { size: (w, h), .. } => w * h,
    }
}

fn main() {
    let c = Shape::Circle { center: (0.0, 0.0), radius: 1.0 };
    println!("{:.2}", area(&c));
}
```

`Shape::Rect { size: (w, h), .. }` 在同一个模式里做了三件事：确认变体是 `Rect`、取出 `size` 字段、把元组拆成 `w` 和 `h`。`..` 表示"其余字段忽略"，当结构体变体字段很多时能省掉一长串名字。

### 多模式、范围与守卫

同一个 arm 可以用 `|` 并列多个模式，用 `a..=b` 匹配闭区间，用 `if` 追加额外条件：

```rust
fn main() {
    for n in [-3, 0, 1, 5, 6, 100] {
        let label = match n {
            x if x < 0 => "负",
            0 => "零",
            1..=5 => "小",
            _ => "大",
        };
        println!("{} -> {}", n, label);
    }
}
```

输出是 `-3 -> 负`、`0 -> 零`、`1 -> 小`、`5 -> 小`、`6 -> 大`、`100 -> 大`。字符也能用范围，`'a'..='z'` 与 `'A'..='Z'` 常被写成同一个 arm：`'a'..='z' | 'A'..='Z' => ...`。

arm 按从上到下的顺序尝试，第一个匹配成功的胜出。所以 `x if x < 0` 必须写在 `0` 前面吗？不必，因为 `0` 这个字面量模式不会匹配负数。但 `_` 一旦写在前面，后面的 arm 就永远轮不到，编译器会用 `unreachable pattern` 警告提醒你。

守卫（guard）是模式后面那个 `if`。它比模式表达力更强，因为可以调用任意函数、比较多个变量。代价是：**带守卫的 arm 不算"覆盖"某个变体**，穷尽性检查会忽略它。上面第一个 `match` 如果删掉最后的 `_`，即使 `x if x < 0` 看起来覆盖了负数，编译器仍然报 E0004——守卫可能为假，编译器不替你推理。

`@` 用来"一边测试一边绑定"。范围模式 `1..=9` 本身不产生值，想在 arm 里用这个数字，就得写 `x @ 1..=9`：

```rust
fn main() {
    for n in [7, 42] {
        match n {
            x @ 1..=9 => println!("一位数 {}", x),
            x => println!("其他 {}", x),
        }
    }
}
```

顺便记住 `match` 的值可以是 `()`。只为了执行副作用时，每个 arm 都写 `println!` 就行，不用给整个 `match` 赋值。

## 9.5 if let、let else、while let

`match` 强制穷尽，但很多场景你只关心一个变体。为了取一个 `Some` 里的值写五个 arm，噪音大于信息。

### if let：只处理一种情况

```rust
fn main() {
    let config: Option<&str> = Some("127.0.0.1");

    if let Some(host) = config {
        println!("host = {}", host);
    } else {
        println!("没有配置 host");
    }
}
```

`if let` 只写一个模式和一个分支，其余情况走 `else`。它**不做穷尽性检查**：`else` 可以省略，没匹配上就什么也不做。这是它和 `match` 最重要的区别，也是它容易埋 bug 的地方——省略 `else` 时，请确认"其余情况什么都不做"真的是你想要的。

多个条件可以串成 `if let ... else if let ... else`，但超过两层就该换回 `match`，因为读者要在脑子里补全剩下的分支。

### let else：成功路径不缩进

函数开头常常是"拿到值就继续，拿不到就退出"。用 `match` 写要先接住结果，再在错误分支里 `return`，同一个名字出现两次；`let ... else` 把"否则"提到绑定的同一行，正常路径因此不用缩进：

```rust
fn parse_port(s: &str) -> u16 {
    let Ok(port) = s.parse::<u16>() else {
        return 8080;
    };
    port
}

fn main() {
    println!("{} {}", parse_port("3000"), parse_port("abc"));
}
```

`else` 块**必须发散**（diverge）：里面要 `return`、`break`、`continue` 或 `panic!`。原因是这个块没有值可以赋给 `port`，控制流必须离开当前语句。`let else` 是 Rust 1.65 稳定的，写命令行工具和解析器时用得非常频繁。

### while let：取到没有为止

```rust
fn main() {
    let mut stack = vec![1, 2, 3];
    while let Some(top) = stack.pop() {
        println!("弹出 {}", top);
    }
    println!("栈空了吗：{}", stack.is_empty());
}
```

`while let` 在模式匹配成功时继续循环，失败时退出。`Vec::pop` 返回 `Option<T>`，正好适合这种"取到空为止"的循环。`Iterator::next`、`BufReader::lines`、通道的 `recv` 都是同一形状。

选择哪一种，看四个问题：

- 需要覆盖所有变体，或者要用 `match` 的返回值：用 `match`。
- 只关心一个变体，其余忽略：用 `if let`，需要兜底就加 `else`。
- 函数开头"不满足就退出"：用 `let ... else`，让正常路径保持零缩进。
- 循环里反复取，直到取不到：用 `while let`。

## 9.6 深入：枚举在内存里长什么样

### tag 加 payload

枚举的值由两部分组成：一个记录"当前是哪个变体"的标签（tag，也叫判别值 discriminant），加上该变体自己的数据（payload）。整块的大小等于"tag 加上最大变体的 payload"，再按对齐规则补齐。

```rust
use std::mem::size_of;
use std::num::NonZeroU32;

enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Empty,
}

fn main() {
    println!("Shape              = {}", size_of::<Shape>());
    println!("Option<Shape>      = {}", size_of::<Option<Shape>>());
    println!("&u8                = {}", size_of::<&u8>());
    println!("Option<&u8>        = {}", size_of::<Option<&u8>>());
    println!("Option<Box<u8>>    = {}", size_of::<Option<Box<u8>>>());
    println!("Option<u32>        = {}", size_of::<Option<u32>>());
    println!("Option<NonZeroU32> = {}", size_of::<Option<NonZeroU32>>());
}
```

在 rustc 1.100.0、x86_64 上输出是 24、24、8、8、8、8、4。

`Shape` 是 24 字节：`Rect` 需要两个 `f64` 共 16 字节，加上 8 字节 tag，正好 24。`Circle` 只用到 8 字节 payload，剩下的 8 字节是浪费——枚举按最大的变体分配，所有变体一样大，这样数组里每个元素才能用固定步长索引。

`Option<Shape>` 也是 24 字节，没有变大。原因是 `Shape` 的 tag 只用了 0、1、2 三个值，还有 253 个位模式是"非法"的，编译器拿其中一个表示 `None`。

`Option<u32>` 是 8 字节：`u32` 的 2^32 个位模式全都合法，没有空位可用，只能额外加 4 字节 tag。换成 `NonZeroU32` 就只要 4 字节，因为 0 被类型排除在外，正好当 `None`。

`Option<&u8>`、`Option<Box<u8>>` 都是 8 字节：引用和 `Box` 不可能为空指针，于是 0 这个位模式被借走表示 `None`。这个技巧叫 **niche 优化**，标准库把它用在引用、`Box`、`Vec`、`String`、`NonZero*` 和 `char`（`Option<char>` 是 4 字节）上。它是 `Option` 能零成本替代 null 的真正原因。

必须强调：这些数字是**当前实现的观测值，不是语言保证**。枚举默认布局是 `repr(Rust)`，字段顺序、tag 大小都可能随版本或平台变化。要和 C 交互、要按固定格式序列化，就加 `#[repr(C)]`（布局跟 C 的 tagged union 一致）或 `#[repr(u8)]`（tag 固定一字节，变体可以当整数用）。

### match 被编译成什么

`match` 没有运行时支持，它全部在编译期被翻译成普通控制流。常见的有三种形态。

第一种是**跳转表**。下面这段 `match` 的取值密集且不规则：

```rust,noplayground
#[inline(never)]
pub fn classify(x: u8) -> u8 {
    match x {
        0 => 3,
        1 => 1,
        2 => 4,
        3 => 1,
        4 => 5,
        5 => 9,
        6 => 2,
        7 => 6,
        8 => 5,
        9 => 3,
        _ => 0,
    }
}
```

用 `rustc -O --emit asm` 看它的核心指令：

```text
cmpb    $9, %cl
ja      .LBB0_1
movzbl  %cl, %eax
leaq    switch.table.classify(%rip), %rcx
movzbl  (%rax,%rcx), %eax
retq
.LBB0_1:
xorl    %eax, %eax
retq

switch.table.classify:
    .ascii "\003\001\004\001\005\t\002\006\005\003"
```

`cmpb $9` 做边界检查，越界跳到最后返回 0；否则把 `x` 当索引查表，一次内存读取拿到结果。表里那十个字节，正好是每个 arm 的返回值。整个过程是 O(1)，只有一次比较和一次查表。

第二种是**比较链**：取值稀疏时（比如 `0 => "a", 1000 => "b", 100000 => "c"`），编译器生成一串 `cmp` 加条件跳转，复杂度 O(n)。这时把最常见的分支放前面有一点点意义，但别为这点差异重构代码。

第三种是**算术化**。把上面的表换成正则的 `0 => 10, 1 => 20, ..., 5 => 60, _ => 0`，LLVM 会发现结果就是 `10 * (x + 1)`，整个 `match` 变成三条 `lea`/`add` 加一个 `cmov`，连跳转都没有。

这三种形态说明同一件事：`match` 是零成本抽象。它不引入运行时机制，只是把"意图"交给编译器，怎么实现由后端按代价模型决定。你在源码里写的 arm 顺序，除了影响"第一个匹配胜出"的语义，不决定机器码的形状。

### 为什么不用虚函数表也能多态

Java 的抽象类方案里，每个 `Message` 对象带一个指向虚函数表的指针，调用 `describe` 是"取指针、查表、间接调用"，对象必须在堆上。

Rust 的枚举方案里，变体编号就是值的一部分，`match` 比较 tag 后直接跳转，没有额外指针，值可以整个放在栈上，也可以连续排进 `Vec`。代价是枚举必须**闭合**：变体在定义处列全，不能由外部扩展。

如果变体集合需要开放（插件系统、第三方扩展的 AST 节点），就得换成 trait 对象：写一个 `trait Node { fn eval(&self) -> i64; }`，再给每种节点各写一个结构体去实现它，节点之间用 `Box<dyn Node>` 组合。这时每个节点是独立类型、各自在堆上分配，调用通过 vtable 间接进行。

判据很清楚：**变体集合固定、需要紧凑存储和高频分派，用枚举；变体集合开放、由外部实现，用 trait 对象。**第 16 章会量化 `dyn` 的代价。

## 9.7 综合：用枚举写一个表达式求值器

把本章的东西合起来，写一个能跑的表达式求值器。表达式是一棵树，节点有四种：数字、加法、乘法、取负。每种节点携带的数据不同，正好用枚举：

```rust
#[derive(Debug)]
enum Expr {
    Num(i64),
    Add(Box<Expr>, Box<Expr>),
    Mul(Box<Expr>, Box<Expr>),
    Neg(Box<Expr>),
}

fn eval(e: &Expr) -> i64 {
    match e {
        Expr::Num(n) => *n,
        Expr::Add(a, b) => eval(a) + eval(b),
        Expr::Mul(a, b) => eval(a) * eval(b),
        Expr::Neg(a) => -eval(a),
    }
}

fn show(e: &Expr) -> String {
    match e {
        Expr::Num(n) => n.to_string(),
        Expr::Add(a, b) => format!("({} + {})", show(a), show(b)),
        Expr::Mul(a, b) => format!("({} * {})", show(a), show(b)),
        Expr::Neg(a) => format!("-{}", show(a)),
    }
}

fn main() {
    // (1 + 2) * (3 + -4)
    let one = Box::new(Expr::Num(1));
    let two = Box::new(Expr::Num(2));
    let three = Box::new(Expr::Num(3));
    let four = Box::new(Expr::Neg(Box::new(Expr::Num(4))));
    let e = Expr::Mul(
        Box::new(Expr::Add(one, two)),
        Box::new(Expr::Add(three, four)),
    );
    println!("{} = {}", show(&e), eval(&e));
}
```

运行输出一行：`((1 + 2) * (3 + -4)) = -3`。

几个细节值得停下来看：

**为什么子节点要装进 `Box`。** `Expr` 的大小必须在编译期确定，而 `Add` 里又装着 `Expr`，直接写 `Add(Expr, Expr)` 会让类型无限展开，编译器报"recursive type has infinite size"。`Box<Expr>` 是一个固定大小的指针，把子节点放到堆上，递归就断开了。第 18 章会讲 `Box` 的所有权语义。

**`eval` 和 `show` 是同一个形状。** 两者都是"对每种变体做一件事，遇到子节点就递归"。这种"在枚举上做遍历"的写法在编译器、解释器、JSON 处理里反复出现，模式固定。

**`*n` 里的星号。** `eval` 接收 `&Expr`，默认绑定模式下 `n` 的类型是 `&i64`，而 `eval` 返回 `i64`，所以要解引用。`a` 和 `b` 是 `&Box<Expr>`，传给 `eval` 时自动解引用成 `&Expr`，不用手写 `&**a`。

**加一个变体会发生什么。** 假设你加 `Div(Box<Expr>, Box<Expr>)`，然后重新编译：`eval` 和 `show` 都会报 E0004，指向缺失的 `Expr::Div` 分支。你不需要 grep 全项目找"哪里处理了 Expr"，编译器已经列出了完整清单。这就是闭合枚举配合穷尽性检查的价值。

**它能怎么变坏。** 两个地方没有防护：`i64` 的加法溢出在 debug 构建下 panic、在 release 下回绕；递归深度只受栈大小限制，构造一个一万层深的表达式就能把它打崩。生产代码里应该让 `eval` 返回 `Result<i64, EvalError>`，并在解析阶段限制嵌套深度——第 10 章会把返回类型改成 `Result`，第 40 章手写 JSON 解析器时会再遇到同样的深度问题。

## 典型错误案例

### 案例 1：漏掉一个变体——E0004

```rust,editable
enum Direction {
    North,
    South,
    East,
    West,
}

fn describe(d: Direction) -> &'static str {
    match d {
        Direction::North => "上",
        Direction::South => "下",
        Direction::East => "右",
    }
}

fn main() {
    println!("{}", describe(Direction::North));
}
```

```text
error[E0004]: non-exhaustive patterns: `Direction::West` not covered
  --> src/main.rs:9:11
   |
 9 |     match d {
   |           ^ pattern `Direction::West` not covered
   |
note: `Direction` defined here
  --> src/main.rs:1:6
   |
 1 | enum Direction {
   |      ^^^^^^^^^
...
 5 |     West,
   |     ---- not covered
   = note: the matched value is of type `Direction`
help: ensure that all possible cases are being handled by adding a match arm with a wildcard pattern or an explicit pattern as shown
   |
12 ~         Direction::East => "右",
13 ~         Direction::West => todo!(),
«««

**原因**：`match` 必须覆盖被匹配值的每一种可能。`Direction` 有四个变体，你只写了三个，编译器就把漏掉的那个指出来。这不是提醒，是错误——Rust 不允许"剩下的情况悄悄跳过"，所以 C 的 `switch` 忘记 `break`、Java 的 `default` 吞掉新枚举值这类事故在这里编译不过。help 里给出的 `todo!()` 是先用占位让代码编译通过的办法，运行到那一支会 panic，之后再补实现。

**修复**：补上 `Direction::West` 分支。如果你确实想让"其余情况"归为一类，显式写 `_ => ...`；代价是以后新增变体时编译器不再提醒你。

```rust
enum Direction {
    North,
    South,
    East,
    West,
}

fn describe(d: Direction) -> &'static str {
    match d {
        Direction::North => "上",
        Direction::South => "下",
        Direction::East => "右",
        Direction::West => "左",
    }
}

fn main() {
    println!("{}", describe(Direction::West));
}
```

### 案例 2：分支类型不一致——E0308

```rust,noplayground
fn main() {
    let n = 3;
    let msg = match n {
        1 => "one",
        2 => String::from("two"),
        _ => "many",
    };
    println!("{}", msg);
}
```

```text
error[E0308]: `match` arms have incompatible types
 --> src/main.rs:5:14
  |
3 |       let msg = match n {
  |  _______________-
4 | |         1 => "one",
  | |              ----- this is found to be of type `&str`
5 | |         2 => String::from("two"),
  | |              ^^^^^^^^^^^^^^^^^^^ expected `&str`, found `String`
6 | |         _ => "many",
7 | |     };
  | |_____- `match` arms have incompatible types
  |
help: consider borrowing here
  |
5 |         2 => &String::from("two"),
  |              +

```

**原因**：`match` 是一个表达式，它的类型由所有分支共同决定，每个分支都必须产出同一种类型。第一个分支给的是字符串字面量 `&str`，第二个给的是 `String`，两者不是一回事。编译器以先出现的分支为基准，在第二个分支上报"expected `&str`, found `String`"。

**修复**：让所有分支产出同一种类型。这里统一成 `String` 最直接；如果你需要的是借用的字符串，可以改成 `"one"`、`"two"` 全用字面量。

```rust
fn main() {
    let n = 3;
    let msg = match n {
        1 => String::from("one"),
        2 => String::from("two"),
        _ => String::from("many"),
    };
    println!("{}", msg);
}
```

### 案例 3：从实现了 Drop 的类型里移出字段——E0509

```rust,noplayground
struct Wrapper {
    value: String,
}

impl Drop for Wrapper {
    fn drop(&mut self) {
        println!("drop {}", self.value);
    }
}

fn main() {
    let w = Wrapper {
        value: String::from("data"),
    };
    match w {
        Wrapper { value } => println!("{}", value),
    }
}
```

```text
error[E0509]: cannot move out of type `Wrapper`, which implements the `Drop` trait
  --> src/main.rs:15:11
   |
15 |     match w {
   |           ^ cannot move out of here
16 |         Wrapper { value } => println!("{}", value),
   |                   ----- data moved here because `value` has type `String`, which does not implement the `Copy` trait
   |
help: consider borrowing the pattern binding
   |
16 |         Wrapper { ref value } => println!("{}", value),
   |                   +++

```

**原因**：`match w` 按值匹配，模式 `Wrapper { value }` 会把 `value` 这个字段从 `w` 里搬出来。但 `Wrapper` 实现了 `Drop`，它的 `drop` 方法在析构时要访问 `self.value`；字段被搬走，析构函数就会读到已经失效的数据。Rust 因此禁止对实现了 `Drop` 的类型做部分移出，报错里那句"cannot move out of type ... which implements the Drop trait"说的就是这件事。

**修复**：按引用匹配。`match &w` 之后，模式绑定的 `value` 是 `&String`，只借用不搬走，`Drop` 仍然能正常工作。

```rust
struct Wrapper {
    value: String,
}

impl Drop for Wrapper {
    fn drop(&mut self) {
        println!("drop {}", self.value);
    }
}

fn main() {
    let w = Wrapper {
        value: String::from("data"),
    };
    match &w {
        Wrapper { value } => println!("{}", value),
    }
}
```

### 案例 4：match 移动后又使用——E0382

```rust,noplayground
enum Msg {
    Text(String),
    Ping,
}

fn main() {
    let m = Msg::Text(String::from("hi"));
    match m {
        Msg::Text(s) => println!("收到 {}", s),
        Msg::Ping => println!("ping"),
    }
    match m {
        Msg::Text(s) => println!("再次收到 {}", s),
        Msg::Ping => println!("ping"),
    }
}
```

```text
error[E0382]: use of moved value
  --> src/main.rs:13:19
   |
 9 |         Msg::Text(s) => println!("收到 {}", s),
   |                   - value moved here
...
13 |         Msg::Text(s) => println!("再次收到 {}", s),
   |                   ^ value used here after move
   |
   = note: move occurs because value has type `String`, which does not implement the `Copy` trait
help: borrow this binding in the pattern to avoid moving the value
   |
 9 |         Msg::Text(ref s) => println!("收到 {}", s),
   |                   +++

```

**原因**：`m` 的类型是 `Msg`，里面装着 `String`，而 `String` 不是 `Copy`。第一次 `match m` 按值匹配，`Msg::Text(s)` 把整个字符串的所有权移进了 `s`；`m` 在那一刻已经被拆开，第二次再用它就是使用已移动的值。注意报错位置在第二次 `match`，真正的原因在第一次。

**修复**：两次都按引用匹配，绑定得到 `&String`，所有权留在 `m` 里。

```rust
enum Msg {
    Text(String),
    Ping,
}

fn main() {
    let m = Msg::Text(String::from("hi"));
    match &m {
        Msg::Text(s) => println!("收到 {}", s),
        Msg::Ping => println!("ping"),
    }
    match &m {
        Msg::Text(s) => println!("再次收到 {}", s),
        Msg::Ping => println!("ping"),
    }
}
```

### 案例 5：变体名拼错——E0599

```rust,noplayground
enum Expr {
    Num(i64),
    Add(Box<Expr>, Box<Expr>),
}

fn main() {
    let e = Expr::add(Box::new(Expr::Num(1)), Box::new(Expr::Num(2)));
    match e {
        Expr::Num(n) => println!("{}", n),
        Expr::Add(_, _) => println!("加法"),
    }
}
```

```text
error[E0599]: no variant, associated function, or constant named `add` found for enum `Expr` in the current scope
 --> src/main.rs:7:19
  |
1 | enum Expr {
  | --------- variant, associated function, or constant `add` not found for this enum
...
7 |     let e = Expr::add(Box::new(Expr::Num(1)), Box::new(Expr::Num(2)));
  |                   ^^^ variant, associated function, or constant not found in `Expr`
  |
  = help: items from traits can only be used if the trait is implemented and in scope
  = note: the following trait defines an item `add`, perhaps you need to implement it:
          candidate #1: `Add`
help: there is a variant with a similar name
  |
7 -     let e = Expr::add(Box::new(Expr::Num(1)), Box::new(Expr::Num(2)));
7 +     let e = Expr::Add(Box::new(Expr::Num(1)), Box::new(Expr::Num(2)));
  |

```

**原因**：变体是类型命名空间里的项，写法固定为 `Expr::Add`，首字母大写。写成小写的 `Expr::add`，编译器会先按关联函数查找，找不到再报 E0599；报错里那条 `candidate #1: Add` 是它在 trait 里找到了同名的 `Add`，属于误导线索，真正有用的是最后一行的相似名提示。这类错误在只改了大小写或少打一个字母时最常见。

**修复**：改成 `Expr::Add`。

```rust
enum Expr {
    Num(i64),
    Add(Box<Expr>, Box<Expr>),
}

fn main() {
    let e = Expr::Add(Box::new(Expr::Num(1)), Box::new(Expr::Num(2)));
    match e {
        Expr::Num(n) => println!("{}", n),
        Expr::Add(_, _) => println!("加法"),
    }
}
```

## 动手练习

1. 给 9.1 的 `Message` 加一个 `Ping(u32)` 变体，让 `describe` 返回"第 n 次心跳"。改完先别急着运行，猜猜不加 `match` 分支时编译器会报哪个错误码。
2. 写 `fn last_char(s: &str) -> Option<char>`，用 `s.chars().last()` 实现；在 `main` 里分别传 `""` 和 `"枚举"`，用 `match` 打印两种结果。
3. 写 `fn parse_port(s: &str) -> Option<u16>`：用 `let ... else` 接住 `s.parse::<u32>()` 的结果，解析失败直接返回 `None`；解析成功后再用范围判断 `(1..=65535).contains(&n)` 校验，超范围也返回 `None`。分别用 `"8080"`、`"70000"`、`"abc"` 测试。
4. （动手跑）给 9.7 的求值器加 `Div(Box<Expr>, Box<Expr>)`，把 `eval` 的返回类型改成 `Option<i64>`：除数为 0 时返回 `None`，`show` 里用 `/`。构造 `(10 / 2)` 与 `(10 / (3 - 3))` 并打印，后者应该输出 `None`。加完变体后编译器会依次指出 `eval` 和 `show` 里缺失的分支。
5. （动手跑）把 9.6 的 `classify` 单独存成文件，用 `rustc -O --emit asm` 编译，找到那张 `switch.table`；再把返回值改成 `0 => 10`、`1 => 20` 一直到 `5 => 60`、`_ => 0` 这样的规则序列，重新编译一次，看看跳转表为什么消失了。

## 小结

- 枚举的值恰好是若干变体之一，每个变体可以携带不同类型、不同数量的数据，类型系统始终知道当前是哪一支。
- 与 C 的 `enum` 加 `union` 相比，Rust 把 tag 和 payload 合成一个类型，读错变体编译不过。
- `Option<T>` 只是一个普通枚举，但它让"可能没有"进入类型；`Option<&T>`、`Option<Box<T>>` 靠 niche 优化做到零开销。
- `match` 必须穷尽；`_` 是你主动放弃提醒的写法，带守卫的 arm 不参与穷尽性判断。
- `|` 并列模式、`a..=b` 范围、`if` 守卫、`x @ 模式` 绑定，四件工具覆盖绝大多数分支逻辑。
- 只关心一个变体用 `if let`；函数开头"不满足就退出"用 `let ... else`（else 块必须发散）；循环取到空为止用 `while let`。
- 枚举在内存里是 tag 加最大变体的 payload；`match` 被编译成跳转表、比较链或算术式，没有运行时开销。
- 变体集合固定用枚举，集合开放用 trait 对象；前者能放在栈上连续存储，后者换来扩展性。遇到不认识的错误码，`rustc --explain E0xxx` 会给出一篇解释。
