# 第 36 章 类型系统进阶

> **本章目标**
>
> - 会用 newtype 与 typestate 把状态机编码进类型，让越序调用在编译期失败
> - 能说出 `PhantomData` 的四种用途，并在需要时选对标记类型
> - 会用常量泛型写 `Matrix<const R: usize, const C: usize>` 与固定容量容器
> - 能在关联类型与泛型参数之间做出选择，判据是同一个类型能否多次实现
> - 会用 sealed trait 封住实现，用超 trait 模拟 trait 别名
> - 看懂协变、逆变与不变，能解释 `Cell<T>` 为什么不能协变

第 14 章讲了泛型的语法与单态化，第 16 章讲了关联类型、`dyn` 与对象安全。这两章解决的是「怎么用」；这一章处理的是「用类型表达约束」——把「这个操作只能在这个状态下做」「这两个类型不能互换」写进类型，交给编译器检查，而不是写进注释等人自觉。

类型系统是一种证明工具。每当你声明一个变量、参数或返回值，你都在向编译器提交一条命题；如果一个函数能通过类型检查，这条命题就被证明了。第 8 章的 newtype 是这条思路最小的一步：把 `f64` 包成 `Meters`，于是「米」和「秒」再也不可能在参数位置上互换。本章把这个手法推到更远处——用类型参数记录状态，用常量参数记录尺寸，用标记类型记录所有权与变型。

先看一段不用类型表达状态的代码。一个 HTTP 请求要经过草稿、已审阅、已发送三个阶段，发送之后不能再改，审阅之前不能发送。用枚举加运行期检查来写，会得到这样的结构：

```rust,noplayground
enum Stage {
    Draft,
    Reviewed,
    Sent,
}

struct Request {
    body: String,
    stage: Stage,
}

impl Request {
    fn send(&mut self) {
        if !matches!(self.stage, Stage::Reviewed) {
            panic!("只有审阅后的请求才能发送");
        }
        self.stage = Stage::Sent;
    }
}
```

这段代码能跑，但约束是运行期的：调用方可以传一个草稿态请求进来，程序在半夜 panic。更糟的是 `stage` 字段是公开的，任何人都能把它改成 `Stage::Sent`，绕过检查。类型系统在这里没有帮上任何忙——所有状态都共享同一个类型 `Request`，编译器看谁都一样。

## 36.1 类型即证明：newtype 与 typestate

把状态从「一个字段的值」挪到「类型参数」上，就得到 typestate（类型状态）模式。请求类型变成 `Request<S>`，`S` 是一个只用来记账的标记类型：

```rust
use std::marker::PhantomData;

#[derive(Debug)]
struct Draft;
#[derive(Debug)]
struct Reviewed;
#[derive(Debug)]
struct Sent;

#[derive(Debug)]
struct Request<S> {
    id: u32,
    body: String,
    _state: PhantomData<S>,
}

impl Request<Draft> {
    fn new(id: u32, body: &str) -> Self {
        Request { id, body: body.to_string(), _state: PhantomData }
    }

    fn review(self) -> Request<Reviewed> {
        Request { id: self.id, body: self.body, _state: PhantomData }
    }
}

impl Request<Reviewed> {
    fn approve(self) -> Request<Sent> {
        Request { id: self.id, body: self.body, _state: PhantomData }
    }
}

fn deliver(req: Request<Sent>) {
    println!("deliver #{}: {}", req.id, req.body);
}

fn main() {
    let draft = Request::new(7, "hello");
    let reviewed = draft.review();
    let sent = reviewed.approve();
    deliver(sent);
}
```

这段程序里没有一处运行期状态检查，但下面每一行都写不进去：

```text
draft.approve();       // error[E0599]: no method named `approve` found for struct `Request<Draft>`
deliver(draft);        // error[E0308]: expected `Request<Sent>`, found `Request<Draft>`
draft.review().review(); // error[E0599]: `review` 只在 `Request<Draft>` 上定义，`Request<Reviewed>` 没有
```

机制有三层。第一，每个阶段是一个独立的零大小类型，`Request<Draft>` 与 `Request<Sent>` 是没有任何关系的两个类型，不能互相赋值。第二，状态迁移函数接收 `self` 而不是 `&mut self`，它把旧值消费掉，返回新类型的值；想「原地改状态」就必须把旧值交出来，因此不存在「改到一半又用旧状态」的窗口。第三，标记类型 `Draft`、`Sent` 只出现在 `PhantomData<S>` 里，不占字段空间。`PhantomData<S>` 是零大小类型（ZST），所以 `Request<Draft>` 和 `Request<Sent>` 的内存布局完全相同，迁移只是移动已有的 `String`，没有额外分配。在 64 位平台上，`Request<Draft>` 与 `Request<Sent>` 的 `size_of` 都是 24（`String` 占 24 字节），`PhantomData` 一个字节都没加。

typestate 有一条容易被忽略的前提：字段必须是私有的。如果 `body` 和 `_state` 是 `pub`，同 crate 内的代码（更不用说下游）可以随手构造 `Request { body, _state: PhantomData }`，把 `Request<Sent>` 凭空造出来，证明就作废了。类型状态模式保护的是「跨模块的调用秩序」，它保护不了模块内部——在定义这些类型的模块里，任何人都能直接访问私有字段。所以正确做法是把字段设为私有，只暴露受控的构造函数与迁移方法。

代价也要说清。状态多起来以后，每个状态都要一个 `impl` 块，迁移方法是「只在这一种状态上可用」的，读代码时得来回跳。类型参数会让错误信息变长：一个 `Request<Reviewed>` 的错误里会同时出现 `Request<Reviewed>`、`Request<Sent>` 和标记类型名。你不能把不同状态的请求放进同一个 `Vec`，除非在边界处把它们统一成一个枚举——而这恰好是应该做的：内部用 typestate 保证秩序，对外用枚举收敛为运行期表示。

标准库和生态里有不少 typestate 的实例。`std::fs::File` 本身不区分状态，但 `std::process::Command` 的 builder 用可变借用来保证顺序；嵌入式生态里的 `embedded-hal` 用 typestate 描述 GPIO 引脚：一个引脚先被配置成输入，再想 `set_high` 就会编译失败。判断一个场景该不该用 typestate，只看一件事：越序操作是「使用者写错了」还是「运行期数据决定的」？前者用类型挡住，后者只能运行期判断。

## 36.2 PhantomData 的四种用途

36.1 里的 `PhantomData<S>` 看起来只是个占位符，让编译器别抱怨「类型参数 `S` 没被使用」。它确实有这个作用，但远不止于此：`PhantomData<T>` 是一个零大小的「声明」，告诉编译器「这个类型在语义上含有 `T`」，从而影响自动 trait（`Send`、`Sync`）、析构检查（drop check）和型变（variance）。同一个 `PhantomData`，填进去的类型不同，语义就完全不同。

先看不加标记会发生什么：

```rust,noplayground
struct Tag<T> {
    value: u32,
}
```

