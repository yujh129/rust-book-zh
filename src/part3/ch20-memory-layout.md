# 内存布局：数据在内存里长什么样

> **本章目标**
>
> - 会用 `size_of` / `align_of` 量出任意类型的尺寸与对齐
> - 理解"对齐"如何导致填充字节，以及字段顺序为什么会改变结构体大小
> - 掌握 `#[repr(C)]`、`#[repr(transparent)]`、`#[repr(packed)]`、`#[repr(align(N))]` 的用途与陷阱
> - 理解枚举的判别式布局，以及 `Option<T>` 的"空指针优化"（niche）
> - 知道 `String`、`&str`、`Vec<T>`、`Box<[T]>`、`dyn Trait` 的尺寸是怎么来的
> - 能判断什么时候该用栈、什么时候必须用堆

写了几十章的 `String`、`Vec`、`Option` 之后，一个问题值得停下来问：**它们在内存里到底占多少字节？** 这不是面试题式的猎奇——FFI、序列化、缓存友好的数据结构、以及"为什么我的结构体复制这么慢"，答案都在布局里。

这一章的所有结论都可以用两个函数现场验证：

```rust
use std::mem::{align_of, size_of};

fn main() {
    println!("u8    {} / {}", size_of::<u8>(), align_of::<u8>());
    println!("i32   {} / {}", size_of::<i32>(), align_of::<i32>());
    println!("u64   {} / {}", size_of::<u64>(), align_of::<u64>());
    println!("usize {} / {}", size_of::<usize>(), align_of::<usize>());
    println!("char  {} / {}", size_of::<char>(), align_of::<char>());
    println!("bool  {} / {}", size_of::<bool>(), align_of::<bool>());
}
```

在 64 位平台上输出：

```text
u8    1 / 1
i32   4 / 4
u64   8 / 8
usize 8 / 8
char  4 / 4
bool  1 / 1
```

注意 `char` 是 **4 字节**（它表示一个 Unicode 标量值，范围 0..=0x10FFFF），而不是 1 字节。`bool` 是 1 字节——尽管它只需要 1 bit。

## 20.1 尺寸与对齐

`size_of::<T>()` 返回 `T` 占多少字节，`align_of::<T>()` 返回它的**对齐要求**。对齐的意思是：这个类型的值的起始地址必须是 N 的整数倍。

为什么需要对齐？因为 CPU 访存有硬件约束和性能差异：

- x86-64 允许非对齐访问，但跨缓存行时会慢，某些指令（如 SSE 的 `movaps`）会直接崩溃
- ARM 的部分指令严格禁止非对齐访问
- 一个 `u64` 如果地址不是 8 的倍数，可能需要两次内存访问

所以编译器保证：**任何 `T` 的地址都能被 `align_of::<T>()` 整除**。对齐是布局的第一条硬约束，尺寸是第二条。

实际规则：

- 结构体的大小 ≥ 各字段大小之和
- 结构体的大小必须是**最大字段对齐**的整数倍
- 结构体的对齐等于**最大字段对齐**

## 20.2 填充：为什么字段顺序会改变大小

```rust
use std::mem::{align_of, size_of};

struct Mixed {
    a: u8,
    b: u32,
    c: u8,
}

fn main() {
    println!("size = {}, align = {}", size_of::<Mixed>(), align_of::<Mixed>());
}
```

输出 `size = 8, align = 4`。字段加起来只有 6 字节，但实际占 8 字节——多出的 2 字节是**填充**（padding）。

原因：`b` 要求 4 字节对齐。如果按声明顺序 `a, b, c` 排，`a` 在 offset 0，`b` 必须在 offset 4（offset 1 不行），于是 1..4 之间空了 3 字节；`c` 在 offset 8；整个结构体对齐 4，大小 12——更糟。

Rust 默认的 `repr(Rust)` **允许编译器重排字段**。它实际的做法是把大对齐的字段先放：

```text
offset 0..4   b: u32
offset 4..5   a: u8
offset 5..6   c: u8
offset 6..8   padding
```

于是 8 字节就够了。这就是为什么**在 Rust 里，结构体的字段顺序不一定等于内存顺序**。

### 手动优化字段顺序

用 `#[repr(C)]` 时字段顺序被固定，填充就回来了：

```rust
use std::mem::size_of;

#[repr(C)]
struct CStruct {
    a: u8,
    b: i32,
    c: u8,
}

struct RustStruct {
    b: i32,
    a: u8,
    c: u8,
}

fn main() {
    println!("#[repr(C)] 声明顺序 a,b,c: {}", size_of::<CStruct>());
    println!("手动排成 b,a,c:           {}", size_of::<RustStruct>());
}
```

输出：

