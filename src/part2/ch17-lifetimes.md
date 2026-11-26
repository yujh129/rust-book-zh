# 第 17 章 生命周期：让引用有据可依

> **本章目标**
>
> - 能给函数签名加上 `<'a>`，说清 `&'a str` 里的 `'a` 约束的是哪一段区间
> - 掌握生命周期省略的三条规则，拿到一个签名就能判断要不要手写标注
> - 会写 `struct Tokenizer<'a> { input: &'a str }` 与 `impl<'a> Tokenizer<'a>`，用零拷贝的方式切分文本
> - 分清 `'static` 的两种用法：真正的全程序引用，与"不包含短借用"的约束
> - 能解释 NLL 为什么放过一部分代码，以及型变（variance）怎样让 `&'a mut T` 锁死调用点

C 语言里，函数返回局部变量的地址是合法代码：

```c
char *bad(void) {
    char buf[16];
    return buf;          /* 编译通过，运行到调用点才出错 */
}
```

这段代码能编译，是因为 C 的类型系统只记录"这是一个 `char *`"，不记录"它指向的那块栈内存还剩多久"。Rust 把这条信息塞进了类型：`&'a str` 里的 `'a` 就是"这块数据至少有效到什么时候"的记号，由借用检查器（borrow checker）在编译期核对。第 6 章讲过借用的基本规则，本章处理的是另一半问题：**当编译器自己推不出来时，你怎么把意图写进签名。**

需要标注的场景比想象中少。大多数函数只有一个引用参数，或者有 `&self`，编译器按固定规则就能补齐，你什么都不用写。真正要动手的是三类地方：多个引用参数而返回值只借自其中一个、结构体里存引用、以及跨函数传递"借来的东西"。把这三类写对，剩下的报错都能归到同一条规则上。

生命周期只存在于编译期。写成 `fn f<'a>(x: &'a str)` 和写成 `fn f(x: &str)`，编译出来的机器码完全一样，也不会因为生命周期不同而复制一份代码——这一点在 17.6 会从型变的角度再确认一次。

## 17.1 从两个参数的函数说起

只有一个引用参数时，编译器不需要你帮忙：

```rust
fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

fn main() {
    let report = String::from("第一行\n第二行");
    println!("{}", first_line(&report));
}
```

返回的引用只可能借自 `text`，没有第二种解释，所以省略标注也安全。参数变成两个，歧义立刻出现：

```rust
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() >= y.len() { x } else { y }
}

fn main() {
    let a = String::from("hello");
    let b = String::from("hi");
    println!("{}", longest(&a, &b));

    {
        let short = String::from("a longer temporary");
        let picked = longest(&a, &short);
        println!("{}", picked);
    }
}
```

三个 `'a` 是同一个名字，含义是"这三个位置共用一段有效区间"：

```text
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str
           ~~      ~~          ~~          ~~
            |        \          /          /
            |         \        /          /
            +---- 同一个区域名，四处指同一段区间
```

这段区间有多长，不由函数体决定，而由每个调用点决定。第一次调用 `longest(&a, &b)` 时，`'a` 必须覆盖到返回值最后一次被使用的地方，同时不能超过 `a` 和 `b` 中先失效的那个。第二次调用在花括号里，`short` 出块即释放，`picked` 也只在块内使用，于是 `'a` 收缩到块内，依然合法。把最后那行 `println!` 挪到块外面，就是案例 3 里的 E0597。

关于标注，先记住三句话：

1. `'a` 是**区域名**，不是"活得多久"。它给一段区间起名字，好让多个位置引用同一段区间。
2. 标注是**约束**，不是指令。它不会让任何值活得更久，只会让编译器在调用点检查"借得够不够久"。
3. 标注**不产生运行时代码**。生命周期在借用检查结束后就被擦除，类型相同的两个引用不会因为 `'a` 不同而变成两种类型。

既然 `longest` 的函数体里 `if` 两边分别返回 `x` 和 `y`，为什么不能写成两个不同的生命周期？

因为返回哪一个取决于运行期的长度比较，编译器不做运行期分析。它只能要求"两边都满足"：把两个参数绑到同一个 `'a`，返回值取其中较短的那段。反过来，如果函数确定只返回第一个参数，两个生命周期就可以分开写，调用方也不必让 `y` 活那么久——17.2 的 `Pair` 就是这种写法。

