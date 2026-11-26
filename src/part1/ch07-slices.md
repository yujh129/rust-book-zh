# 第 7 章 切片与字符串的边界

> **本章目标**
> - 说清 str、&str、String 三者的分工，知道每个类型该出现在哪个位置
> - 从"指针 + 长度"的角度理解切片，能解释 &[T] 为什么占 16 字节
> - 按字节安全地切分字符串，用 chars、char_indices、get 替代裸下标
> - 掌握 split_at、chunks、windows、lines、trim 这组 API 的取舍
> - 给函数选出正确的参数类型，并说清 Deref 强制转换替你做了什么
> - 分清哪些越界在编译期被挡住、哪些要等到运行期 panic

第 4 章已经用过切片：`&v[1..3]` 取一段，`&s[..]` 借整个字符串。那时只需要知道"能取"，不需要知道它为什么能取、取错了会怎样。

这一章补上这两件事。字符串的麻烦在于：**Rust 的字符串是 UTF-8 字节序列，而切片下标按字节计数**。中文和 emoji 占多个字节，随手写下的 `&s[0..5]` 就可能落在某个字符的中间。编译期查不出来，运行期 panic。

先把结论放在前面：切片是"指针 + 长度"，不拥有数据；字符串切片的下标是字节偏移，不是字符序号。本章剩下的内容基本都是这两句话的推论。

## 7.1 str、&str 与 String

前几章一直在用 `String::from` 和字符串字面量，却从没解释过为什么字面量的类型是 `&str` 而不是 `String`。要回答这个问题，得先分清三个类型。

### 三种类型的分工

- `str` 是"一段 UTF-8 字节"这个类型本身。它不带长度信息，长度存放在指向它的指针里，所以单独一个 `str` 无法确定大小。
- `String` 是拥有者：栈上三个字段（指针、容量、长度），字节放在堆上。它可以增长、可以修改。
- `&str` 是借用：一个指向字节的指针加一个字节长度，指向别人的 UTF-8 数据，自己不拥有。

`&str` 可以指向 `String` 的整个缓冲区、缓冲区的一段、或者程序里的一段只读静态数据（字面量就属于最后一种）：

```rust
fn main() {
    let owned: String = String::from("hello");   // 拥有堆上的字节
    let borrowed: &str = &owned;                 // 借用整个字符串
    let literal: &str = "hello";                 // 指向只读静态数据
    let part: &str = &owned[1..3];               // 借用其中一段

    println!("{owned} / {borrowed} / {literal} / {part}");
    println!("{} {} {}", owned.len(), borrowed.len(), part.len());
}
```

三个值打印出来都是文本，但内存里的形态完全不同：

| 类型 | 拥有数据 | 栈上大小 | 典型来源 |
| --- | --- | --- | --- |
| String | 是 | 24 字节 | String::from、to_string、format! |
| &str | 否 | 16 字节 | 字面量、&String、函数参数 |
| str | 否 | 无固定大小 | 只能通过指针出现 |

表中的数字在 64 位平台上成立：`String` 的三个字段各 8 字节，`&str` 的指针与长度各 8 字节。这个"16 字节"是下一节的主角。

有一件事容易误解：`&str` 不是"指向 String 的引用"，而是"指向 str 的引用"。`String` 能变成 `&str`，是因为它实现了 `Deref<Target = str>`，编译器在中间替你插了一次解引用（7.5 节展开）。所以 `&String` 与 `&str` 是两个不同的类型，前者是指向结构体的普通指针（8 字节），后者才是胖指针。

### UTF-8：长度不等于字符数

UTF-8 是变长编码：一个字符占 1 到 4 个字节。

| 内容 | 每个字符字节数 | 例子 |
| --- | --- | --- |
| ASCII | 1 | a、0、空格 |
| 拉丁扩展、希腊、西里尔 | 2 | e 上加声调、α |
| 中日韩、常见符号 | 3 | 你、好、， |
| emoji、生僻汉字 | 4 | 大多数表情、扩展 B 区汉字 |

由此推出本章最重要的一条事实：`s.len()` 返回的是**字节数**，不是字符数。