```text
error[E0392]: type parameter `T` is never used
 --> src/main.rs:1:12
  |
1 | struct Tag<T> {
  |            ^ unused type parameter
  |
  = help: consider removing `T` or using a marker such as `PhantomData`
```

编译器拒绝「声明了却不用」的类型参数，因为它无法判断你想表达哪一种语义。加一个 `PhantomData<T>` 就是在回答这个问题。下面四种写法对应四种回答。

### 用途一：标记所有权

```rust,noplayground
use std::marker::PhantomData;

struct Owned<T> {
    ptr: *mut T,
    _owns: PhantomData<T>,
}
```

`PhantomData<T>` 在这里的含义是「`Owned<T>` 拥有一个 `T`」。它带来两个后果：第一，`Owned<T>` 的 `Send`/`Sync` 跟随 `T`——`T` 不能跨线程，`Owned<T>` 也不能；第二，析构检查认为 `T` 可能随 `Owned<T>` 一起被销毁，于是借用检查器会阻止「先销毁 `Owned<T>`、再使用借来的 `T`」这种顺序。手写 `Box<T>`、容器、FFI 的所有权句柄时，这个标记是必须的；只写 `*mut T` 而不写 `PhantomData<T>`，编译器会认为这个类型不拥有任何东西，析构顺序就失去保护。

### 用途二：标记生命周期

```rust,noplayground
use std::marker::PhantomData;

struct Borrowed<'a, T: ?Sized> {
    ptr: *const T,
    _borrows: PhantomData<&'a T>,
}
```

裸指针不带生命周期，`Borrowed` 想表达「我借用了某个存活至少 `'a` 的值」，就必须把 `'a` 挂到类型上，而 `'a` 本身不能作为字段，于是用 `PhantomData<&'a T>`。这个标记的型变是协变的：`Borrowed<'static, T>` 可以当 `Borrowed<'a, T>` 用。零拷贝解析器、切片迭代器、借用视图都用这个套路。`T: ?Sized` 是为了让 `T = str` 或 `[u8]` 也能用。

### 用途三：标记可变性

```rust,noplayground
use std::cell::Cell;
use std::marker::PhantomData;

struct Invariant<T> {
    ptr: *mut T,
    _invariant: PhantomData<Cell<T>>,
}
```

默认情况下，泛型参数是协变的，`Invariant<&'static str>` 能悄悄变成 `Invariant<&'a str>`。如果这个类型内部允许通过共享引用写入（内部可变性），协变就是灾难：把一个短生命期的引用写进被当作长生命期使用的容器，读出来就是悬垂引用。`Cell<T>` 对 `T` 不变（invariant），把 `PhantomData<Cell<T>>` 放进去，整个类型就继承了这个不变性。`Cell` 为什么不变，36.7 会从读写两个方向解释。

### 用途四：声明 Send 与 Sync 的观点

```rust
use std::marker::PhantomData;
use std::rc::Rc;

struct NotSend<T> {
    id: u64,
    _not_send: PhantomData<*const T>,
}

struct AlwaysSend<T> {
    id: u64,
    _always: PhantomData<fn() -> T>,
}

fn assert_send<T: Send>() {}

fn main() {
    assert_send::<AlwaysSend<Rc<u8>>>();
    println!("ok");
}
```

`*const T` 既不是 `Send` 也不是 `Sync`，所以 `NotSend<T>` 永远不能跨线程，哪怕它的字段全是 `u64`。这正是 FFI 句柄需要的：一个来自 C 库的指针只能在创建它的线程上用。反过来，`fn() -> T` 是函数指针，函数指针总是 `Send + Sync`，而且对 `T` 协变，于是 `AlwaysSend<T>` 无论 `T` 是什么都能跨线程——只要你不真的在里面存 `T`。这段程序的 `assert_send::<AlwaysSend<Rc<u8>>>()` 能通过，`assert_send::<NotSend<Rc<u8>>>()` 则会被拒绝。

四种用途可以叠在一个类型上：`PhantomData<(T, &'a U, Cell<V>)>` 这样的元组同时声明所有权、生命周期与不变性。但叠得越多，读代码的人越难还原你的意图，通常拆成两个标记字段更清楚。

还有两个容易踩的坑。第一，`PhantomData<T>` 会让类型继承 `T` 的自动 trait，如果你的 `T` 恰好是 `Rc<u8>`，那这个类型就意外地不能跨线程了；把标记换成 `PhantomData<fn() -> T>` 可以只要协变、不要 `T` 的 `Send` 约束。第二，`PhantomData` 只影响类型层面的推导，不影响运行期布局：它是零大小类型，加上它不会让结构体变大，也不会多一次内存访问。36.1 里 `Request<Draft>` 与 `Request<Sent>` 大小相同，原因就在这里。

## 36.3 常量泛型：把尺寸写进类型

`[T; N]` 是最早的常量泛型。在 Rust 1.51 之前，数组长度无法泛化，写一个处理任意长度数组的函数只能靠宏或切片；现在长度是类型的一部分，`[u8; 3]` 和 `[u8; 4]` 是两个不同的类型，编译器知道循环要转几圈。把这一点用在自定义类型上，就得到「尺寸即类型」的设计。

固定尺寸矩阵是最直观的例子。行数 `R` 和列数 `C` 作为常量参数出现在类型里，乘法只在 `K` 匹配时才有定义：

```rust
use std::ops::Mul;

#[derive(Debug, Clone, Copy, PartialEq)]
struct Matrix<const R: usize, const C: usize> {
    data: [[f64; C]; R],
}

impl<const R: usize, const C: usize> Matrix<R, C> {
    fn new(data: [[f64; C]; R]) -> Self {
        Matrix { data }
    }

    fn get(&self, r: usize, c: usize) -> f64 {
        self.data[r][c]
    }
}

impl<const R: usize, const K: usize, const C: usize> Mul<Matrix<K, C>> for Matrix<R, K> {
    type Output = Matrix<R, C>;

    fn mul(self, rhs: Matrix<K, C>) -> Matrix<R, C> {
        let mut out = [[0.0; C]; R];
        for r in 0..R {
            for c in 0..C {
                let mut sum = 0.0;
                for k in 0..K {
                    sum += self.data[r][k] * rhs.data[k][c];
                }
                out[r][c] = sum;
            }
        }
        Matrix { data: out }
    }
}

fn main() {
    let a = Matrix::new([[1.0, 2.0, 3.0], [4.0, 5.0, 6.0]]);
    let b = Matrix::new([[7.0, 8.0], [9.0, 10.0], [11.0, 12.0]]);
    let c = a * b;
    println!("{:?}", c.data);
    println!("{:?}", c.get(1, 1));
    println!("{}", std::mem::size_of::<Matrix<2, 3>>());
}
```

输出是 `[[58.0, 64.0], [139.0, 154.0]]`、`154.0` 和 `48`。最值得注意的是第三行：`Matrix<2, 3>` 占 6 个 `f64`，共 48 字节，维度完全编码在类型里，运行期不带任何长度字段。