```text
#[repr(C)] 声明顺序 a,b,c: 12
手动排成 b,a,c:           8
```

在实际项目里，如果一个结构体被大量复制（`Copy` 类型进数组、进 `Vec`），把 `u64`/`f64` 放前面、`u8`/`bool` 放后面，能省下可观的内存。不过要注意：**先测量再优化**，别为了 4 字节把可读性搞垮。

### 什么时候必须关心填充

- 结构体被放进大数组：`Vec<Item>` 有 100 万个元素时，每个省 4 字节就是 4 MB
- 结构体要跨进程/跨网络发送：填充字节是未初始化的，可能泄露栈上的旧数据
- FFI：C 的布局规则和 `repr(C)` 一致，但字段顺序必须匹配

## 20.3 repr 家族：控制布局的四个开关

Rust 用 `#[repr(...)]` 控制布局。默认是 `repr(Rust)`（编译器自由重排，**不保证跨编译版本稳定**）。

### `#[repr(C)]`：C 兼容布局

`repr(C)` 结构体按声明顺序放置字段，再按目标 C ABI 对齐。传给 C 的每个字段也要有兼容的表示，外层属性不会改变内层结构体的布局。

```rust
#[repr(C)]
struct Point {
    x: f64,
    y: f64,
}

fn main() {
    let p = Point { x: 1.0, y: 2.0 };
    println!("{} bytes", std::mem::size_of_val(&p));
    // 可以安全地转成 C 里的 struct Point { double x, y; }
}
```

无字段的 `repr(C)` 枚举采用目标 C ABI 对枚举的表示，大小可能随目标变化。带载荷时，它相当于一个结构体，先放判别式，再放能容纳各变体载荷的联合体，中间和末尾都可能有填充。

### `#[repr(transparent)]`：零开销包装

只有一个非零大小字段的结构体，可以声明为"透明"——它在 ABI 上与内部字段完全相同：

```rust
#[repr(transparent)]
struct Meters(f64);

fn takes_f64(x: f64) -> f64 {
    x * 2.0
}

fn main() {
    let m = Meters(3.5);
    // 与 f64 布局完全一致，可安全用于 FFI
    println!("{} bytes", std::mem::size_of::<Meters>());
    println!("{}", takes_f64(m.0));
}
```

这让"newtype 模式"可以零成本地用于 FFI——你不会因为包了一层 `Meters` 就改变 ABI。但注意限制：除了承载布局的字段，其他字段必须大小为 0、对齐为 1。实现 `Drop` 本身不禁止透明表示；若涉及所有权，跨语言调用还需单独约定谁负责析构。

### `#[repr(packed)]`：消灭填充（危险）

`packed` 强制去掉所有填充，代价是字段可能**不对齐**：

```rust
#[repr(packed)]
struct PackedHeader {
    tag: u8,
    len: u32,
    flags: u16,
}

fn main() {
    println!("size = {}", std::mem::size_of::<PackedHeader>());
}
```

输出 `size = 7`（1 + 4 + 2，无填充；在本章目标上，按 `tag, len, flags` 声明的 `repr(C)` 版本占 12 字节）。这在解析网络协议头时很有用，因为协议就是这么定义的。

但 `packed` 有一个致命限制：**不能为可能未对齐的字段创建引用**。因为 `&p.len` 会是一个未对齐的 `&u32`，而 Rust 规定 `&T` 必须对齐（否则是 UB）。编译器会用 `E0793` 拦住你。取值时要先复制到局部变量：

```rust
#[repr(packed)]
struct PackedHeader {
    tag: u8,
    len: u32,
}

fn main() {
    let h = PackedHeader { tag: 1, len: 100 };

    // 正确：复制出来
    let len = h.len;
    println!("{} {}", h.tag, len);

    // 错误（编译不过）：let r = &h.len;
}
```

### `#[repr(align(N))]`：提高对齐

有时需要让数据的起始地址对齐到缓存行（64 字节），避免"伪共享"（false sharing）：

```rust
use std::mem::{align_of, size_of};

#[repr(align(64))]
struct CacheLine {
    value: u64,
}

fn main() {
    println!("size = {}, align = {}", size_of::<CacheLine>(), align_of::<CacheLine>());
}
```

输出 `size = 64, align = 64`。这个技巧在多线程计数器里很常见：每个线程的计数器独占一条缓存行，就不会和别的线程抢同一条线上的数据。

`repr(align)` 可以和 `repr(C)` 组合，但**不能和 `repr(packed)` 组合**（`E0587`）。

### 四个开关的关系

```rust
#[repr(C, align(16))]
struct AlignedBuffer {
    data: [u8; 8],
}

fn main() {
    println!("{} / {}", std::mem::size_of::<AlignedBuffer>(), std::mem::align_of::<AlignedBuffer>());
}
```