```rust
fn main() {
    let s = String::from("héllo，世界");

    println!("len()   = {}", s.len());            // 字节数
    println!("chars() = {}", s.chars().count());  // 字符数
    println!("bytes   = {:?}", &s.as_bytes()[..6]);

    for (i, c) in s.char_indices().take(4) {
        println!("字节 {i:>2} -> {c}");
    }
}
```

`é` 占 2 字节，`，` 占 3 字节，所以字节数明显大于字符数。`char_indices` 给出的 `i` 是每个字符的**起始字节偏移**，这也解释了下一节要讲的事：合法的切片边界只有这些偏移，加上末尾的 `len`。

还有一个更细的层次：一个 `char` 是 Unicode 标量值（scalar value），不是"人眼看到的字"。`é` 可以写成单个码位，也可以写成 `e` 加一个组合重音符；后者是两个 `char`，看起来却只有一个字。要按"用户感知的字符"切分，需要字素簇（grapheme cluster）分割，标准库不提供，得用 unicode-segmentation 之类的 crate。本章只处理 `char` 这一层。

顺便回答开头的问题：字符串字面量直接编译进二进制的只读数据段，没有运行期分配，也没有所有者，所以类型是 `&'static str`。写 `let s = "hi"` 时不需要堆，也就不需要 `String`。

## 7.2 切片的本质：指针 + 长度

切片要解决的问题很具体：函数想处理"一串元素"，但不想拥有它，也不想为它分配内存。C 的做法是把数组退化成指针，长度另外传一个参数（`void sum(const int *xs, size_t n)`），两个值必须配对，错了没人管。

Rust 把这两个值合成一个类型：`&[T]` 就是一个指向首元素的指针加一个元素个数。这种"指针 + 额外元数据"的引用叫胖指针（fat pointer）。它仍然是一个值，可以整体传递，但不再是 8 字节。

```rust
use std::mem::size_of;

fn main() {
    println!("&u8     = {} 字节", size_of::<&u8>());
    println!("&[u8]   = {} 字节", size_of::<&[u8]>());
    println!("&str    = {} 字节", size_of::<&str>());
    println!("String  = {} 字节", size_of::<String>());
    println!("&String = {} 字节", size_of::<&String>());

    let v = vec![10, 20, 30, 40, 50];
    let mid: &[i32] = &v[1..4];
    println!("{:?}，len = {}", mid, mid.len());
}
```

在 64 位平台上输出是 8、16、16、24、8。普通引用 `&u8` 只有指针；`&[u8]` 和 `&str` 都是 16 字节；`&String` 又回到 8 字节，因为它指向的是那个 24 字节的结构体，长度藏在结构体里。

内存里大致是这样：

```text
String s（栈，24 字节）              堆缓冲区（UTF-8 字节）
┌──────────┬──────┬──────┐        ┌────┬────┬────┬────┬────┐
│ ptr      │ cap  │ len  │───────▶│ h  │ e  │ l  │ l  │ o  │
└──────────┴──────┴──────┘        └────┴────┴────┴────┴────┘
                                     ▲              ▲
                                     │              │
                              ┌──────┴────┬─────────┴──┐
                              │ ptr        │ len = 3    │   ← &s[1..4]
                              └────────────┴────────────┘
```

由此得到切片的三个性质：

1. **创建切片是 O(1)**。只写 16 字节，不动堆上的数据。
2. **切片不拥有数据**。原数据被释放后切片就悬垂，所以借用检查器要求切片的生命周期不超过原数据。
3. **切片可以再切片**。`&mid[1..]` 只是把指针往后挪、长度减一，仍然指向同一块内存。

把 `mid` 再切一次（`&mid[1..3]`）不会产生新内存：指针往后挪、长度减一，它和 `all[2..4]` 是同一段内存的两个视图。切片之间允许重叠，因为共享引用本来就可以有多个别名——只要没人写。

要写就得换成 `&mut [T]`，布局一模一样，规则变成独占。`&mut [T]` 不是 `Copy`：把可变切片赋给另一个变量会移动它，避免出现两个能写的别名。`&[T]` 是 `Copy` 的，随便复制。

范围语法是 `Index` trait 的语法糖。`&v[1..4]` 展开后大致是 `&*Index::index(&v, 1..4)`，`str` 与 `[T]` 各自实现了 `Index<Range<usize>>`，语义不同：`[T]` 按元素下标，`str` 按字节偏移。这个差别是下一节的主题。