乘法实现里的 `K` 是「共享维度」。`Matrix<R, K>` 乘 `Matrix<K, C>` 得到 `Matrix<R, C>`，三个参数在签名里出现了两次，编译器据此约束维度必须首尾相接。把 `a` 与另一个 `Matrix<2, 3>` 相乘会得到类型错误，因为找不到 `Mul<Matrix<2, 3>>` 的实现——维度错误的矩阵乘法根本写不出来，不需要运行期断言。

第二个例子是固定容量的环形缓冲。容量 `N` 是类型参数，缓冲区是 `[Option<T>; N]`，压满之后新值覆盖最旧的值：

```rust
struct Ring<T, const N: usize> {
    buf: [Option<T>; N],
    next: usize,
}

impl<T, const N: usize> Ring<T, N> {
    fn new() -> Self {
        Ring { buf: std::array::from_fn(|_| None), next: 0 }
    }

    fn push(&mut self, value: T) {
        self.buf[self.next] = Some(value);
        self.next = (self.next + 1) % N;
    }

    fn iter(&self) -> impl Iterator<Item = &T> {
        let n = self.next;
        self.buf[n..].iter().chain(self.buf[..n].iter()).filter_map(|slot| slot.as_ref())
    }
}

fn main() {
    let mut ring = Ring::<i32, 3>::new();
    for i in 1..=5 {
        ring.push(i);
    }
    let values: Vec<i32> = ring.iter().copied().collect();
    println!("{:?}", values);
}
```

压入 1 到 5 之后，环里是最近三个值，输出 `[3, 4, 5]`。`std::array::from_fn` 是 `[T; N]` 的通用初始化入口：`T` 没有实现 `Copy` 时不能用 `[None; N]` 字面量，因为那要求元素可复制，`from_fn` 逐个调用闭包绕开了这个限制。`iter` 里的 `chain` 把环形在逻辑上「拉直」：从 `next` 处切一刀，先读后半段，再读前半段。

常量参数也可以有默认值，`struct Buf<const N: usize = 16>` 合法，于是 `Buf` 与 `Buf<32>` 都能用。不过常量泛型目前有一条硬限制：常量参数只能作为整体使用，不能参与类型层面的算术。下面这种签名无法通过编译：

```rust,noplayground
fn double<const N: usize>(xs: [u8; N]) -> [u8; N * 2] {
    let mut out = [0_u8; N * 2];
    for i in 0..N {
        out[i] = xs[i] * 2;
    }
    out
}
```

```text
error: generic parameters may not be used in const operations
  = help: const parameters may only be used as standalone arguments, i.e. `N`
```

`[u8; N * 2]`、`[u8; N + 1]` 这类「编译期表达式」由 nightly 的 `generic_const_exprs` 提供，稳定版只能绕开：要么把目标长度也声明成一个常量参数 `<const M: usize>`，由调用方保证 `M == N * 2`；要么返回 `Vec<u8>`，把长度交给运行期。这也解释了为什么 `Matrix` 的乘法要把 `K` 显式写出来，而不是从 `R`、`C` 推导。

最后一个使用要点：每个具体的常量参数组合都会单态化出一份代码。`Matrix<2, 3>` 与 `Matrix<3, 3>` 是两个类型、两份机器码。维度组合一多时会有代码膨胀，这和泛型参数的单态化是同一回事。如果维度必须在运行期才知道，常量泛型就不适用，应该用切片 `&[f64]` 加运行期检查。

## 36.4 关联类型还是泛型参数

第 16 章说明了 `Iterator` 为什么用关联类型 `Item`。这里换一个角度：当你设计自己的 trait 时，怎么知道该用 `type Output` 还是 `<T>`？判据只有一条——**同一个类型对同一个 trait 需不需要实现多次**。

先看泛型参数版本。它给「输出类型」开了一个参数位，同一个 `Point` 可以对多种输出各实现一次：

```rust
trait EncodeAs<T> {
    fn encode_as(&self) -> T;
}

#[derive(Debug, Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

impl EncodeAs<String> for Point {
    fn encode_as(&self) -> String {
        format!("({}, {})", self.x, self.y)
    }
}

impl EncodeAs<Vec<i32>> for Point {
    fn encode_as(&self) -> Vec<i32> {
        vec![self.x, self.y]
    }
}

fn main() {
    let p = Point { x: 1, y: 2 };
    let s: String = p.encode_as();
    let v: Vec<i32> = p.encode_as();
    println!("{} {:?}", s, v);
}
```

`Point` 同时是 `EncodeAs<String>` 和 `EncodeAs<Vec<i32>>`，两种编码共存。代价是调用处必须知道返回类型，否则编译器无法在多个实现之间选择：

```text
error[E0283]: type annotations needed for `Point`
  = note: cannot satisfy `Point: EncodeAs<_>`
  = note: multiple `impl`s found: `EncodeAs<Vec<i32>>`, `EncodeAs<String>`
```

`let s: String = p.encode_as();` 里的 `: String` 不是可选的装饰，它是消解歧义的唯一信息。

再看关联类型版本。输出被声明为 `Self` 的一个组成部分，对每个 `Self` 只能有一个取值：

```rust
trait Encode {
    type Output;

    fn encode(&self) -> Self::Output;
}

#[derive(Debug, Clone, Copy)]
struct Point {
    x: i32,
    y: i32,
}

impl Encode for Point {
    type Output = String;

    fn encode(&self) -> String {
        format!("({}, {})", self.x, self.y)
    }
}

fn main() {
    let p = Point { x: 1, y: 2 };
    println!("{}", p.encode());
}
```

调用 `p.encode()` 不需要任何标注：`Point: Encode` 只有一个实现，`Output` 必然是 `String`。想再给 `Point` 加一个 `Encode<Vec<i32>>` 是不可能的，因为 `Output` 由 `Self = Point` 唯一决定；硬写第二遍会得到 E0119。

两种写法的差别可以归结为三点。第一，多重实现：泛型参数允许，关联类型不允许。第二，调用处的类型推断：关联类型版本通常无需标注，泛型参数版本在有多个实现时需要标注。第三，trait 作为约束的写法：`T: EncodeAs<String>` 明确要求某一种编码，`T: EncodeAs<_>` 则允许任意编码，表达力更宽，但约束也更模糊。

标准库里的选择几乎都能用这条判据解释。`Iterator` 用关联类型 `Item`，因为一个迭代器只有一种元素类型。`From<T>` 用泛型参数，因为 `String` 可以从 `&str`、`char`、`Vec<u8>` 等多种来源构造。`Add<Rhs>` 两者都用：`Rhs` 是泛型参数（`i32` 可以加 `i32`，也可以为它实现加 `Duration`），`Output` 是关联类型（同一次加法只有一种结果类型）。一个 trait 同时出现泛型参数和关联类型，往往说明：参数是「输入的多种选择」，关联类型是「每个选择的唯一结果」。

实践中的建议是先从关联类型开始，因为它的错误信息更直接、调用更省心。只有当确实需要一个类型实现多次，或者要让调用方来决定输出类型时，才把参数提到 trait 上。把参数提到 trait 上还有一个副作用：它会被 trait 约束和 impl 的匹配逻辑看见，进而牵扯到一致性（coherence）——36.8 会展开。