这也是生命周期报错常常显得"不讲理"的根源：你不是在告诉编译器"这个值活多久"，而是在承诺"我返回的东西借自哪里"。承诺写得越松，调用点越自由；写得越紧，限制越多。

## 17.2 标注写在哪：函数、结构体、impl

生命周期参数和类型参数写在同一个尖括号里，位置在函数名之后、值参数之前：

```rust
struct Pair<'a, 'b> {
    left: &'a str,
    right: &'b str,
}

impl<'a, 'b> Pair<'a, 'b> {
    fn left(&self) -> &'a str {
        self.left
    }

    fn right(&self) -> &'b str {
        self.right
    }
}

fn main() {
    let long = String::from("long-lived");
    let left;
    {
        let short = String::from("short-lived");
        let pair = Pair { left: &long, right: &short };
        left = pair.left();
        println!("{} {}", pair.left(), pair.right());
    }
    println!("{}", left);
}
```

三个位置各有含义：

- `struct Pair<'a, 'b>` 声明"这个类型带有两个区域参数"，字段里的 `&'a str` 用的就是它们。
- `impl<'a, 'b> Pair<'a, 'b>` 读作"对任意两个区域 `'a`、`'b`，为 `Pair<'a, 'b>` 实现下面的方法"。这里的 `'a` 是**参数**，不是某个具体区域。
- `fn left(&self) -> &'a str` 的返回值绑定在 `'a` 上，与 `&self` 的借用无关。所以 `pair` 出了内层块之后，`left` 仍能继续用——它借的是 `long`，不是 `pair`。

对比 `right`：它绑定 `'b`。在上面的代码里 `short` 出了块就释放，如果在外层打印 `pair.right()`，编译器会拒绝，因为 `'b` 已经结束。

位置总览：

```text
fn longest<'a>(x: &'a str) -> &'a str     函数 / 方法的泛型参数表
struct Parser<'a> { input: &'a str }      结构体定义
enum Token<'a> { Word(&'a str) }          枚举定义
impl<'a> Parser<'a>                        impl 头
type Slice<'a> = &'a [u8];                类型别名
```

一条容易撞上的规则：**声明了就必须用**。`struct Parser<'a, 'b> { input: &'a str }` 里 `'b` 没出现在任何字段上，编译器报 E0392（lifetime parameter `'b` is never used），并提示删掉它或用 `PhantomData` 标记。结构体不存引用却要带生命周期参数时，通常说明这个参数根本不需要。

## 17.3 省略规则：大多数签名不用写

手写标注是例外，省略才是常态。省略不是"没有生命周期"，而是编译器按三条规则补出来：

1. **每个被省略的输入引用各得一个独立的生命周期参数。** `fn f(x: &str, y: &str)` 展开成 `fn f<'a, 'b>(x: &'a str, y: &'b str)`。
2. **如果恰好只有一个输入生命周期，输出引用就用它。** 这是 `first_line` 能通过的原因。
3. **如果有 `&self` 或 `&mut self`，输出引用就用 `self` 的生命周期。** 方法最常走这条。

规则 3 的例子，一个只读剩余文本的小解析器：

```rust
struct Parser<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn rest(&self) -> &str {
        &self.input[self.pos..]
    }
}

fn main() {
    let mut p = Parser { input: "abcdef", pos: 2 };
    println!("{}", p.rest());
    p.pos = 4;
    println!("{}", p.rest());
}
```

`fn rest(&self) -> &str` 展开后是：

```text
fn rest<'s>(&'s self) -> &'s str
```

返回的引用借用 `self`，`self` 什么时候不能再借，返回值就什么时候失效。这与字段本身的 `'a` 无关：`rest` 返回的是"从 `pos` 到末尾"的切片，它的有效期只需覆盖 `&self` 那次借用。

规则 3 只对带 `self` 的方法生效。关联函数没有 `self`，走规则 2：`fn new(input: &'a str) -> Self` 的返回值直接继承参数的 `'a`。

类型里的 `_` 是占位符，意思是"这里有一个生命周期，名字我不关心"，例如 `Parser<'_>`。它不改变规则，只是省略名字：返回值的生命周期仍然来自唯一的输入引用。

什么时候省略会失败？两个以上的输入引用、没有 `self`、又返回引用。这时三条规则都给不出答案，编译器报 E0106，要求你显式写出返回的引用借自谁——这就是案例 1。