## 7.3 按字节索引

`&s[a..b]` 里的 `a` 和 `b` 是字节偏移，不是字符序号。合法条件有三个：

1. `a <= b`，否则 panic；
2. `b <= s.len()`，否则 panic；
3. `a` 与 `b` 都落在字符边界上，否则 panic。

前两条和数组越界是一回事，第三条是字符串独有的。看一个例子：

```rust,noplayground
fn main() {
    let s = "你好世界";
    println!("{}", &s[0..5]);
}
```

```text
thread 'main' panicked at src/main.rs:3:22:
end byte index 5 is not a char boundary; it is inside '好' (bytes 3..6 of string)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

`你` 占 0..3，`好` 占 3..6，字节 5 是"好"的第三个字节。从这里切开，前半段就不再是合法的 UTF-8，任何按 UTF-8 解码的代码都会读到半个字符。Rust 选择当场 panic，而不是交给你一段可能是乱码的字节。

报错信息里的 `(bytes 3..6 of string)` 直接告诉你这个字节属于哪个字符、该字符占哪一段，照着改就行。

### is_char_boundary 与安全取值

`str::is_char_boundary(i)` 判断偏移 `i` 能不能作为切片边界。规则很短：`i == 0` 和 `i == len` 永远为真；其余位置看该字节的高两位，凡是 UTF-8 的续字节（形如 `10xxxxxx`）都不是边界。

```rust
fn main() {
    let s = "你好，Rust";

    println!("0  -> {}", s.is_char_boundary(0));
    println!("3  -> {}", s.is_char_boundary(3));
    println!("4  -> {}", s.is_char_boundary(4));

    println!("{:?}", s.get(0..3));
    println!("{:?}", s.get(0..4));

    for (i, c) in s.char_indices() {
        print!("{i}:{c}  ");
    }
    println!();
}
```

`get` 是索引的 Option 版本：范围非法时返回 `None`，不 panic。凡是"用户输入的位置可能不合法"的场合，都应该用 `get` 而不是下标。

这两个工具合起来能写"按字节上限截断、但不切坏字符"的函数：从上限往回退，最多退 3 个字节必然落到边界上（一个字符最长 4 字节）。标准库里同语义的 `floor_char_boundary` 尚未稳定。

### 四个遍历接口

| 接口 | 产出 | 用途 |
| --- | --- | --- |
| chars | char | 逐字符处理，最常用 |
| char_indices | (usize, char) | 需要知道字节位置时 |
| bytes | u8 | 处理 ASCII、协议解析 |
| as_bytes | &[u8] | 一次性拿到全部字节，零拷贝 |

`chars` 与 `bytes` 都是 O(n)：前者要解码 UTF-8，后者直接读字节。由此带来一个性能陷阱：**`s.chars().nth(k)` 是 O(k)**，在循环里按下标遍历字符会退化成 O(n²)。需要随机访问字符时，先 `let cs: Vec<char> = s.chars().collect()`，或者建一张字节偏移表。

### 与其他语言的对比

同样是"取第 5 个字符"，各语言的代价差别很大：Python 的 `s[5]` 最符合直觉，代价是字符串按最大码位宽度存储、内存占用更高；Java 的下标是 UTF-16 码元，一个 emoji 占两个，按下标切会切出孤立代理项；C 和 Go 按字节下标，切坏了静默产生乱码，C 越界还是未定义行为。Rust 也选字节——零拷贝地和操作系统、网络、文件打交道，这些接口本来就是字节流——但把两类错误都变成当场 panic 或可处理的 `None`。

## 7.4 切片 API 地图

标准库给切片和 str 配了一大批方法，名字相近、语义差别不小。按"你要做什么"分成三组来看。

### 切成两半：split_at 与 split_at_mut

`split_at(i)` 在偏移 `i` 处切一刀，返回两个切片。对 `str` 而言 `i` 同样是字节偏移，落在字符中间会 panic。

`split_at_mut` 解决的是第 6 章留下的问题：借用检查器不允许同时可变借用同一个 Vec 的两处。切成两个不重叠的切片之后，两个 `&mut` 就都合法了。

```rust
fn main() {
    let mut v = vec![1, 2, 3, 4, 5, 6];
    let (left, right) = v.split_at_mut(3);

    for (a, b) in left.iter_mut().zip(right.iter_mut()) {
        std::mem::swap(a, b);
    }
    println!("{v:?}");

    let (head, tail) = "hello world".split_at(5);
    println!("{head:?} {tail:?}");
}
```

同类方法还有 `split_first`、`split_last`、`split_once`（按分隔符切一次，返回 `Option`）。它们的共同点是"一次产出固定数量的切片"，因此不需要生命周期标注就能安全返回。

### 分块与滑窗：chunks、chunks_exact、windows

```rust
fn main() {
    let v: Vec<i32> = (1..=7).collect();

    for c in v.chunks(3) {
        print!("{c:?} ");
    }
    println!();

    for c in v.chunks_exact(3) {
        print!("{c:?} ");
    }
    println!("余数 {:?}", v.chunks_exact(3).remainder());

    for w in v.windows(3).take(2) {
        print!("{w:?} ");
    }
    println!();

    println!("{:?}", v.first_chunk::<3>());
    println!("{:?}", v.last_chunk::<3>());
}
```

区别在一张表里：

| 方法 | 每块长度 | 是否重叠 | 末尾不足一块 |
| --- | --- | --- | --- |
| chunks(n) | n | 否 | 作为一个短块产出 |
| chunks_exact(n) | n | 否 | 丢弃，可用 remainder() 取出 |
| windows(n) | n | 是，每次右移一个 | 不产出，长度不足时为空迭代器 |
| rchunks(n) | n | 否 | 从右往左切 |

`chunks_exact` 的价值在于每块长度都等于 `n`，编译器与优化器能用上这个信息；`windows` 常用来比较相邻元素，比如判断数组是否单调递增。两个函数在 `n == 0` 时都会 panic——空块没有意义。

### 字符串专用：按空白切、按行切、去空白

```rust
fn main() {
    let text = "  alpha beta\tgamma \n delta\r\nepsilon ";

    let words: Vec<&str> = text.split_whitespace().collect();
    println!("{} 个词: {:?}", words.len(), words);

    let lines: Vec<&str> = "a\r\nb\nc".lines().collect();
    println!("{:?}", lines);

    println!("{:?}", "  x  ".trim());
    println!("{:?}", "k=v".split_once('='));
    println!("{:?}", "2024-01-02".split('-').collect::<Vec<_>>());
}
```

几个容易踩的点：

- `split_whitespace` 把连续空白当成一个分隔符，并且自动去掉首尾空白；`split(' ')` 会产生空字符串。
- `lines` 按 `\n` 切，并去掉行尾的 `\r`，所以 Windows 换行不会留在返回值里；`split('\n')` 会把 `\r` 留下。
- `trim` 去掉两端所有 Unicode 空白（不只是空格），`trim_matches` 可以自定义要剥掉的字符。
- `split` 返回迭代器而不是 Vec，需要收集时才分配；直接用 `for` 遍历零分配。

## 7.5 参数为什么收 &str 而不是 &String

先看一个看起来无害的签名：`fn word_count(s: &String) -> usize`。它能跑，但只对 `&String` 有效：字面量 `"c d"` 是 `&str`，传进去报 expected `&String`, found `&str`；`&"c d"` 是 `&&str`，也不行。Deref 的方向是 `String` 到 `str`，反过来没有路。

把参数换成 `&str`，一个签名能接住所有字符串形态：

```rust
fn word_count(s: &str) -> usize {
    s.split_whitespace().count()
}