输出 `16 / 16`。`repr(C)` 定字段顺序，`align` 提高对齐（顺带把大小撑到 16）。`repr(Rust)` + `align` 也是合法的。

## 20.4 枚举的布局：判别式与 niche 优化

枚举比结构体多一个东西：**判别式**（discriminant），用来记录"当前是哪个变体"。

```rust
use std::mem::size_of;

enum Shape {
    Circle(f64),
    Square(f64),
    Empty,
}

fn main() {
    println!("size = {}", size_of::<Shape>());
}
```

输出 `16`：8 字节判别式（为了对齐把 `u8` 判别式撑到 8）+ 8 字节的 `f64`。一个只有 8 字节载荷的枚举，实际要 16 字节。

如果所有变体都没有载荷，判别式就是全部：

```rust
enum Color {
    Red,
    Green,
    Blue,
}

fn main() {
    println!("{}", std::mem::size_of::<Color>());
}
```

输出 `1`。三个变体只需要 2 bit，但最小分配单位是 1 字节。

### 关键：`Option<T>` 为什么不额外占空间

```rust
use std::mem::size_of;
use std::num::NonZeroU32;

fn main() {
    println!("Option<i32>       {}", size_of::<Option<i32>>());
    println!("Option<u32>       {}", size_of::<Option<u32>>());
    println!("Option<NonZeroU32> {}", size_of::<Option<NonZeroU32>>());
    println!("Option<Box<u8>>   {}", size_of::<Option<Box<u8>>>());
    println!("Option<&u8>       {}", size_of::<Option<&u8>>());
    println!("Option<bool>      {}", size_of::<Option<bool>>());
}
```

输出：

```text
Option<i32>       8
Option<u32>       8
Option<NonZeroU32> 4
Option<Box<u8>>   8
Option<&u8>       8
Option<bool>      1
```

`Option<u32>` 要 8 字节（4 字节载荷 + 4 字节判别式），但 `Option<NonZeroU32>` 只要 4 字节——因为 `NonZeroU32` **不可能是 0**，所以 0 可以拿来表示 `None`。这个"空闲的非法值"就叫 **niche**。

同样的道理：

- 本章 64 位目标上，`T: Sized` 时 `Option<Box<T>>` 和 `Option<&T>` 是 8 字节，因为指针永远非空（`Box`/`&` 不能是 null），null 表示 `None`
- `Option<bool>` 是 1 字节，因为 `bool` 只有 0 和 1 合法，2 可以表示 `None`
- `Option<String>` 也是 24 字节——`String` 内部的指针是 `NonNull`，同样有 niche

这个优化贯穿标准库，也是"Rust 的 `Option` 零成本"这一说法的主要依据。你在写库时也可以借它：如果一个整数天然不为零，用 `NonZeroU32` 而不是 `u32`，就能让 `Option` 免费。

```rust
use std::num::NonZeroU32;

struct UserId(NonZeroU32);

fn main() {
    let id = NonZeroU32::new(42).unwrap();
    // 结构体大小与内部字段一致
    println!("{}", std::mem::size_of::<UserId>());
}
```

### 手写 niche 优化

自己定义的枚举拿不到这个优化（除非用 `#[repr(transparent)]` 或 nightly 特性），但用 `NonZero` 系列做字段可以：

```rust
use std::num::NonZeroUsize;

enum Slot {
    Occupied(NonZeroUsize),
    Empty,
}

fn main() {
    println!("{}", std::mem::size_of::<Slot>());
}
```

输出 `8`，而不是 `16`——`Empty` 用 0 表示，`Occupied` 存非零下标。

## 20.5 标准类型的布局

### `String`、`Vec<T>`、`&str`

```rust
use std::mem::size_of;

fn main() {
    println!("String        {}", size_of::<String>());
    println!("&str          {}", size_of::<&str>());
    println!("Vec<u8>       {}", size_of::<Vec<u8>>());
    println!("Option<String> {}", size_of::<Option<String>>());
    println!("&[u8]         {}", size_of::<&[u8]>());
    println!("Box<[u8]>     {}", size_of::<Box<[u8]>>());
}
```

输出：

```text
String        24
&str          16
Vec<u8>       24
Option<String> 24
&[u8]         16
Box<[u8]>     16
```

解释：

- `String` 和 `Vec<T>` 都是"三元组"：`(ptr, cap, len)`，每个 8 字节，共 24
- `&str` 和 `&[T]` 是**胖指针**：`(ptr, len)`，共 16
- `Box<[T]>` 也是胖指针：`(ptr, len)`，但拥有所有权
- `Option<String>` 仍是 24，靠 `NonNull` 的 niche