## 36.5 trait 别名与 sealed trait

这两个话题都关于「控制 trait 的边界」：一个想把一组约束打包成一个名字，一个想公开 trait 却不允许外部实现。

### trait 别名：语法已定，稳定版还没有

理想中的写法是 `trait Show = Display + Debug;`，之后所有需要「既能显示又能调试」的地方只写 `T: Show`。这段语法目前仍在 nightly 后面：加 `#![feature(trait_alias)]` 才可用，稳定版会报 `error[E0658]: trait aliases are experimental`（跟踪 issue #41517）。

稳定版能做的等价物是「超 trait + 全覆盖实现」：

```rust
use std::fmt::{Debug, Display};

trait Show: Display + Debug {}

impl<T: Display + Debug> Show for T {}

fn render(value: &impl Show) -> String {
    format!("{} | {:?}", value, value)
}

fn main() {
    println!("{}", render(&42));
}
```

三个要点。第一，`Show` 没有方法，只是一个约束的集合，所以 `impl Show` 位置可以正常当约束用。第二，`impl<T: Display + Debug> Show for T {}` 是一条 blanket impl，它把「所有满足条件的类型」一次性实现掉；这也意味着下游无法再给任何类型单独实现 `Show`，因为会与这条实现冲突。第三，这个方案产生了一个真实的 trait，而 trait 别名不会——别名只是约束的简写，不参与 impl，也不会出现在 `dyn` 里。

`&dyn Show` 可以拼出来，但它没有任何方法可调；想用它格式化，得先向上转型成 `dyn Display`（trait upcasting 自 1.86 起稳定）。多数时候不值得，直接写 `&(impl Display + Debug)` 更清楚。

### sealed trait：公开能力，保留实现权

库作者经常需要「让用户能调用 trait 的方法，但不允许用户为自己的类型实现这个 trait」。理由通常是演进：如果外部有实现，给 trait 加一个方法就是破坏性变更；封住实现，加方法就安全。

做法是把「实现许可」放进一个外部无法命名的模块：

```rust
mod sealed {
    pub trait Sealed {}
}

pub trait Unit: sealed::Sealed {
    fn to_base(&self) -> f64;
}

pub struct Meters(pub f64);

impl sealed::Sealed for Meters {}

impl Unit for Meters {
    fn to_base(&self) -> f64 {
        self.0
    }
}

fn main() {
    println!("{}", Meters(3.0).to_base());
}
```

外部 crate 想写 `impl Unit for Fahrenheit` 时，编译器会先要求 `Fahrenheit: Sealed`，而 `sealed` 模块是私有的，外部根本写不出 `impl sealed::Sealed for ...`，于是得到 E0277：`the trait bound Fahrenheit: Sealed is not satisfied`。注意 `Sealed` 本身必须声明为 `pub`，否则 `Unit` 的超 trait 列表会触发「私有 trait 出现在公开接口」的检查；真正起作用的是外层模块不可见。

这个模式有几点使用上的讲究。`sealed` 模块不要导出，也不要写成 `pub mod`。如果想让文档干净，可以加 `#[doc(hidden)]`。封住实现不等于封住调用：外部依然可以调用 `to_base`，只是不能新增实现。若 trait 带有必须由实现者提供的关联类型，封住之后外部连「构造一个实现」都做不到，效果更彻底。

## 36.6 impl Trait 的三个位置

`impl Trait` 出现在参数位置、返回位置、trait 方法返回位置时，语义并不相同。混淆这三者，是很多「为什么这里不能写 dyn」问题的根源。

### 参数位置：匿名泛型

`fn print_all(items: impl IntoIterator<Item = i32>)` 是泛型函数的简写，等价于 `fn print_all<I: IntoIterator<Item = i32>>(items: I)`。它按单态化处理：每个具体类型生成一份代码，没有虚调用。参数位置的 `impl Trait` 有一个限制：每个出现都是独立的泛型参数，无法表达「两个参数是同一个类型」。`fn zip(a: impl Iterator, b: impl Iterator)` 里的两个 `impl` 是两种可能不同的类型；要约束它们相同，只能退回显式泛型参数 `fn zip<I: Iterator>(a: I, b: I)`。另外它不能被 turbofish 指定，`print_all::<Vec<i32>>(...)` 写不出来。

### 返回位置：隐藏具体类型

`fn doubled(values: &[i32]) -> impl Iterator<Item = i32> + '_` 返回一个「实现了 Iterator 的某种类型」，调用方知道能力，不知道具体类型。这也叫 RPIT（return-position impl Trait）。

```rust
fn doubled(values: &[i32]) -> impl Iterator<Item = i32> + '_ {
    values.iter().map(|v| v * 2)
}

fn main() {
    let values = vec![1, 2, 3];
    let doubled: Vec<i32> = doubled(&values).collect();
    println!("{:?}", doubled);
}
```

`+ '_` 不能省。返回类型里含有一个借用 `&[i32]`，但 `impl Iterator` 的约束列表里没有任何生命周期，编译器默认不把输入的生命周期捕获进返回值。省略后，它按 `'static` 处理，而 `values` 是局部变量，于是报「lifetime may not live long enough」，帮助信息会建议加上 `+ '_`。这类捕获规则在 2024 edition 有变化：RPIT 会自动捕获作用域内所有生命周期；2021 及更早则要求显式写出。写库时显式加 `+ '_` 最稳妥。

如果隐藏的类型确实捕获了某个具名生命周期，却没有出现在约束里，编译器会给出 E0700：

```text
error[E0700]: hidden type for `impl Sized` captures lifetime that does not appear in bounds
  = help: to declare that the `impl Sized` captures `'a`, you can add an explicit `+ use<'a>` bound
```

`+ use<'a>` 是 2024 edition 引入的精确捕获语法，2021 下也能用。它比 `+ 'a` 更精确：只捕获列出的生命周期与类型参数，而不是整个环境。

返回位置的 `impl Trait` 不是 trait 对象：类型在编译期确定，走静态分派，调用方无法换成别的实现，也无法把两个不同函数的返回值放进同一个变量。需要运行期多态时，仍然要写 `Box<dyn Trait>`。

### trait 方法返回位置：RPITIT

从 1.75 起，trait 的方法也可以返回 `impl Trait`，称为 RPITIT（return-position impl Trait in trait）：

```rust
trait Container {
    fn items(&self) -> impl Iterator<Item = &u32>;
}

struct Fixed(Vec<u32>);

impl Container for Fixed {
    fn items(&self) -> impl Iterator<Item = &u32> {
        self.0.iter()
    }
}

fn total(c: &impl Container) -> u32 {
    c.items().sum()
}

fn main() {
    println!("{}", total(&Fixed(vec![1, 2, 3])));
}
```