fn total(xs: &[i32]) -> i32 {
    xs.iter().sum()
}

fn main() {
    let owned = String::from("a b c");
    let boxed: Box<str> = "d e".into();
    let shared: std::sync::Arc<str> = std::sync::Arc::from("f g");

    println!("{} {} {}", word_count(&owned), word_count(&boxed), word_count(&shared));
    println!("{}", word_count("h i j"));

    let arr = [1, 2, 3];
    let v = vec![4, 5, 6];
    println!("{} {} {}", total(&arr), total(&v), total(&v[1..]));
}
```

转换发生在编译期，机器码里没有额外步骤：编译器把 `&String` 解引用再取地址，等价于你手写 `&*owned`。这就是 Deref 强制转换（deref coercion）。

### 规则与生效位置

规则只有三条：

1. 若 `T: Deref<Target = U>`，则 `&T` 可以当 `&U` 用；`&mut T` 同理变成 `&mut U`。
2. `&mut T` 可以退化成 `&T`，反过来不行——那会凭空造出写权限。
3. 允许连续多次，编译器一直解引用到类型匹配为止。

它只发生在少数几个位置（coercion site）：函数实参、带类型标注的 `let`、结构体字段初始化、`return` 的返回值、方法调用的接收者。别的位置需要手写 `&*x`。

链的长度取决于嵌套层数：`&Arc<str>` 到 `&str` 要解两层，`&Box<str>` 解一层。方法调用更宽松一些，`s.len()` 会自动找到 `str` 上的 `len`，这条规则叫自动解引用（autoderef）。

泛型不享受这个待遇。`fn show<T>(x: T)` 收到 `&String` 时，`T` 就固定成 `&String`，不会变成 `&str`。需要"什么字符串都收"的泛型接口，写 `fn show<T: AsRef<str>>(s: T)`，内部用 `s.as_ref()` 拿到 `&str`。`AsRef` 走的是 trait 方法，与 Deref 是两套机制：前者是"能借成"这种能力的显式声明，后者是智能指针的通用解引用，第 15 章讲 trait 时会回到这个区别。

什么时候该收 `&String`？几乎不需要。要修改内容时收 `&mut String`（`&mut str` 只能整体替换字符，不能改长度）；要把字符串存起来时收 `String`，明确表示所有权转移。其余场合一律 `&str`。

## 7.6 深入：布局、转换与边界检查

### 胖指针的确切布局

`&str` 与 `&[T]` 在内存里都是两个字：一个地址、一个长度。顺序由编译器决定，语言不保证，你也不该依赖。

```rust
use std::mem::size_of;