注意 `String` 和 `&str` 的差别不是"有没有堆"，而是"谁拥有堆"：`String` 拥有，`&str` 只是指向别人的内存（可能在堆、在栈、在静态区）。

```rust
fn main() {
    let s: String = String::from("hello");
    let slice: &str = &s[1..3];

    println!("String 24 字节 -> 堆上的 5 字节");
    println!("&str 16 字节 -> 指向堆上的第 1..3 字节");
    println!("{}", slice);
}
```

### `dyn Trait`：vtable 从哪来

```rust
trait Shape {
    fn area(&self) -> f64;
}

struct Circle {
    r: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        3.14159 * self.r * self.r
    }
}

fn main() {
    let c = Circle { r: 1.0 };
    let r: &Circle = &c;
    let d: &dyn Shape = &c;

    println!("&Circle    {} 字节", std::mem::size_of_val(&r));
    println!("&dyn Shape {} 字节", std::mem::size_of_val(&d));
    println!("面积 {}", d.area());
}
```

输出 `&Circle 8 字节`、`&dyn Shape 16 字节`。`&dyn Shape` 是一个胖指针：`(data_ptr, vtable_ptr)`。

vtable 是编译器为每个 `(具体类型, trait)` 组合生成的静态表，里面是函数指针和类型信息。调用 `d.area()` 时，运行期从 vtable 里取出 `area` 的地址再间接调用——这就是"动态分派"的全部机制。

```text
&dyn Shape ──┬──> data: Circle { r: 1.0 }
             └──> vtable: [drop_in_place, size, align, area]
```

因为 vtable 的存在，`dyn Trait` 比泛型慢一点（一次间接跳转，且无法内联），但换来"一个容器装多种类型"的能力。

## 20.6 ZST：零大小类型

```rust
struct Marker;

fn main() {
    println!("{}", std::mem::size_of::<Marker>());
    println!("{}", std::mem::size_of::<[Marker; 1000]>());
    println!("{}", std::mem::size_of::<Option<Marker>>());
}
```

输出 `0`、`0`、`1`。ZST（Zero-Sized Type）不占空间：

- `struct Marker;` 是 ZST
- `[Marker; 1000]` 仍是 0 字节
- `Option<Marker>` 是 1 字节（需要区分 `None`）

ZST 在标准库里到处都是：`std::marker::PhantomData`、闭包捕获为空时的闭包类型、`HashMap` 的默认哈希器。

`PhantomData<T>` 是"我不存 `T`，但我需要编译器认为我拥有 `T`"的标记：

```rust
use std::marker::PhantomData;

struct Handle<T> {
    id: usize,
    _marker: PhantomData<T>,
}

fn main() {
    let h: Handle<String> = Handle { id: 1, _marker: PhantomData };
    println!("{} 字节", std::mem::size_of_val(&h));
}
```

输出 `8`。`Handle<String>` 和 `Handle<u8>` 是不同的类型（类型安全），但都不多占一个字节。

ZST 还有一个有趣的推论：`Vec<()>` 的元素不占空间，所以 `push` 永远不分配内存：

```rust
fn main() {
    let mut v: Vec<()> = Vec::new();
    for _ in 0..3 {
        v.push(());
    }
    println!("len = {}, capacity = {}", v.len(), v.capacity());
}
```

输出 `len = 3, capacity = 18446744073709551615`——`Vec` 对 ZST 把 capacity 设成 `usize::MAX`，表示"内存无限"。这是标准库对 ZST 的特殊处理。

## 20.7 栈与堆

```rust
fn main() {
    let on_stack = 42;                       // 栈：编译期确定大小
    let on_heap = Box::new(42);              // 堆：运行期分配

    println!("{} {}", on_stack, on_heap);

    let big = [0u8; 4096];                   // 4 KiB 的局部数组
    println!("{}", big.len());
}
```

- 栈：函数调用时一次性分配，大小编译期已知，分配/释放只是一次指针加减
- 堆：通过分配器申请内存，大小可以在运行时决定；代价取决于大小、分配器和负载

数组的元素直接存放在数组值里；这个值可以是局部变量，也可以是堆上结构体的字段。`Box<T>`、`Vec<T>` 把所有权句柄和所管理的堆存储分开。

**常见的栈溢出场景**：

```rust,noplayground
fn main() {
    // 递归太深，或超大数组
    let arr = [0u8; 8 * 1024 * 1024]; // 8 MB，超过 Windows 默认 1 MB 栈
    println!("{}", arr.len());
}
```

这段在 Windows 上会直接 abort（`thread 'main' has overflowed its stack`），而且无法用 `catch_unwind` 捕获。要处理大数据，用 `Vec` 或 `Box::new([0u8; N])`。


## 20.8 析构：Drop 与销毁顺序