RPITIT 让「返回借用数据的迭代器」在 trait 里成为可能，不必写 `Box<dyn Iterator>`，也不必为关联类型起名字。但它有一个直接后果：这样的 trait 不是 dyn 兼容的，`&dyn Container` 会被拒绝（E0038，见案例 5）。原因很直白——虚表需要为每个方法记录一个确定的返回类型，而 `impl Iterator` 的具体类型是实现方各自的秘密，无法放进同一张表。

需要同时保留 trait 对象与「返回迭代器」时，只有两条路：把返回类型改成 `Box<dyn Iterator<Item = &u32> + '_>`（有堆分配），或者把迭代器类型提升为关联类型 `type Items<'a>: Iterator<Item = &'a u32>`（需要 GAT，写法更重）。选哪条取决于调用方是否真的需要 `dyn`。

### `Box<dyn Trait + 'a>`

`Box<dyn Trait>` 默认等价于 `Box<dyn Trait + 'static>`。返回一个借用了局部数据的闭包时，必须把生命周期写出来：

```rust
fn make_greeting(name: &str) -> Box<dyn Fn() -> String + '_> {
    let owned = name.to_string();
    Box::new(move || format!("hi {}", owned))
}

fn main() {
    let greeting = make_greeting("rust");
    println!("{}", greeting());
}
```

漏掉 `+ '_` 会得到「lifetime may not live long enough」，帮助信息建议加 `+ '_` 或 `+ 'static`。反过来，如果闭包确实只捕获 `'static` 数据，不写反而更好：`Box<dyn Fn() -> String>` 可以被存进全局容器，带上 `'_` 就把它绑在了调用栈上。生命周期标注的原则在这里体现得很直接：能短则短，但必须诚实。

## 36.7 型变：协变、逆变与不变

回到一个最基础的问题：为什么 `&'static str` 能当 `&'a str` 用？因为 Rust 里存在子类型关系——`'static` 比任何具体的 `'a` 都长，长生命周期是短生命周期的子类型。型变（variance）描述的是：当子类型关系出现在泛型构造器内部时，它会不会、以及以什么方向传递到外层。

三种可能的行为，用「`'long` 是 `'short` 的子类型」来陈述：

- 协变（covariant）：`F<'long>` 也是 `F<'short>` 的子类型，方向不变
- 逆变（contravariant）：`F<'short>` 是 `F<'long>` 的子类型，方向反转
- 不变（invariant）：两个方向都不成立，`F<'long>` 与 `F<'short>` 只能各用各的

`&'a T` 对 `'a` 协变，这就是开头那个问题的答案：`&'static str` 是 `&'a str` 的子类型，所以前者可以出现在后者的位置。直觉也支持它——一个借用活得比需要的更久，不会制造悬垂；反过来，把短生命周期的引用塞进要求长生命周期的位置，才会出事。

```rust
fn wants_short<'a>(s: &'a str, _local: &'a str) -> usize {
    s.len()
}

fn main() {
    let local = String::from("short");
    let n = wants_short("static literal", &local);
    println!("{}", n);
}
```

调用处 `'a` 被推断为 `local` 的生命周期，字面量是 `&'static str`，由于 `'static` 覆盖 `'a`，协变允许把它传进去。如果 `&'a T` 是不变的，这段代码就写不出来，字符串字面量也没法传给任何借用参数——那会是灾难性的。

函数类型是理解逆变的最好例子。`fn(T)` 对 `T` 逆变，因为参数位置是「消费」：一个能接受更宽输入的消费者，可以替换一个只接受更窄输入的消费者。`fn(T) -> U` 则对 `T` 逆变、对 `U` 协变，也就是那句常被引用的话：输入位置逆变，输出位置协变。

```rust
use std::marker::PhantomData;

struct Consumer<T> {
    _marker: PhantomData<fn(T)>,
}

fn needs_static(_consumer: Consumer<&'static str>) {}

fn main() {
    let local = String::from("short");
    let consumer: Consumer<&str> = Consumer { _marker: PhantomData };
    needs_static(consumer);
    println!("{}", local);
}
```

`Consumer<&'short str>` 能被当作 `Consumer<&'static str>` 使用，因为它在参数位置上消费 `T`。把 `PhantomData<fn(T)>` 换成 `PhantomData<T>`，这段代码立刻失败：后者是协变的，方向正好相反。

`Cell<T>` 是不变的，这个设计不是随手定的。`Cell` 通过共享引用同时提供读和写：`get(&self) -> T` 让 `T` 出现在输出位置，要求协变；`set(&self, T)` 让 `T` 出现在输入位置，要求逆变。两个方向的要求同时存在，唯一的相容选择就是不变。

```rust,noplayground
use std::cell::Cell;

fn overwrite<'a>(cell: &Cell<&'a str>, value: &'a str) {
    cell.set(value);
}

fn main() {
    let local = String::from("short");
    let cell: Cell<&'static str> = Cell::new("static");
    overwrite(&cell, &local);
    println!("{}", cell.get());
}
```

```text
error[E0597]: `local` does not live long enough
  --> src/main.rs:10:22
   |
 8 |     let local = String::from("short");
   |         ----- binding `local` declared here
 9 |     let cell: Cell<&'static str> = Cell::new("static");
   |               ------------------ type annotation requires that `local` is borrowed for `'static`
10 |     overwrite(&cell, &local);
   |                      ^^^^^^ borrowed value does not live long enough
11 |     println!("{}", cell.get());
12 | }
   | - `local` dropped here while still borrowed
```

如果 `Cell<&'static str>` 能协变成 `Cell<&'a str>`，第 10 行就会把一个只活到第 12 行的引用写进本来承诺「永远有效」的格子里，第 11 行读出来就是悬垂引用。如果它逆变成 `Cell<&'static str>`，`get` 又会把短引用当长引用返回，同样不安全。所以它只能不变。

`&mut T` 也是同样的道理：对 `T` 不变。`&mut` 允许通过它读、也允许通过它写，读写并存就要求不变。注意 `&'a mut T` 对 `'a` 仍然是协变的，不变的是被指向的 `T`。这也解释了一个常见的编译错误：不能把 `&mut Vec<&'static str>` 传给接受 `&mut Vec<&'a str>` 的函数，因为 `&mut Vec<_>` 把不变性一路传到了元素类型上。

型变不是手写的标注，而是编译器从字段类型自动推导出来的：结构体对某个参数的型变，等于该参数在所有字段里出现位置的型变组合。`PhantomData` 之所以有用，正是因为它让你可以「伪造」一个字段，从而选择型变——`PhantomData<T>` 协变，`PhantomData<fn(T)>` 逆变，`PhantomData<Cell<T>>` 与 `PhantomData<fn(T) -> T>` 不变。36.2 的四种用途里，用途三就是这一节的应用。

最后要提醒的是：型变错误不能用 `unsafe` 绕过，也不该用 `transmute` 绕过。编译器拒绝的那些赋值之所以被拒绝，是因为放行它们会在某个角落制造悬垂引用；换一个类型、加一层包装、或者接受一个更短的约束，才是正解。

## 36.8 深入：trait 求解、coherence 与 never type