fn main() {
    println!("&str          = {}", size_of::<&str>());
    println!("&[u8; 4]      = {}", size_of::<&[u8; 4]>());
    println!("Option<&str>  = {}", size_of::<Option<&str>>());
    println!("Option<&[u8]> = {}", size_of::<Option<&[u8]>>());
}
```

四个值分别是 16、8、16、16。`&[u8; 4]` 是普通引用，长度为 4 这件事写在类型里，不需要运行期携带，所以只有 8 字节——这也解释了为什么把数组转成切片（`&arr[..]`）才会变成 16 字节。

`Option<&str>` 也是 16 字节，没有因为多一个 `None` 分支而膨胀。原因是空指针优化（niche optimization）：引用永远不为空，编译器把全零的指针位模式借给 `None` 用。同理 `Option<Box<T>>`、`Option<&mut T>` 都不增加大小。这不是语法糖，是布局层面的优化，`std::mem::size_of` 能直接验证。

以 `&owned[..]` 为例，编译器只是从 `String` 结构体里读两个字段（偏移 0 的指针、偏移 16 的长度），拼成 `(ptr, len)`。没有函数调用、没有 trait 对象、没有运行期查表：Deref 是编译期完成的类型改写，手写 `&s[..]` 和通过 `&str` 传参生成的汇编完全一样。

### 边界检查的代价

切片下标会生成一次比较加一个 panic 分支：若 `end > len` 就调用 `panic_bounds_check`。字符串切片还多两次 `is_char_boundary` 检查。

这条分支通常可以忽略，因为：

- 分支预测器几乎从不猜错（合法访问占绝大多数）；
- 在 `for x in &v` 这类迭代器循环里，长度不变，编译器能把检查提到循环外，甚至消掉——标准库的 `slice::Iter` 内部用的就是无检查访问；
- 检查失败的那条路径被标记为冷路径（cold），不占指令缓存。

但它**不会因为开了 `--release` 就消失**。panic 是语义的一部分，优化器只能在能证明安全时删除检查。确实需要压掉时用 `get_unchecked`，那是不安全代码，责任转移给你（第 35 章）。

对比 C++：`v[i]` 不做检查，`v.at(i)` 做检查并抛异常。Rust 把 `[]` 定义成检查版，把无检查版放进 `unsafe`——默认安全，需要时显式放弃。

### str 为什么是 unsized

`str` 表示"一段合法的 UTF-8 字节"，但这段有多少字节是运行期才知道的。类型的大小必须在编译期确定（栈帧布局、数组元素间距都依赖它），所以 `str` 没有 `Sized`，不能作为变量类型、不能按值传参、不能放进数组。

凡是"大小不固定"的类型都这样：`[T]`、`str`、`dyn Trait`。它们只能躲在指针后面，由指针补上缺失的那部分信息——切片补长度，trait 对象补虚表指针。这也解释了为什么胖指针恰好是"普通指针 + 一份元数据"。

泛型参数默认带 `Sized` 约束，所以 `fn f<T>(x: T)` 里的 `T` 不能是 `str`。要接受 unsized 类型，写 `T: ?Sized`，并且参数必须改成引用：

```rust
use std::mem::size_of_val;