前面七节讲的都是「内存怎么摆」。这一节讲「内存什么时候被收回去」——`Drop` 是 Rust 把「释放资源」写进类型系统的地方。

### Drop 是什么

```rust
struct Noisy(&'static str);

impl Drop for Noisy {
    fn drop(&mut self) {
        println!("离开：{}", self.0);
    }
}

fn main() {
    let _a = Noisy("a");
    let _b = Noisy("b");
    println!("main 即将结束");
}
```

输出：

```text
main 即将结束
离开：b
离开：a
```

三条要点：

1. `Drop::drop(&mut self)` 由编译器在值离开作用域时自动调用，**你不需要也不能**显式调用它（见下面案例 1）。
2. 局部变量按**声明的逆序**销毁：后声明的先走，像栈一样。
3. `Drop` 与所有权绑定：值被移动，析构责任也跟着转移；只有最后一个持有者负责销毁。

### 完整的销毁顺序（实测）

```rust
struct Noisy(&'static str);

impl Drop for Noisy {
    fn drop(&mut self) {
        println!("销毁 {}", self.0);
    }
}

struct Pair {
    a: Noisy,
    b: Noisy,
}

fn main() {
    let _p = Pair { a: Noisy("字段 a"), b: Noisy("字段 b") };
    let _x = Noisy("局部 x");
    let _y = Noisy("局部 y");
    println!("--- 局部变量声明完毕");
    let v = vec![Noisy("v0"), Noisy("v1"), Noisy("v2")];
    println!("--- vec 长度 {}", v.len());
}
```

本机实测输出（rustc 1.100，去掉字段未读取的警告）：

```text
--- 局部变量声明完毕
--- vec 长度 3
销毁 v0
销毁 v1
销毁 v2
销毁 局部 y
销毁 局部 x
销毁 字段 a
销毁 字段 b
```

这张输出把三条规则一次说清了：

| 容器 | 顺序 | 原因 |
|---|---|---|
| 局部变量 | 声明顺序的**逆序** | 等价于栈的弹出 |
| 结构体字段 | 声明顺序的**正序** | 字段按内存顺序依次 drop |
| `Vec<T>` / 数组 | 从前往后 | 按下标依次 drop |
| 元组 | `t.0` → `t.1` | 同结构体 |

注意「局部变量逆序」与「字段正序」并不矛盾：`_p` 先于 `_x` 声明，所以 `_p` 最后销毁；而 `_p` 内部的字段按 `a`、`b` 的正序销毁。

### 什么时候 drop

```rust
struct Noisy(&'static str);

impl Drop for Noisy {
    fn drop(&mut self) {
        println!("销毁 {}", self.0);
    }
}

fn main() {
    let a = Noisy("a");
    {
        let inner = Noisy("inner");
        println!("内层作用域里 inner 还活着");
    }
    println!("内层作用域已结束");

    let b = Noisy("b");
    let _moved = b; // 所有权转移给 _moved，二者没有区别
    println!("b 已被移走");

    let c = Noisy("c");
    let _ = c; // 立即 drop，不等到作用域结束
    println!("c 已立即销毁");

    drop(a); // 等价于 std::mem::drop，提前销毁
    println!("a 已提前销毁");
}
```

输出顺序：`inner` → `c` → `a` → `_moved`（原 `b`）。两个容易忽略的细节：

- `let _ = c;` 会立刻丢弃，因为 `_` 不是绑定，值没有主人；而 `let _c = c;` 会活到作用域结束。这是「下划线变量」最常踩的坑。
- `drop(x)` 只是 `std::mem::drop` 的调用，它按值接收参数，函数返回时参数被销毁——这正是「提前析构」的实现方式。

### 两个必须知道的编译器报错

**案例 1：手动调用析构函数——E0040**

```rust,noplayground
struct Guard;

impl Drop for Guard {
    fn drop(&mut self) { println!("guard 销毁"); }
}

fn main() {
    let g = Guard;
    g.drop();
}
```

```text
error[E0040]: explicit use of destructor method
 --> src/main.rs:7:7
  |
7 |     g.drop();
  |       ^^^^ explicit destructor calls not allowed
  |
help: consider using `drop` function
  |
7 -     g.drop();
7 +     drop(g);
  |
```

**原因**：如果允许手动调用，`g` 在作用域结束时又会自动 drop 一次，同一个资源被释放两次（double free）。编译器直接封死这条路。

**修复**：用 `drop(g)`，它把值移进函数并在函数内销毁，之后 `g` 不可用——所有权系统保证了「最多一次」。

**案例 2：从实现了 Drop 的类型里移出字段——E0509**