读标准库签名时，可以在心里把省略补全。`str::split` 的签名是 `fn split<'a, P>(&'a self, pat: P) -> Split<'a, P>`，返回值绑的是 `&self` 而不是 `pat`，所以分隔符字符串可以在循环里临时构造，切片本身却必须比迭代器活得久。

## 17.4 结构体持有引用：写一个 Tokenizer

标准库的 `split_whitespace` 已经能切词，但它每次都从头开始，也不能记住位置。真实的词法分析器需要三样东西：原文、当前位置、以及"再给我一个词"的能力。原文不该被复制，所以结构体里存的是引用：

```rust
#[derive(Debug)]
struct Tokenizer<'a> {
    input: &'a str,
    pos: usize,
}

impl<'a> Tokenizer<'a> {
    fn new(input: &'a str) -> Self {
        Tokenizer { input, pos: 0 }
    }

    fn next_token(&mut self) -> Option<&'a str> {
        let bytes = self.input.as_bytes();
        while self.pos < bytes.len() && bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        if self.pos == bytes.len() {
            return None;
        }
        let start = self.pos;
        while self.pos < bytes.len() && !bytes[self.pos].is_ascii_whitespace() {
            self.pos += 1;
        }
        Some(&self.input[start..self.pos])
    }

    fn tokens(mut self) -> Vec<&'a str> {
        let mut out = Vec::new();
        while let Some(tok) = self.next_token() {
            out.push(tok);
        }
        out
    }
}

fn main() {
    let text = String::from("let x = 42;");
    let mut t = Tokenizer::new(&text);
    while let Some(tok) = t.next_token() {
        println!("{:?}", tok);
    }
    println!("{:?}", Tokenizer::new(&text).tokens());
}
```

关键在 `next_token` 的返回类型：`Option<&'a str>`，不是 `Option<&str>`。后者会按规则 3 展开成 `Option<&'s str>`，`'s` 是 `&mut self` 的借用区间，于是每个 token 都锁住整个 `Tokenizer`：

```rust,noplayground
impl<'a> Tokenizer<'a> {
    fn next_token(&mut self) -> Option<&str> {
        self.input.get(self.pos..)
    }
}

fn main() {
    let text = String::from("a b");
    let mut t = Tokenizer::new(&text);
    let first = t.next_token();
    let second = t.next_token();   // error[E0499]：t 还被 first 借着
    println!("{:?} {:?}", first, second);
}
```

写成 `&'a str` 之后，token 借的是 `text`，不是 `t`。`t` 可以继续推进，已经取出的 token 也能一直用到 `text` 失效为止——`tokens` 能把它们收进 `Vec<&'a str>` 就是这个道理。

结构体与原文的关系，编译器会一路盯着：

```rust,noplayground
fn main() {
    let t;
    {
        let text = String::from("temporary");
        t = Tokenizer::new(&text);
    }
    println!("{:?}", t);
}
```

这就是案例 3 的 E0597：`text` 在块尾释放，`Tokenizer<'a>` 里的 `'a` 也随之结束，`t` 不能活到块外。规则很直白：**持有引用的结构体，有效期不得超过被借用的数据。**

### 与"直接拥有 String"的取舍

把字段换成 `String` 也能编译，但用法会变窄：`next_token` 只能写 `fn next_token(&mut self) -> Option<&str>`，返回值借自 `self`。

区别在两处。第一，`next_token` 返回的 `&str` 借自 `self`，所以不能像 `tokens` 那样把多个结果同时收进容器——第二次调用会要求第一次的借用已经结束，收集只能靠 `to_string()` 复制。第二，构造拥有所有权的版本时要付出一次分配和拷贝；换来的是它不依赖外部数据，可以自由返回、存进别的结构体、跨线程传递。

判据很简单：

- **数据已经在内存里、解析是短生命周期操作**：存 `&'a str`，零拷贝，解析器随数据一起消亡。
- **解析结果要长期保存、要跨函数边界返回**：存 `String` 或直接产出拥有所有权的 token，代价是拷贝。
- **有时借、有时拥有**：用 `Cow<'a, str>`，第 6 章已经给过例子。

反过来，如果一开始就把 token 设计成 `String`，上面的生命周期问题全部消失，但每切一个词都要分配一次堆内存。对一个百万行的日志解析器，这就是几十万次额外分配。

### 把 Tokenizer 变成迭代器