fn byte_len<T: ?Sized>(x: &T) -> usize {
    size_of_val(x)
}

fn main() {
    let s: &str = "hello";
    let arr = [1u64, 2, 3];

    println!("{}", byte_len(s));
    println!("{}", byte_len(&arr[..]));
}
```

`size_of_val` 从胖指针里读出长度，所以能对 unsized 值工作——它拿到的正是那份"指针里带的元数据"。

## 典型错误案例

### 案例 1：把中文按字节切开

```rust,editable
fn main() {
    let s = "你好世界";
    println!("{}", &s[0..5]);
}
```

```text
thread 'main' panicked at src/main.rs:3:22:
end byte index 5 is not a char boundary; it is inside '好' (bytes 3..6 of string)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

**原因**：下标 5 落在"好"的第三个字节上。从这里切开，前半段以半个 UTF-8 字符结尾，不再是合法的 `str`，所以标准库在切片时直接 panic，而不是交出一段坏数据。

**修复**：按字符取，或用 `get` 让非法范围返回 `None`：

```rust,editable
fn main() {
    let s = "你好世界";
    println!("{:?}", s.get(..5));                        // None，不 panic
    println!("{}", s.chars().take(1).collect::<String>()); // 你
}
```

### 案例 2：遍历字符时修改字符串

```rust,editable
fn main() {
    let mut s = String::from("abc");
    for c in s.chars() {
        if c == 'b' {
            s.push('!');
        }
    }
}
```

```text
error[E0502]: cannot borrow `s` as mutable because it is also borrowed as immutable
 --> src/main.rs:5:13
  |
3 |     for c in s.chars() {
  |       -        ---------
  |       |        |
  |       |        immutable borrow occurs here
  |       |        immutable borrow later used here
  | |
4 | |         if c == 'b' {
5 | |             s.push('!');
  | |             ^^^^^^^^^^^ mutable borrow occurs here
6 | |         }
7 | |     }
  | |_____- this for loop borrows `s` immutably, preventing mutation within its body
```

**原因**：`chars()` 借走了 `&s`，迭代器还在读缓冲区；`push` 可能触发堆扩容并释放旧内存，让迭代器手里的指针悬垂。这与第 6 章"遍历时 push Vec"是同一个错误，只是换成了字符串。

**修复**：先算出结果再替换，不要让读写重叠：

```rust,editable
fn main() {
    let mut s = String::from("abc");
    s = s.replace('b', "b!");
    println!("{s}");
}
```

### 案例 3：切片范围超出长度

```rust,editable
fn main() {
    let v = vec![1, 2, 3, 4, 5];
    let mid = &v[2..10];
    println!("{mid:?}");
}
```

```text
thread 'main' panicked at src/main.rs:3:17:
range end index 10 out of range for slice of length 5
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

**原因**：`v` 的长度 5 要到运行期才确定（`Vec` 的缓冲区在堆上），编译器不会替你做这个检查，`Index` 实现里只有一句 `assert!`。范围来自计算时（`i + n`、用户输入、协议字段）这种情况极常见。

**修复**：`get` 返回 `Option`，把越界变成可处理的分支：

```rust,editable
fn main() {
    let v = vec![1, 2, 3, 4, 5];
    let mid = v.get(2..10);
    println!("{mid:?}");
}
```

### 案例 4：从切片里移出元素

```rust,editable
fn first(v: &[String]) -> String {
    v[0]
}