这一节把前七节的机制拉到编译器内部看一眼：trait 约束是怎么被证明的、为什么有些 impl 注定冲突、`!` 这个没有值的类型又是怎么参与推断的。

### trait 求解是一次目标驱动的搜索

每个 `T: Trait` 约束都是一个待证明的目标（goal）。编译器手里的规则是程序里所有 impl：找到 self 类型能匹配的 impl，就把目标拆成该 impl 的前提。整个过程像一次逻辑证明，失败时给出「不满足」，递归太深时给出溢出。

一个自引用的 blanket impl 会让搜索永不终止：

```rust,noplayground
trait Foo {}

impl<T: Foo> Foo for T {}

struct S;

fn need<T: Foo>() {}

fn main() {
    need::<S>();
}
```

```text
error[E0275]: overflow evaluating the requirement `S: Foo`
  --> src/main.rs:10:5
   |
10 |     need::<S>();
   |     ^^^^^^^^^^^
   |
note: required by a bound in `need`
  --> src/main.rs:7:12
   |
 7 | fn need<T: Foo>() {}
   |            ^^^ required by this bound in `need`
```

要证明 `S: Foo`，唯一可用的 impl 要求先证明 `S: Foo`，目标没有缩小，搜索无限展开。注意报错位置在调用处 `need::<S>()`，而不是 impl 定义处：`impl<T: Foo> Foo for T` 本身是合法的，只有真正去求解一个具体类型时才暴露问题。把递归深度调大没有用，环本身才是 bug——修复方式是删掉这条自引用实现，或者把条件改成能收敛的（例如 `impl Foo for S`）。

### coherence：一致性保证与孤儿规则

Rust 承诺：对任意类型和 trait，最多只有一个 impl 在起作用。这就是一致性（coherence），它保证无论调用代码写在哪里，方法解析的结果都一样。为了在全球范围内维持这个承诺，编译器必须在本地就能判断两个 impl 是否会重叠。E0119 就是这么来的：`impl<T: Display> Render for T` 和 `impl<T: Debug> Render for T` 在「同时实现 Display 与 Debug 的类型」上重叠，即使你举不出具体反例，编译器也不允许。

只靠一致性还不够，还必须有孤儿规则（orphan rule）来限制 impl 的书写位置，否则下游 crate 可以给上游类型加实现，冲突只有链接时才可能发现。孤儿规则要求 impl 满足「本地类型出现在 trait 与类型参数之前」；最直接的违法写法是给外部类型实现外部 trait：

```rust,noplayground
use std::fmt;

struct Local;

impl fmt::Display for Vec<Local> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.len())
    }
}

fn main() {
    println!("{}", Vec::<Local>::new());
}
```

```text
error[E0117]: only traits defined in the current crate can be implemented for types defined outside of the crate
 --> src/main.rs:5:1
  |
5 | impl fmt::Display for Vec<Local> {
  | ^^^^^^^^^^^^^^^^^^^^^^----------
  |                       |
  |                       `Vec` is not defined in the current crate
  |
  = note: impl doesn't have any local type before any uncovered type parameters
  = note: define and implement a trait or new type instead
```

`Vec<Local>` 里出现了本地类型 `Local`，看起来够本地了，但孤儿规则看的是「在 `Vec` 这一层之前有没有本地类型」。`Vec` 是外部的，而它包着的 `Local` 出现在一个「未覆盖的类型参数」之后，于是被拒。绕开它的标准做法是 newtype：`struct LocalVec(Vec<Local>);`，然后为 `LocalVec` 实现任何 trait，因为整个类型都是本地的。

有一个例外值得知道：`&` 和 `Box` 被标记为基本类型（`#[fundamental]`），在孤儿规则面前是透明的。`&` 不算「外部遮盖」，所以 `impl Display for &Wrapper` 是合法的，只要 `Wrapper` 是本地类型：

```rust
use std::fmt;

struct Wrapper(i32);

impl fmt::Display for &Wrapper {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "wrapper({})", self.0)
    }
}

fn main() {
    let w = Wrapper(3);
    println!("{}", &w);
}
```

输出 `wrapper(3)`。把 `&` 换成 `Vec`，同样位置就会报 E0117；这就是 `#[fundamental]` 与不透明类型包装的区别。

blanket impl 还有一个容易被忽视的后果：`impl<T: Display> Render for T` 不只覆盖你现在的类型，还堵住了下游为它们的类型实现 `Render` 的路——下游写 `impl Render for MyType` 会因为可能与 blanket 重叠而得到 E0119。发布库时如果只想给自己的类型用，就不要开 blanket；确实要开，就用 sealed trait（36.5）把实现权收住。

### never type 与 fallback

`!` 是底类型（bottom type）：它没有任何值，因此可以强制转换成任何类型。`panic!()`、`loop {}`、`return`、`std::process::exit` 的返回类型都是 `!`，这正是「不会正常返回」的代码能出现在任意表达式位置的原因。在当前工具链上，`!` 也能直接写在类型位置：`fn _take(_x: !) {}` 与 `Vec<!>` 都能通过编译，说明 `!` 已经是一个可命名的类型，而不是只能出现在函数返回位置的记号。

fallback 的问题出在推断：`panic!()` 给出 `!`，`!` 能变成任何类型，如果上下文不约束它，编译器就不知道该选哪个。早期版本会把它「回退」成 `()`，于是下面这段代码在某些版本上只是警告：

```rust,noplayground
trait Tag {
    fn tag(&self) -> &'static str;
}

impl Tag for () {
    fn tag(&self) -> &'static str {
        "unit"
    }
}

impl Tag for i32 {
    fn tag(&self) -> &'static str {
        "i32"
    }
}

fn main() {
    let x = panic!();
    println!("{}", x.tag());
}
```

在当前工具链上它已经是错误，同时附带一条未来不兼容警告：

```text
warning: method call on a diverging inference variable
  --> src/main.rs:19:22
   |
19 |     println!("{}", x.tag());
   |                      ^^^
   |
   = help: consider providing a type annotation
   = warning: this was previously accepted by the compiler but is being phased out; it will become a hard error in a future release!
   = note: for more information, see issue #156047 <https://github.com/rust-lang/rust/issues/156047>

error[E0599]: no method named `tag` found for type `!` in the current scope
  --> src/main.rs:19:22
   |
19 |     println!("{}", x.tag());
   |                      ^^^ method not found in `!`
```

这里 `x` 的类型没有回退成 `()`，而是保持 `!`，于是 `!` 上没有 `tag` 方法，直接报 E0599。教训很直接：不要对发散表达式的值做推断依赖的操作。真要用这个值，就显式写清类型——`let x: () = panic!();`——让 `!` 向 `()` 强制转换，行为立刻确定。跟踪 issue 是 #156047，这条兼容性收紧还在推进中。

## 典型错误案例

下面五个案例都来自本机 rustc 1.100.0 的实际输出，顺序按「从类型本身的问题，到 trait 与一致性，再到对象安全」排列。

### 案例 1：把草稿当成已发送——E0308