既然每次都是"还有没有下一个"，不如实现 `Iterator`。注意 `Item` 带的是 `'a`，不是 `&self` 的区间：

```rust
struct Words<'a> {
    rest: &'a str,
}

impl<'a> Words<'a> {
    fn new(input: &'a str) -> Self {
        Words { rest: input }
    }
}

impl<'a> Iterator for Words<'a> {
    type Item = &'a str;

    fn next(&mut self) -> Option<&'a str> {
        let s = self.rest.trim_start();
        if s.is_empty() {
            self.rest = "";
            return None;
        }
        let end = s.find(char::is_whitespace).unwrap_or(s.len());
        let (word, rest) = s.split_at(end);
        self.rest = rest;
        Some(word)
    }
}

fn main() {
    let text = String::from("alpha beta gamma");
    for w in Words::new(&text) {
        println!("{}", w);
    }
    let all: Vec<&str> = Words::new(&text).collect();
    println!("{:?}", all);
}
```

`next` 拿走 `&mut self`，返回 `Option<&'a str>`。这在借用检查器看来是合法的，因为返回的切片是从 `self.rest`（一个 `&'a str`）复制出来的引用，而不是对 `self` 字段的借用。**区分"借用 self"和"从 self 里复制一个引用出来"，是写迭代器时最关键的一步。** 前者让 `next` 之间互相排斥，后者不会。

第 13 章讲过的适配器现在都能用了：`Words::new(&text).filter(|w| w.len() > 4).count()` 不产生任何 `String`。

## 17.5 'static 与 T: 'a

`'static` 是标注里被误解最多的一个。它的字面含义只有一个：**这个引用在整个程序运行期间都有效**。它出现在两个位置，含义相关但用法不同。

### 位置一：类型里的 &'static T

字符串字面量的类型就是 `&'static str`：字节被编译进可执行文件的只读段，程序退出才消失。同理，`static` 项、以及 `Box::leak` 泄漏出来的引用（第 18 章）都是 `'static`。

这个位置上的 `'static` 无法伪造。运行期从文件读出来的 `String` 不可能变成 `&'static str`，因为它的内存由变量管理，变量一放就没了。想强行延长，只能泄漏：`Box::leak(Box::new(s))` 放弃回收，换来一个 `&'static mut str`。

### 位置二：约束里的 T: 'static

作为约束时，`T: 'static` 读作"**T 里不包含任何短于 `'static` 的借用**"。它描述的是 T 的内容，不是 T 的寿命：

```rust
fn keep<T: 'static>(value: T) -> T {
    value
}

fn main() {
    let owned = String::from("堆上的字符串");
    println!("{}", keep(owned));   // String: 'static，因为它不借用任何东西

    let n = 5;
    println!("{}", keep(n));       // i32: 'static

    // println!("{}", keep(&n));   // error[E0597]：argument requires that `n` is borrowed for `'static`
}
```

`String` 明明会被 drop，为什么满足 `'static`？因为它拥有自己的缓冲区，不指向任何栈上数据；"会被释放"和"是否含短借用"是两件事。反过来，`&n` 借用了一个局部变量，这个借用的区域不可能覆盖整个程序，所以 `T = &i32` 不满足约束。

这条约束最常见的出场是 trait 对象和线程：

```rust
use std::error::Error;

fn load_config() -> Result<(), Box<dyn Error + 'static>> {
    Err("config.toml not found".into())
}

fn main() {
    if let Err(e) = load_config() {
        println!("{}", e);
    }
}
```

`Box<dyn Error>` 默认就等价于 `Box<dyn Error + 'static>`：错误对象要能被返回、被放进 `Result`、跨线程传递，就不能偷偷借着什么局部数据。如果错误类型确实借用了输入，可以写 `Box<dyn Error + '_>`，让编译器把错误绑定到输入上；代价是这个错误值不能活得比输入久。

两个版本的取舍：`+ 'static` 更通用（返回值可以存进任何地方），`+ '_` 更省（错误里直接引用原文，不复制）。标准库把前者设为默认，是因为绝大多数调用方想要一个能自由传递的错误值。

同样的约束也出现在 `std::thread::spawn` 上：它要求闭包 `F: Send + 'static`。原因不是"闭包必须永远存在"，而是新线程可能在创建它的栈帧销毁之后才运行——闭包要是借了那个栈帧里的变量，就是悬垂引用。第 21 章讲线程时会再看到这条约束。