```rust,noplayground
struct Named { name: String }

impl Drop for Named {
    fn drop(&mut self) { println!("{}", self.name); }
}

fn main() {
    let n = Named { name: String::from("a") };
    let s = n.name;
    println!("{}", s);
}
```

```text
error[E0509]: cannot move out of type `Named`, which implements the `Drop` trait
 --> src/main.rs:7:13
  |
7 |     let s = n.name;
  |             ^^^^^^
  |             |
  |             cannot move out of here
  |             move occurs because `n.name` has type `String`, which does not implement the `Copy` trait
  |
help: consider borrowing here
  |
7 |     let s = &n.name;
  |             +
```

**原因**：`Drop::drop` 会在 `n` 销毁时读取 `self.name`。如果把 `name` 移走，`n` 就处于「部分未初始化」状态，`drop` 里再读它就是读未初始化内存。所以「实现了 Drop 的类型」不允许移动出字段。

**修复**：借用、克隆，或者用 `Option<String>` + `take()` 把字段换成 `None`（这也正是很多库的做法）：

```rust
struct Named { name: String }

impl Drop for Named {
    fn drop(&mut self) { println!("销毁 {}", self.name); }
}

struct Safe { name: Option<String> }

impl Drop for Safe {
    fn drop(&mut self) {
        println!("销毁 {:?}", self.name);
    }
}

fn main() {
    let n = Named { name: String::from("a") };
    println!("借用的长度 {}", n.name.len());

    let mut s = Safe { name: Some(String::from("b")) };
    let taken = s.name.take(); // 字段变成 None，所有权合法地移出
    println!("取到 {:?}", taken);
    // s 在这里销毁时 name 是 None
}
```

### 跳过析构：ManuallyDrop 与 forget

有时你**不希望**值被自动销毁——例如某块内存要交给 C 释放，或者要把它转移进一块手工管理的内存。

```rust
use std::mem::ManuallyDrop;

struct Res(u32);

impl Drop for Res {
    fn drop(&mut self) { println!("释放 {}", self.0); }
}

fn main() {
    let a = ManuallyDrop::new(Res(1));
    let b = Res(2);
    println!("a 的字段是 {}", a.0);
    drop(b);
    println!("结束：只有 b 被释放，a 泄漏了");
}
```

输出只有 `释放 2`。`ManuallyDrop<T>` 与 `T` 布局完全相同，只是告诉编译器「别自动 drop 它」。这是 `Vec`、`String` 内部实现的基础设施：它们自己管理缓冲区，不能依赖字段的自动析构。

`std::mem::forget(x)` 是另一种跳过方式：它按值接收 `x` 并什么都不做。两者的区别是 `ManuallyDrop` 之后还能 `take()` 回来，`forget` 则是彻底放弃。二者都是**安全的**（不产生 UB），只是泄漏内存——Rust 的安全承诺不包含「一定不泄漏」。

### 深入：drop 的代码到底长什么样

编译器为每个需要析构的类型生成一个隐藏函数（drop glue）。以 `Pair` 为例，等价于：

```rust,noplayground
// 编译器生成的伪代码，不可手写
fn drop_pair(p: &mut Pair) {
    drop_in_place(&mut p.a);
    drop_in_place(&mut p.b);
}
```

三个推论：

1. **drop 是静态分派的**：编译器在编译期就知道每个类型的 drop glue，不存在运行期查表。`dyn Trait` 的 vtable 里也带一个 drop 指针（第 20.5 节讲的 vtable 布局就包含它）。
2. **`Copy` 与 `Drop` 互斥**：`Copy` 类型按位复制，若它还有析构逻辑就会重复释放，所以 `#[derive(Copy)]` 与 `impl Drop` 同时出现会被编译器拒绝（E0184）。
3. **drop 里 panic 是危险的**：如果 drop 已经在处理另一个 panic（栈展开过程中），再 panic 会直接 `abort`。这也是为什么 `Drop` 实现里极少用 `unwrap()`。

```rust,noplayground
// E0184：Copy 与 Drop 不能共存
#[derive(Copy, Clone)]
struct Both(u32);

impl Drop for Both {
    fn drop(&mut self) {}
}

fn main() {
    let _ = Both(1);
}
```

```text
error[E0184]: the trait `Copy` cannot be implemented for this type; the type has a destructor
 --> src/main.rs:2:8
  |
1 | #[derive(Copy, Clone)]
  |          ---- in this derive macro expansion
2 | struct Both(u32);
  |        ^^^^ `Copy` not allowed on types with destructors
  |
note: destructor declared here
 --> src/main.rs:5:5
  |
5 |     fn drop(&mut self) {}
  |     ^^^^^^^^^^^^^^^^^^
```
## 典型错误案例

### 案例 1：`packed` 与 `align` 同时出现——E0587