```rust,editable
use std::marker::PhantomData;

struct Draft;
struct Sent;

struct Request<S> {
    body: String,
    _state: PhantomData<S>,
}

impl Request<Draft> {
    fn new(body: &str) -> Self {
        Request { body: body.to_string(), _state: PhantomData }
    }
    fn send(self) -> Request<Sent> {
        Request { body: self.body, _state: PhantomData }
    }
}

fn log_sent(req: &Request<Sent>) {
    println!("{}", req.body);
}

fn main() {
    let draft = Request::new("hello");
    log_sent(&draft);
}
```

```text
error[E0308]: mismatched types
  --> src/main.rs:26:14
   |
26 |     log_sent(&draft);
   |     -------- ^^^^^^ expected `&Request<Sent>`, found `&Request<Draft>`
   |     |
   |     arguments to this function are incorrect
   |
   = note: expected reference `&Request<Sent>`
              found reference `&Request<Draft>`
note: function defined here
  --> src/main.rs:20:4
   |
20 | fn log_sent(req: &Request<Sent>) {
   |    ^^^^^^^^ -------------------
```

**原因**：`Request<Draft>` 与 `Request<Sent>` 是两个毫不相关的类型，`send` 的作用是消费旧值、返回新类型的值，它不会「原地修改」。这里跳过了 `send`，直接拿草稿去调用只接受已发送请求的函数，编译器只能判定类型不匹配。报错落在实参上而不是方法调用上，也是这个原因：`send` 本身没有问题，问题是没有调用它。

**修复**：先完成状态迁移，拿到 `Request<Sent>` 再传：

```rust,editable
use std::marker::PhantomData;

struct Draft;
struct Sent;

struct Request<S> {
    body: String,
    _state: PhantomData<S>,
}

impl Request<Draft> {
    fn new(body: &str) -> Self {
        Request { body: body.to_string(), _state: PhantomData }
    }
    fn send(self) -> Request<Sent> {
        Request { body: self.body, _state: PhantomData }
    }
}

fn log_sent(req: &Request<Sent>) {
    println!("{}", req.body);
}

fn main() {
    let draft = Request::new("hello");
    let sent = draft.send();
    log_sent(&sent);
}
```

### 案例 2：`PhantomData` 把类型标成了不可发送——E0277

```rust,editable
use std::marker::PhantomData;

struct Handle<T> {
    id: u64,
    _not_send: PhantomData<*const T>,
}

impl<T> Handle<T> {
    fn new(id: u64) -> Self {
        Handle { id, _not_send: PhantomData }
    }
    fn show(&self) -> u64 {
        self.id
    }
}

fn main() {
    let h = Handle::<u8>::new(1);
    std::thread::spawn(move || {
        println!("{}", h.show());
    })
    .join()
    .unwrap();
}
```

```text
error[E0277]: `*const u8` cannot be sent between threads safely
   --> src/main.rs:19:24
    |
 19 |       std::thread::spawn(move || {
    |       ------------------ ^------
    |       |                  |
    |  _____|__________________within this `{closure@src/main.rs:19:24: 19:31}`
    | |     |
    | |     required by a bound introduced by this call
 20 | |         println!("{}", h.show());
 21 | |     })
    | |_____^ `*const u8` cannot be sent between threads safely
    |
    = help: within `{closure@src/main.rs:19:24: 19:31}`, the trait `Send` is not implemented for `*const u8`
note: required because it appears within the type `PhantomData<*const u8>`
note: required because it appears within the type `Handle<u8>`
   --> src/main.rs:5:5
    |
  5 |     _not_send: PhantomData<*const T>,
    |     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ required by this field
note: required because `h` is used within this closure
```

**原因**：`Handle<T>` 里没有裸指针字段，但 `PhantomData<*const T>` 让它继承了裸指针的自动 trait：既不 `Send` 也不 `Sync`。`thread::spawn` 要求闭包 `Send`，闭包又捕获了 `h`，于是整条链被拒绝。注意闭包体是 `h.show()`——方法调用需要整个 `h`，编译器因此捕获整个 `h`；如果闭包写成 `println!("{}", h.id)`，2021 版的不相交字段捕获只会捕获 `id: u64`，这段代码会静默通过。同一个类型，换个写法就绕过了标记，这是自查时容易漏掉的地方。

**修复**：想清楚标记是「有意为之」还是「误用」。如果句柄必须留在原线程，就不要把它移进 `spawn`；如果确实要共享，就用真正的所有权容器，并把访问限制在锁内：

```rust,editable
use std::sync::{Arc, Mutex};

#[derive(Debug)]
struct Handle {
    id: u64,
}

fn main() {
    let handle = Arc::new(Mutex::new(Handle { id: 1 }));
    let worker = Arc::clone(&handle);
    std::thread::spawn(move || {
        println!("{}", worker.lock().unwrap().id);
    })
    .join()
    .unwrap();
    println!("{}", handle.lock().unwrap().id);
}
```

### 案例 3：impl 里的类型参数没人约束——E0207

```rust,editable
trait Described {
    fn describe() -> &'static str;
}

struct Config;

impl<T> Described for Config {
    fn describe() -> &'static str {
        "config"
    }
}

fn main() {
    println!("{}", Config::describe());
}
```

```text
error[E0207]: the type parameter `T` is not constrained by the impl trait, self type, or predicates
 --> src/main.rs:7:6
  |
7 | impl<T> Described for Config {
  |     -^-
  |     ||
  |     |unconstrained type parameter
  |     help: remove the unused type parameter `T`

error[E0282]: type annotations needed
  --> src/main.rs:14:20
   |
14 |     println!("{}", Config::describe());
   |                    ^^^^^^ cannot infer type for type parameter `T`
```

**原因**：`T` 只出现在 `impl` 的头部，既没有被 trait 使用，也没有出现在 self 类型 `Config` 或任何 where 谓词里。这样的 impl 会同时覆盖无限多个 `T`，但每个 `T` 的实现内容又完全相同，编译器无法为调用点选定一个。它先拒绝 impl 本身，再在调用处连带报 E0282：`Config::describe()` 里的 `T` 无从推断。修 typestate 或泛型代码时，这种残留参数很常见——把状态参数从结构体上删掉了，却忘了删 impl 上的。

**修复**：如果 `T` 没有语义，直接删掉：

```rust,editable
trait Described {
    fn describe() -> &'static str;
}

struct Config;

impl Described for Config {
    fn describe() -> &'static str {
        "config"
    }
}

fn main() {
    println!("{}", Config::describe());
}
```

如果 `T` 确实要区分行为，就把它放到能被约束的位置：写进 trait（`trait Described<T>`）、写进 self 类型（`impl<T> Described for Config<T>`），或者用 `PhantomData` 让它成为类型的一部分（见 36.2）。

### 案例 4：两条 blanket impl 撞车——E0119

```rust,editable
use std::fmt::{Debug, Display};

trait Render {
    fn render(&self) -> String;
}

impl<T: Display> Render for T {
    fn render(&self) -> String {
        format!("{}", self)
    }
}

impl<T: Debug> Render for T {
    fn render(&self) -> String {
        format!("{:?}", self)
    }
}

fn main() {
    println!("{}", 1_u32.render());
}
```