一个实用提醒：**不要给参数写 `&'static str`**，除非你确实只接受字面量。`fn greet(name: &'static str)` 会让所有传 `&String` 的调用方报错，而 `fn greet(name: &str)` 什么都能接。约束越少，API 越好用。

### T: 'a 是什么意思

`T: 'a` 读作"T 活得至少和 `'a` 一样久"，等价的说法是"T 里不含短于 `'a` 的借用"。它保证：把一个 T 放进带 `'a` 的结构体时，T 不会先于那个结构体失效。

多数时候编译器会替你补上，这条规则叫隐含约束（implied bounds）：写下 `&'a T`，等价于同时要求 `T: 'a`。所以下面的写法不用显式加约束：

```rust
struct Holder<'a, T> {
    value: &'a T,
}

impl<'a, T> Holder<'a, T> {
    fn get(&self) -> &'a T {
        self.value
    }
}

fn main() {
    let n = 7u8;
    let h = Holder { value: &n };
    println!("{}", h.get());
}
```

需要手写 `T: 'a` 的场合，是 T 与 `'a` 的关系不直接体现在字段类型里时——例如通过 `PhantomData` 标记，或在 `impl` 与 `where` 子句里想表达"这个实现只对不短于 `'a` 的 T 成立"。标准库签名里看到 `where T: 'a` 时，按上面的读法翻译即可。

## 17.6 深入：区域推理、NLL 的边界与型变

### NLL 算的是"最后一次使用"

第 6 章说过，Rust 2018 之后借用结束于最后一次使用，而不是作用域末尾。实现上，rustc 先把函数体降成 MIR（middle-level IR），在每个程序点上算出哪些借用还活着（liveness），再把这些点收集成"区域"，最后检查冲突。区域是**程序点的集合**，不是花括号划出的区间——这就是"非词法"（non-lexical）的含义。

这套分析有两个后果。好消息是临时借用能提前结束，像 `v.push(4)` 这种曾经必须调整顺序的代码现在能直接写。坏消息是区域仍然是一个整体：一个借用要么在整个集合上有效，要么无效，编译器不会按分支分别判断。

### NLL 过不去的那类代码

"先查，没有就插入，再返回引用"是最典型的例子：

```rust,noplayground
use std::collections::HashMap;

fn get_or_insert<'a>(map: &'a mut HashMap<String, String>, key: &str) -> &'a str {
    match map.get(key) {
        Some(v) => v,
        None => {
            map.insert(key.to_string(), String::new());
            map.get(key).unwrap()
        }
    }
}

fn main() {
    let mut m = HashMap::new();
    println!("{}", get_or_insert(&mut m, "a"));
}
```

```text
error[E0502]: cannot borrow `*map` as mutable because it is also borrowed as immutable
 --> src/main.rs:7:13
  |
3 | fn get_or_insert<'a>(map: &'a mut HashMap<String, String>, key: &str) -> &'a str {
  |                  -- lifetime `'a` defined here