```rust,editable
#[repr(C, align(8), packed)]
struct Bad {
    a: u8,
}

fn main() {
    let _ = Bad { a: 1 };
}
```

```text
error[E0587]: type has conflicting packed and align representation hints
 --> src/main.rs:2:1
  |
2 | struct Bad { a: u8 }
  | ^^^^^^^^^^

error: aborting due to 1 previous error
```

**原因**：`packed` 要求"对齐降到 1"，`align(8)` 要求"对齐升到 8"，两者在语义上直接矛盾。编译器不做折中，直接拒绝。

**修复**：二选一。需要紧凑布局用 `packed`，需要高对齐用 `align`：

```rust,editable
#[repr(C)]
struct Compact {
    a: u8,
}

#[repr(C, align(8))]
struct Aligned {
    a: u8,
}

fn main() {
    println!("{} {}", std::mem::size_of::<Compact>(), std::mem::size_of::<Aligned>());
}
```

### 案例 2：取 `packed` 结构体字段的引用——E0793

```rust,editable
#[repr(packed)]
struct Packed {
    a: u8,
    b: u32,
}

fn main() {
    let p = Packed { a: 1, b: 2 };
    let r = &p.b;
    println!("{}", r);
}
```

```text
error[E0793]: reference to field of packed struct is unaligned
 --> src/main.rs:6:13
  |
6 |     let r = &p.b;
  |             ^^^^
  |
  = note: this struct is 1-byte aligned, but the type of this field may require higher alignment
  = note: creating a misaligned reference is undefined behavior (even if that reference is never dereferenced)
  = help: copy the field contents to a local variable, or replace the reference with a raw pointer and use `read_unaligned`/`write_unaligned`

error: aborting due to 1 previous error
```

**原因**：`b` 在 offset 1，而 `u32` 需要 4 字节对齐。`&p.b` 会得到一个地址不是 4 的倍数的 `&u32`，而 Rust 保证 `&T` 永远对齐——违反它就是 UB，所以编译器用借用检查器拦住你。注意报错说的是"即使不解引用也是 UB"：`&T` 这个类型本身就携带了对齐承诺。

**修复**：复制到局部变量（`Copy` 类型），或用 `std::ptr::read_unaligned`：

```rust,editable
#[repr(packed)]
struct Packed {
    a: u8,
    b: u32,
}

fn main() {
    let p = Packed { a: 1, b: 2 };

    // 方案一：复制（字段是 Copy 类型时可读）
    let b = p.b;
    println!("{} {}", p.a, b);

    // 方案二：非对齐读取
    let raw: u32 = unsafe { std::ptr::read_unaligned(std::ptr::addr_of!(p.b)) };
    println!("{}", raw);
}
```

### 案例 3：对不定长类型求尺寸——E0277

```rust,editable
fn main() {
    println!("{}", std::mem::size_of::<str>());
}
```

```text
error[E0277]: the size for values of type `str` cannot be known at compilation time
 --> src/main.rs:2:40
  |
2 |     println!("{}", std::mem::size_of::<str>());
  |                                        ^^^ doesn't have a size known at compile-time
  |
  = help: the trait `Sized` is not implemented for `str`
note: required by an implicit `Sized` bound in `std::mem::size_of`
 --> src/main.rs:2:40
  |
  = note: ...
```

**原因**：`str`、`[T]`、`dyn Trait` 都是**不定长类型**（unsized），大小取决于运行期的长度。`size_of::<T>` 的 `T` 有隐式 `Sized` bound，所以 `str` 不满足。

**修复**：用 `&str`（胖指针，16 字节），或用 `size_of_val` 对具体的值求大小：

```rust,editable
fn main() {
    println!("{}", std::mem::size_of::<&str>());

    let s: &str = "hello";
    println!("{}", std::mem::size_of_val(s));
    println!("{}", s.len());
}
```

`size_of_val(s)` 对 `&str` 返回 16（胖指针本身的尺寸），`s.len()` 返回 5（内容的字节数）。两者容易混淆。

### 案例 4：`transmute` 尺寸不匹配——E0512

```rust,editable
fn main() {
    let x: u32 = 1;
    let y: u64 = unsafe { std::mem::transmute(x) };
    println!("{}", y);
}
```

```text
error[E0512]: cannot transmute between types of different sizes, or dependently-sized types
 --> src/main.rs:3:27
  |
3 |     let y: u64 = unsafe { std::mem::transmute(x) };
  |                           ^^^^^^^^^^^^^^^^^^^
  |
  = note: source type: `u32` (32 bits)
  = note: target type: `u64` (64 bits)

error: aborting due to 1 previous error
```

**原因**：`transmute` 是"把这些字节重新解释成另一个类型"，类型尺寸必须完全相同，否则会读越界或留下垃圾字节。`u64` 有 64 位，源只有 32 位。