```text
error[E0119]: conflicting implementations of trait `Render`
  --> src/main.rs:13:1
   |
 7 | impl<T: Display> Render for T {
   | ----------------------------- first implementation here
...
13 | impl<T: Debug> Render for T {
   | ^^^^^^^^^^^^^^^^^^^^^^^^^^^ conflicting implementation
```

**原因**：两条实现的 self 类型都是裸 `T`，只要一个类型同时实现 `Display` 与 `Debug`，两个 impl 就都适用，违反「每个 (trait, 类型) 至多一个实现」的一致性承诺。编译器不做否定推理：它不会因为「目前恰好没有这种类型」而放行，因为在别的 crate 里随时可能冒出一个同时实现两者的类型。冲突在 impl 定义处就报，而不是等到调用。

**修复**：只保留一条 blanket，把定制能力让给别的机制：

```rust,editable
use std::fmt::Display;

trait Render {
    fn render(&self) -> String;
}

impl<T: Display> Render for T {
    fn render(&self) -> String {
        format!("{}", self)
    }
}

fn main() {
    println!("{}", 1_u32.render());
}
```

如果两种渲染都必须存在，就拆成两个 trait（`RenderDisplay` 与 `RenderDebug`），或者保留一个 trait、把「用哪种格式」变成带约束的方法参数。需要按类型特化时，nightly 的 specialization 是另一条路，但它至今没有稳定。

### 案例 5：`impl Trait` 返回让 trait 失去 dyn 兼容——E0038

```rust,editable
trait Numbers {
    fn numbers(&self) -> impl Iterator<Item = u32>;
}

struct Fixed(Vec<u32>);

impl Numbers for Fixed {
    fn numbers(&self) -> impl Iterator<Item = u32> {
        self.0.iter().copied()
    }
}

fn total(source: &dyn Numbers) -> u32 {
    source.numbers().sum()
}

fn main() {
    println!("{}", total(&Fixed(vec![1, 2, 3])));
}
```

```text
error[E0038]: the trait `Numbers` is not dyn compatible
  --> src/main.rs:13:19
   |
13 | fn total(source: &dyn Numbers) -> u32 {
   |                   ^^^^^^^^^^^ `Numbers` is not dyn compatible
   |
note: for a trait to be dyn compatible it needs to allow building a vtable
  --> src/main.rs:2:26
   |
 1 | trait Numbers {
   |       ------- this trait is not dyn compatible...
 2 |     fn numbers(&self) -> impl Iterator<Item = u32>;
   |                          ^^^^^^^^^^^^^^^^^^^^^^^^^ ...because method `numbers` references an `impl Trait` type in its return type
```

**原因**：RPITIT 的返回类型由每个实现各自决定，`Fixed` 返回的是 `Copied<Iter<'_, u32>>`，另一个实现可以返回完全不同的类型。虚表需要为方法记录一个确定的返回类型和大小，多个实现返回不同类型时这张表就建不起来，于是 trait 不再 dyn 兼容。这条限制和 36.6 的结论互为表里：能写 `impl Trait` 返回的 trait，通常就不能当 `dyn` 用。

**修复**：如果调用方只需要静态分派，把 `&dyn Numbers` 改成 `&impl Numbers`：

```rust,editable
trait Numbers {
    fn numbers(&self) -> impl Iterator<Item = u32>;
}

struct Fixed(Vec<u32>);

impl Numbers for Fixed {
    fn numbers(&self) -> impl Iterator<Item = u32> {
        self.0.iter().copied()
    }
}

fn total(source: &impl Numbers) -> u32 {
    source.numbers().sum()
}

fn main() {
    println!("{}", total(&Fixed(vec![1, 2, 3])));
}
```

如果必须保留 `dyn`，就把返回类型擦成 `Box<dyn Iterator<Item = u32> + '_>`，用一次堆分配换运行期多态。还有一种写法是把返回类型提升成关联类型（GAT），适合同时想要零分配和对象安全之外的抽象的场景，代价是实现方要显式命名迭代器类型。

## 动手练习

1. **加一个失败态**：在 36.1 的 `Request` 上增加 `Failed`，要求只有 `Request<Sent>` 能迁移到 `Failed`，且 `Request<Failed>` 没有 `deliver` 方法。写出全部 `impl` 块，并说明为什么用 `failed: bool` 字段达不到同样的保证。
2. **给注册表挑标记**：设计 `struct Registry<'a, T> { items: Vec<&'a T> }`，要求它对 `T` 协变，并且当 `T` 不是 `Sync` 时 `Registry` 也不是 `Sync`。指出该用哪一种 `PhantomData`；如果把标记换成 `PhantomData<fn() -> T>`，型变与自动 trait 各会怎么变。
3. **矩阵补全**：为 36.3 的 `Matrix<R, C>` 实现 `transpose(self) -> Matrix<C, R>`，并写一个返回 `Matrix<N, N>` 的单位矩阵构造函数。解释为什么 `transpose` 的返回类型必须交换两个常量参数，而不是保持原样。
4. **型变判断**：先判断下面三行能否编译，再用 `rustc` 验证并解释。(a) `let f: fn(&'static str) = takes_any;`，其中 `takes_any` 是 `fn(&str)`；(b) `let cell: Cell<&'static str> = Cell::new(&local);`，`local` 是局部 `String`；(c) `let s: &str = "literal";` 之后把 `s` 传给期望 `&'static str` 的函数。

## 小结

- typestate 把状态编码成类型参数，用「消费 `self`、返回新类型」的迁移函数保证顺序，代价是每个状态一个 `impl` 块、错误信息变长。
- 状态字段必须私有，否则同模块代码可以绕过构造函数直接拼出任意状态，类型证明失效。
- `PhantomData` 的四种用途是标记所有权、生命周期、可变性与 `Send`/`Sync` 观点；它是零大小类型，不改变布局。
- 常量泛型让尺寸成为类型的一部分，维度错误在编译期就被拒绝；常量参数不能参与类型层面的算术，`N * 2` 这类表达式仍然需要 nightly。
- 关联类型适合「输出由 Self 唯一决定」，泛型参数适合「同一类型需要多种实现」；判据是同一类型能否多次实现该 trait。
- trait 别名仍未稳定，稳定版的替代品是「超 trait + blanket impl」；sealed trait 用私有模块封住实现权，同时保留调用权。
- `impl Trait` 在参数位置是匿名泛型，在返回位置是隐藏具体类型，在 trait 方法返回位置是 RPITIT；RPITIT 会让 trait 失去 dyn 兼容。
- 型变由字段结构自动推导：`&'a T` 协变，`fn(T)` 逆变，`Cell<T>` 与 `&mut T` 不变；`PhantomData` 是唯一的手动旋钮。
- trait 求解是目标驱动的搜索，自引用的 blanket impl 会导致 E0275 溢出；一致性拒绝一切可能重叠的 impl，孤儿规则则限制 impl 的书写位置。