4 |     match map.get(key) {
  |           --- immutable borrow occurs here
5 |         Some(v) => v,
  |                    - returning this value requires that `*map` is borrowed for `'a`
6 |         None => {
7 |             map.insert(key.to_string(), String::new());
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ mutable borrow occurs here

error: aborting due to 1 previous error
```

`map.get(key)` 的借用必须活到 `'a`，因为 `Some` 分支把它返回出去了。NLL 给这个借用算出的区域包含函数里所有可能用到它的点——包括 `None` 分支里的 `insert`。可在 `None` 分支上，那个借用其实已经死了；NLL 无法表达"只在这条路径上有效"，于是报冲突。

这类问题的通用解法是**把"查"和"插"合成一次借用**。HashMap 的 `Entry` API 就是为此设计的：

```rust
use std::collections::HashMap;

fn get_or_insert<'a>(map: &'a mut HashMap<String, String>, key: &str) -> &'a str {
    map.entry(key.to_string()).or_default().as_str()
}

fn main() {
    let mut m = HashMap::new();
    m.insert(String::from("a"), String::from("1"));
    println!("{}", get_or_insert(&mut m, "a"));
    println!("{}", get_or_insert(&mut m, "b"));
}
```

`entry` 一次性拿到槽位，后续的 `or_default` 和 `as_str` 都基于同一次可变借用，没有"先共享、后独占"的切换。

社区里有一个专门的提案要放宽这类限制：**Polonius**。它把分析单位从"区域"换成"借用（loan）"，按控制流判断某个借用在某条路径上是否还可能被使用，于是上面那段代码能通过。到本书写作时（Rust 1.100），Polonius 仍只在 nightly 的 `-Zpolonius` 下提供实验实现，稳定版用的还是 NLL。实际写代码时，记住这个模式就够了：**返回引用的函数里出现"先查再插"，几乎总要改成 Entry**。

### 型变：为什么 `&'a mut T` 会锁死调用点

类型构造器对生命周期有"能不能替换"的规则，叫型变（variance）：

- **协变（covariant）**：长生命周期可以当短的用。`&'long str` 可以传给要 `&'short str` 的位置。
- **不变（invariant）**：只能完全相等。`&'a mut T` 对 T 不变。
- **逆变（contravariant）**：方向相反，只出现在函数参数位置。

常见类型的型变：

```text
类型                    对 'a           对 T
&'a T                   协变            协变
&'a mut T               协变            不变
Box<T> / Vec<T>         —               协变
Cell<T> / RefCell<T>    —               不变
fn(T) -> U              参数逆变        返回协变
```

`&'a mut T` 对 T 不变，是安全性要求，不是实现偷懒。假设允许把 `&mut Vec<&'short u8>` 当成 `&mut Vec<&'long u8>` 用，函数就能通过这个可变引用，把一个只活到 `'short` 的引用塞进一个被当作 `'long` 的容器；调用方之后拿着容器读，读到的就是悬垂引用。共享引用没有这个问题——你只能读，不能往里写。

这条规则和 17.2 的 `impl` 写法叠在一起，就会锁死调用点。案例 4 里的 `fn add(&'a mut self, child: Node<'a>)` 正是这个形状：`Node<'a>` 含 `Vec<Node<'a>>` 与 `&'a str`，而 `&'a mut` 让里面的 `'a` 冻结，于是"借用 root"和"root 里的数据活多久"被绑成同一段区域，第二次调用必然冲突。修法不是加更多标注，而是**把借用的区域和数据的区域拆开**：`fn add(&mut self, child: Node<'a>)` 里那个省略的生命周期是每次调用新生成的短区域，'a 仍然描述节点的数据。

同样的道理适用于函数形式：

```text
fn f<'a>(x: &'a mut Vec<&'a u8>)        // 借用区域与元素区域绑死
fn f<'a, 'b>(x: &'a mut Vec<&'b u8>)    // 拆开：借用是 'a，元素是 'b
```

第二行能接受 `&'short mut Vec<&'short u8>`，第一行不能——它对类型的要求严格到没有调用方能满足，除了恰好两段区域相同的情况。看到 `&'a mut T` 里又出现 `'a`，就该警觉：这通常不是你想要的意思。

生命周期不参与单态化：`fn f<'a>(x: &'a str)` 只会生成一份代码，`&'long str` 与 `&'short str` 在机器码里都是"指针 + 长度"。

## 典型错误案例

下面五个错误覆盖了标注、返回引用、结构体存引用、可变借用与 impl 头五类场景，报错文本来自 rustc 1.100.0 的真实输出。

### 案例 1：两个引用参数却省略标注——E0106

```rust,editable
fn longest(x: &str, y: &str) -> &str {
    if x.len() >= y.len() { x } else { y }
}

fn main() {
    let a = String::from("long");
    let b = String::from("short");
    println!("{}", longest(&a, &b));
}
```

```text
error[E0106]: missing lifetime specifier
 --> src/main.rs:1:33
  |
1 | fn longest(x: &str, y: &str) -> &str {
  |               ----     ----     ^ expected named lifetime parameter
  |
  = help: this function's return type contains a borrowed value, but the signature does not say whether it is borrowed from `x` or `y`
help: consider introducing a named lifetime parameter
  |
1 | fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
  |           ++++     ++          ++          ++
```

**原因**：两个输入引用各自被省略规则补成独立的生命周期，输出引用没有可继承的那个"唯一来源"，编译器拒绝猜。它也猜不了——返回哪个取决于运行期的长度比较。

**修复**：用一个相同的 `'a` 把三者绑在一起，返回值取两者中较短的有效区间：

```rust,editable
fn longest<'a>(x: &'a str, y: &'a str) -> &'a str {
    if x.len() >= y.len() { x } else { y }
}

fn main() {
    let a = String::from("long");
    let b = String::from("short");
    println!("{}", longest(&a, &b));
}
```

### 案例 2：返回了局部变量的切片——E0515

```rust,editable
fn split_csv(input: &str) -> Vec<&str> {
    let owned = input.to_uppercase();
    owned.split(',').collect()
}

fn main() {
    let fields = split_csv("a,b,c");
    println!("{:?}", fields);
}
```

```text
error[E0515]: cannot return value referencing local variable `owned`
 --> src/main.rs:3:5
  |
3 |     owned.split(',').collect()
  |     -----^^^^^^^^^^^^^^^^^^^^^
  |     |
  |     returns a value referencing data owned by the current function
  |     `owned` is borrowed here

error: aborting due to 1 previous error
```

**原因**：签名里的 `Vec<&str>` 按省略规则声称"借自 input"，函数体却把切片建在局部变量 `owned` 上。`owned` 在函数返回时释放，切片随之悬垂。注意报错位置在函数体内部，说明编译器先相信了签名，再检查实现是否兑现。

**修复**：要么让返回值真正借自 input（去掉 `to_uppercase`，直接切原文），要么把结果变成拥有所有权的 `String`：

```rust,editable
fn split_csv(input: &str) -> Vec<String> {
    let owned = input.to_uppercase();
    owned.split(',').map(str::to_string).collect()
}

fn main() {
    let fields = split_csv("a,b,c");
    println!("{:?}", fields);
}
```

### 案例 3：引用活得比被借用的值长——E0597

```rust,editable
fn main() {
    let r;
    {
        let s = String::from("hi");
        r = &s;
    }
    println!("{}", r);
}
```

```text
error[E0597]: `s` does not live long enough
 --> src/main.rs:5:13
  |
4 |         let s = String::from("hi");
  |             - binding `s` declared here
5 |         r = &s;
  |             ^^ borrowed value does not live long enough
6 |     }
  |     - `s` dropped here while still borrowed
7 |     println!("{}", r);
  |                    - borrow later used here
```

**原因**：`r` 的类型是 `&String`，它的区域必须覆盖第 7 行的使用点；而 `s` 在第 6 行的右花括号处被 drop。编译器把三条关键位置都标了出来：声明处、借用处、使用处。

**修复**：让被借用的值活得比引用久——把 `s` 提到与 `r` 同一层，或者干脆返回所有权：

```rust,editable
fn main() {
    let s = String::from("hi");
    let r = &s;
    println!("{}", r);
}
```

### 案例 4：方法签名把借用绑死在结构体的生命周期上——E0499

```rust,editable
struct Node<'a> {
    data: &'a str,
    children: Vec<Node<'a>>,
}

impl<'a> Node<'a> {
    fn add(&'a mut self, child: Node<'a>) {
        self.children.push(child);
    }
}

fn build<'a>(root: &'a mut Node<'a>) {
    root.add(Node { data: "a", children: Vec::new() });
    root.add(Node { data: "b", children: Vec::new() });
}

fn main() {
    let mut root = Node { data: "root", children: Vec::new() };
    build(&mut root);
}
```

```text
error[E0499]: cannot borrow `*root` as mutable more than once at a time
  --> src/main.rs:14:5
   |
12 | fn build<'a>(root: &'a mut Node<'a>) {
   |          -- lifetime `'a` defined here
13 |     root.add(Node { data: "a", children: Vec::new() });
   |     --------------------------------------------------
   |     |
   |     first mutable borrow occurs here
   |     argument requires that `*root` is borrowed for `'a`
14 |     root.add(Node { data: "b", children: Vec::new() });
   |     ^^^^ second mutable borrow occurs here
```

**原因**：`&'a mut self` 里的 `'a` 不是"这次调用的借用区间"，而是结构体参数里那个 `'a`。`&mut` 对 pointee 不变，于是内层 `'a` 被冻结，每次 `add` 都要求借 `root` 整整 `'a`——第一次调用就把 `root` 借到了最后，第二次无从下手。17.6 讲过这条型变规则。

**修复**：方法接收者只写 `&mut self`，让编译器为每次调用生成一个短的借用区间；`Node<'a>` 里的 `'a` 继续描述节点数据：

```rust,editable
struct Node<'a> {
    data: &'a str,
    children: Vec<Node<'a>>,
}

impl<'a> Node<'a> {
    fn add(&mut self, child: Node<'a>) {
        self.children.push(child);
    }
}

fn main() {
    let mut root = Node { data: "root", children: Vec::new() };
    root.add(Node { data: "a", children: Vec::new() });
    root.add(Node { data: "b", children: Vec::new() });
    println!("{} {}", root.data, root.children.len());
}
```

### 案例 5：impl 方法漏了参数的生命周期——E0621

```rust,editable
struct Config<'a> {
    name: &'a str,
}

impl<'a> Config<'a> {
    fn new(name: &str) -> Config<'a> {
        Config { name }
    }
}

fn main() {
    let c = Config::new("demo");
    println!("{}", c.name);
}
```

```text
error[E0621]: explicit lifetime required in the type of `name`
 --> src/main.rs:7:9
  |
7 |         Config { name }
  |         ^^^^^^^^^^^^^^^ lifetime `'a` required
  |
help: add explicit lifetime `'a` to the type of `name`
  |
6 |     fn new(name: &'a str) -> Config<'a> {
  |                   ++
```

**原因**：返回类型写了 `Config<'a>`，参数 `name` 却被省略规则补成另一个独立生命周期 `'1`。签名没有承诺 `name` 活够 `'a`，函数体却把它存进要求 `'a` 的字段。`impl` 头里的 `'a` 不是"随便挑一个区域"，它对整个 impl 块内的签名都成立。

**修复**：把参数写成 `&'a str`，与字段、返回值统一：

```rust,editable
struct Config<'a> {
    name: &'a str,
}

impl<'a> Config<'a> {
    fn new(name: &'a str) -> Config<'a> {
        Config { name }
    }
}

fn main() {
    let c = Config::new("demo");
    println!("{}", c.name);
}
```

## 动手练习

1. **基础标注**：给 `fn longer(x: &str, y: &str) -> &str` 补上生命周期并解释为什么不能只写 `'_`；再写一个只返回 `x` 的版本，让两个参数的生命周期分开。
2. **结构体与 impl**：定义 `struct Excerpt<'a> { part: &'a str }`，实现 `fn announce(&self, announcement: &str) -> &str`。先预测返回的引用借自谁，再写调用代码验证；然后改成返回 `String`，说明编译结果为什么变了。
3. **作品扩展**：给 17.4 的 `Tokenizer` 增加数字 token，让它返回 `Option<Token<'a>>`，其中 `enum Token<'a> { Word(&'a str), Number(&'a str) }`。
4. **型变改错**：把案例 4 的 `fn add(&'a mut self, child: Node<'a>)` 改成 `fn add<'b>(&'b mut self, child: Node<'b>)`，它能编译吗？再试 `fn add<'b, 'c>(&'b mut self, child: Node<'c>)`，比较两个版本的约束强弱。
5. **动手跑一遍**：复现 17.6 的 `get_or_insert`，用 `rustc --edition 2021` 确认 E0502，再改用 `entry` 修复；最后写一个返回 `Option<&str>` 的版本，比较两种 API 的取舍。

## 小结

- 生命周期标注不改变任何值的寿命，它把"引用能活多久"写成签名的一部分，让编译器有据可查。
- 标注出现在四个位置：`&'a T`、`Foo<'a>`、`fn f<'a>()` 的泛型列表、`T: 'a` 约束。
- 省略三规则：每个引用参数各得一个生命周期；只有一个输入时输出继承它；有 `&self` 时输出继承 `self` 的那个。规则之外，编译器一律报 E0106。
- 返回的引用必须指向在约定生命周期内仍然有效的数据。除借自参数或 `self`，也可以返回 `&'static str` 字面量或引用静态项。
- 结构体存非静态借用时需要相应生命周期参数；只存 `&'static str` 时无需声明 `<'a>`，`impl` 头要重复它；方法返回字段引用时用 `&self` 的省略形式最省事。
- `'static` 在类型位置表示"活到程序结束"，在约束位置表示"不含短借用"；`T: 'static` 与 `T` 会不会被 drop 无关。
- NLL 把借用结束点算到最后一次使用，但不是万能：先查再插这类代码仍会冲突，解法是 `Entry` 或等到 Polonius 落地。
- `&'a mut T` 对 `T` 不变，`fn add(&'a mut self, child: Node<'a>)` 会把借用锁死在结构体的生命周期上；拆成两个生命周期通常就能修好。
- 生命周期不参与单态化，全部检查发生在编译期，运行时没有额外开销。