fn main() {
    let v = vec![String::from("a"), String::from("b")];
    println!("{}", first(&v));
}
```

```text
error[E0508]: cannot move out of type `[String]`, a non-copy slice
 --> src/main.rs:2:5
  |
2 |     v[0]
  |     ^^^^
  |     |
  |     cannot move out of here
  |     move occurs because `v[_]` has type `String`, which does not implement the `Copy` trait
  |
help: consider cloning the value if the performance cost is acceptable
  |
2 |     v[0].clone()
  |         ++++++++
```

**原因**：`v[0]` 是一个位置表达式，`String` 不实现 `Copy`，要把它赋值出去就得搬走三个字段。但 `&[String]` 只是借用，搬走会在原 `Vec` 里留下一个空洞，所以编译器拒绝。

**修复**：借出去，而不是搬出去：

```rust,editable
fn first(v: &[String]) -> &str {
    v[0].as_str()
}

fn main() {
    let v = vec![String::from("a"), String::from("b")];
    println!("{}", first(&v));
}
```

确实需要一份独立副本时写 `v[0].clone()`，代价是堆分配，要写明白。

### 案例 5：把 str 当成有大小的类型

```rust,editable
fn greet(name: str) {
    println!("hello, {name}");
}

fn main() {
    greet("world");
}
```

```text
error[E0277]: the size for values of type `str` cannot be known at compilation time
 --> src/main.rs:1:16
  |
1 | fn greet(name: str) {
  |                ^^^ doesn't have a size known at compile-time
  |
  = help: the trait `Sized` is not implemented for `str`
help: function arguments must have a statically known size, borrowed types always have a known size
  |
1 | fn greet(name: &str) {
  |                +
```

**原因**：`str` 是 unsized 类型，长度在运行期才知道，而函数参数要在栈上占一个大小固定的位置。报错里后续还会有 `E0308: expected str, found &str`，说的都是同一件事。

**修复**：加一个 `&` 变成 `&str`，长度由胖指针携带；需要持有所有权时改成 `String`。

## 动手练习

1. 写 `fn safe_slice(s: &str, a: usize, b: usize) -> Option<&str>`，要求 `a <= b`、`b <= s.len()` 且两端都是字符边界，否则返回 `None`。
2. 用 `chunks(8)` 实现 `hex_dump(data: &[u8])`，每行输出"偏移 + 8 个十六进制字节 + ASCII 预览"，ASCII 之外的字节显示为点。
3. 用 `split_whitespace` 找出文本中最长的单词；长度相同时返回最先出现的那个。再用 `split(|c: char| !c.is_alphanumeric())` 做一版，比较两者对中文标点的处理差异。
4. 实现 `fn split_chars(s: &str, n: usize) -> (&str, &str)`，按字符数而不是字节数切开，只用一次 `char_indices` 扫描。
5. 用 `windows(2)` 判断 `&[i32]` 是否严格递增，再用 `split_at_mut` 原地反转一个 `Vec<i32>`，说明后者为什么必须用可变切片。

## 小结

- `str` 是一段 UTF-8 字节，unsized；`String` 拥有堆缓冲区（指针、容量、长度三字段）；`&str` 是指针加长度的借用。
- 切片是胖指针：`&[T]`、`&str` 各 16 字节，创建 O(1)，不拷贝数据，生命周期不得超过原数据。
- 字符串下标按字节计数；合法范围要求两端都落在字符边界上，否则 panic。`len()` 是字节数，`chars().count()` 才是字符数。
- 用 `get` 代替下标处理不可信位置，用 `char_indices`/`is_char_boundary` 计算边界，`chars().nth(k)` 是 O(k)。
- `split_at_mut`、`chunks_exact`、`windows`、`lines`、`trim` 覆盖了绝大多数切分需求；`lines` 会去掉行尾的 `\r`，`split('\n')` 不会。
- 参数收 `&str` 与 `&[T]`，Deref 强制转换让 `&String`、`&Box<str>`、`&Arc<str>` 都能直接传入，代价为零。
- Deref 转换只发生在函数实参、带标注的 `let`、字段初始化、返回值这些位置；泛型不触发，需要 `AsRef<str>`。
- 越界检查是运行期分支，release 也不会消失；`get` 返回 `Option`，`get_unchecked` 才是不安全出口。