**修复**：显式转换，保留语义：

```rust,editable
fn main() {
    let x: u32 = 1;
    let y: u64 = x as u64;
    println!("{}", y);

    // 两种类型大小相同，且 u32 的每个字节组合都是有效的 [u8; 4]
    let bits: [u8; 4] = unsafe { std::mem::transmute(x) };
    println!("{:?}", bits);
}
```

注意字节序：`transmute` 得到的是内存中的原始字节，在小端机器上 `x = 1` 会变成 `[1, 0, 0, 0]`。跨平台序列化要用 `to_le_bytes`。

### 案例 5：透明的结构体有两个字段——E0690

```rust,editable
#[repr(transparent)]
struct Pair {
    a: u32,
    b: u32,
}

fn main() {
    let p = Pair { a: 1, b: 2 };
    println!("{}", p.a + p.b);
}
```

```text
error[E0690]: transparent struct needs at most one non-trivial field, but has 2
 --> src/main.rs:2:1
  |
2 | struct Pair {
  | ^^^^^^^^^^^ needs at most one non-trivial field, but has 2
3 |     a: u32,
  |     ------ this field has non-zero size
4 |     b: u32,
  |     ------ this field has non-zero size

error: aborting due to 1 previous error
```

**原因**：`transparent` 的语义是"这个类型与它唯一的非零字段 ABI 完全相同"。有两个非零字段时，它就不再是"透明"的，而是普通结构体。

**修复**：去掉 `transparent`，或只保留一个载荷字段（其他字段必须是 ZST，例如 `PhantomData`）：

```rust,editable
use std::marker::PhantomData;

#[repr(transparent)]
struct Tagged<'a> {
    value: u32,
    _marker: PhantomData<&'a ()>,
}

fn main() {
    let t = Tagged { value: 7, _marker: PhantomData };
    println!("{} {}", t.value, std::mem::size_of_val(&t));
}
```

## 动手练习

**练习 1**：写一个 `struct Order { id: u8, amount: u64, flag: bool }`，先按这个顺序打印 `size_of`，再重排字段让尺寸最小，用 `#[repr(C)]` 版本对比。说明差多少字节、差在哪。

**练习 2**：用 `NonZeroU32` 实现 `struct Id(NonZeroU32)`，并写一个 `enum Item { Present(Id), Missing }`。打印 `size_of::<Item>()`，再换成 `u32` 版本对比，解释差异来源。

**练习 3**：协议头是 `[u8 tag][u32 len][u16 flags]`，共 7 字节。用 `#[repr(packed)]` 建模它，写一个函数从 `&[u8]` 解析出这三个字段（提示：`std::ptr::read_unaligned` + `copy_from_slice` 都可以），并验证 `size_of` 是 7 而不是 8。

**练习 4**：写两个 `#[repr(align(64))]` 的计数器，各开一个线程递增一千万次，测量总耗时；再写一个不加 `align` 的版本（两个计数器相邻）对比。这是伪共享的实际演示（按需调大循环次数）。

**练习 5**：制作一张"尺寸表"：对 `u8`/`i32`/`String`/`&str`/`Vec<u8>`/`Box<[u8]>`/`&dyn Display`/`Box<dyn Display>`/`Option<&u8>`/`Option<u32>`/`Option<NonZeroU32>` 各打印 size 与 align，然后逐一解释为什么是这个数字。

## 小结

- `size_of` 给尺寸，`align_of` 给对齐；任何 `T` 的地址必须是 `align_of::<T>()` 的倍数
- 结构体大小必须是最大字段对齐的整数倍，多出来的字节是填充
- `repr(Rust)` 允许编译器重排字段以减少填充，所以**字段顺序不等于内存顺序**
- `repr(C)` 固定为 C 的布局规则，FFI 必须用；`repr(transparent)` 让 newtype 零成本；`repr(packed)` 降低字段对齐要求，不能借用可能未对齐的字段（`E0793`）；`repr(align(N))` 提高对齐
- 枚举自带判别式，尺寸 = max(判别式, 最大变体) 向上取整到对齐
- niche 优化让 `Option<&T>`、`Option<Box<T>>`、`Option<NonZeroU32>`、`Option<bool>` 不额外占字节
- `String`/`Vec<T>` 是 `(ptr, cap, len)` 共 24 字节；`&str`/`&[T]`/`Box<[T]>` 是 `(ptr, len)` 共 16 字节
- `&dyn Trait` 是胖指针 `(data, vtable)`，16 字节；动态分派就是查 vtable
- ZST 不占空间，`PhantomData<T>` 让类型参数化但零开销
- 局部大数组可能耗尽线程栈，堆分配灵活但有分配器开销
