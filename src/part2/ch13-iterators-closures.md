# 第 13 章 迭代器与闭包

> **本章目标**
>
> - 给自定义类型实现 `Iterator`，说清 `next` 返回 `Option` 的契约
> - 判断一段迭代器链什么时候真正开始干活，惰性求值在哪一步被触发
> - 按场景选对适配器与消费者，知道 `any`/`find` 在哪一步短路、`fold` 怎么替代大半消费者
> - 判断闭包该实现 `Fn`、`FnMut` 还是 `FnOnce`，会用 `move` 修"借用活不够久"
> - 解释迭代器链为什么能编译成和手写 `for` 循环相同的机器码，`collect` 如何靠 `size_hint` 预分配

一段"遍历并处理"的代码，绝大多数人一开始都是这么写的：

```rust
fn main() {
    let v = vec![10, 20, 30];
    let mut total = 0;
    for i in 0..v.len() {
        total += v[i];
    }
    println!("{}", total);
}
```

它有三处容易出事的地方。`0..v.len()` 是两段独立的计算，将来有人把 `v` 换成别的容器、忘了改写循环条件，就会漏掉或越界；`v[i]` 每次都要过一次边界检查，编译器不一定能证明 `i` 合法；最要命的是，这段循环**没法组合**——想"跳过前两个、只取偶数、再平方求和"，你得在循环体里塞三个 `if`/`continue`，逻辑和遍历纠缠在一起。

Rust 的做法是把这件事拆成两半：**迭代器**负责"怎么走"，**闭包**负责"每步做什么"。拆开之后，"跳过两个再取偶数"就变成一条链：`v.iter().skip(2).filter(|x| *x % 2 == 0)`。每段都能单独读、单独改、单独测试。

代价是两条新的心智负担：链式代码什么时候真正执行？闭包捕获变量之后，那个变量还归谁？本章把这两个问题讲透，顺带让你能自己写一个迭代器。

## 13.1 Iterator：只有一个必选方法的 trait

先不写 `for`，直接用 `next` 走一遍：

```rust
fn main() {
    let v = vec![10, 20, 30];
    let mut it = v.iter();
    println!("{:?} {:?} {:?}", it.next(), it.next(), it.next());
    println!("{:?} {:?}", it.next(), it.next());
}
```

输出 `Some(10) Some(20) Some(30)` 和 `None None`：第三个 `next` 之后迭代器已经走完。每次 `next` 返回一个 `Option`：有下一个元素就是 `Some`，走完了就是 `None`。**`None` 之后继续调 `next` 仍然返回 `None`**，这是标准库所有迭代器共同遵守的约定，虽然编译器并不检查它——你手写迭代器时也得守。

`Iterator` 的定义只有两个必填项：

```rust,noplayground
pub trait Iterator {
    type Item;
    fn next(&mut self) -> Option<Self::Item>;
    // 其余几十个方法都有默认实现
}
```

`Item` 是**关联类型**（associated type），不是泛型参数：一个类型只能有一种"产出元素"。`Vec<i32>::iter()` 产出 `&i32`，那就只能是 `&i32`。为什么 `Iterator` 用关联类型而不用 `trait Iterator<T>`，第 16 章会给出完整理由，这里先记住结论：产出类型由迭代器自己决定，不由调用者指定。

### for 是 next 的语法糖

`for x in v` 会被展开成 `let mut it = v.into_iter(); loop { match it.next() { Some(x) => 处理 x, None => break } }`。入口是 `IntoIterator::into_iter`，`Iterator` 只是它的产物；所以一个类型想支持 `for`，要实现的 trait 是 `IntoIterator` 而不是 `Iterator`——两者的分工是"能变成迭代器"和"是迭代器"。

### 三个入口：into_iter、iter、iter_mut

同一个 `Vec<T>` 有三个入口，区别只在**元素类型**和**原集合的归属**：

| 写法 | Item 类型 | 原集合在循环后还能用吗 |
| --- | --- | --- |
| `v.into_iter()` 或 `for x in v` | `T` | 不能，所有权被搬进迭代器 |
| `v.iter()` 或 `for x in &v` | `&T` | 能，只是共享借用 |
| `v.iter_mut()` 或 `for x in &mut v` | `&mut T` | 能，独占借用结束后恢复 |

选哪个不是风格问题，而是所有权问题。只需要读，就 `&v`；读完还要用，别写 `for x in v`；要原地改，用 `&mut v`。

### next 为什么要 &mut self

`next` 的接收者是 `&mut self`，因为迭代器**有状态**：`v.iter()` 内部记着"走到第几个了"。这直接带来两个后果。第一，变量要声明成 `mut`：`let it = v.iter();` 之后直接写 `it.next()` 会报 E0596（cannot borrow as mutable），得改成 `let mut it`。第二，一个迭代器同一时刻只能有一个拉取者。`&mut` 是独占借用，所以两个线程不可能同时 `next` 同一个迭代器——数据竞争在这里被类型系统从源头掐掉了（第 21 章讲 `Send` 时会再回到这个点）。

## 13.2 惰性：适配器只搭管道

`map`、`filter` 这些方法叫**适配器**（adapter）。它们**不执行任何计算**，只返回一个包着上游的新迭代器。下面这段代码可以证明：

```rust
fn main() {
    let v = vec![1, 2, 3];
    let doubled = v.iter().map(|x| {
        println!("map 处理 {}", x);
        x * 2
    });
    println!("链条搭好了，还没开始");
    let sum: i32 = doubled.sum();
    println!("sum = {}", sum);
}
```

```text
链条搭好了，还没开始
map 处理 1
map 处理 2
map 处理 3
sum = 12
```

"链条搭好了"先打印，说明 `map` 那三行一次都没跑。真正开始干活的是 `sum()`——这类方法叫**消费者**（consumer），它内部不停调 `next`，一个元素走完整条链，直到 `next` 返回 `None`。这个模型解释了三件容易困惑的事。

**一、链条可以反复搭、一次用。** 迭代器是"一次性"的：`sum()` 消费掉 `doubled` 之后，`doubled` 就被 `self` 拿走了，想再算一遍得重新 `v.iter().map(...)`。

**二、短路是真的短路。** `any`、`all`、`find`、`position`、`take`、`take_while` 一旦不再需要元素，就不会继续调 `next`：`vec![1, 2, 3, 4, 5].iter().find(|x| **x > 2)` 返回 `Some(3)`，后面的 4、5 根本没被访问。如果链条里有带副作用的操作（打印、写文件、发请求），短路会让它少执行——这既是优化，也是坑：别把必须执行的操作塞进 `map` 里指望它一定跑。

**三、适配器可以无限长。** `(0..).map(|x| x * x)` 是合法的，因为没人拉它就不会算。配上边界才落地：`(0..).map(|x| x * x).take(5).collect::<Vec<u64>>()` 得到 `[0, 1, 4, 9, 16]`。`(0..)` 是 `RangeFrom`，没有上界，但它只在 `take` 要元素时被拉五次。

## 13.3 适配器与消费者

标准库把迭代器方法分成两类，分清这两类是读懂链式代码的前提：

- **适配器**：接收一个迭代器，返回另一个迭代器，不触发计算。
- **消费者**：接收一个迭代器，返回一个普通值（数字、`Vec`、`Option`、`bool`），触发计算。

### 常用适配器

| 适配器 | 作用 | 一句话提醒 |
| --- | --- | --- |
| `map` | 每个元素过一遍函数 | 一对一，元素个数不变 |
| `filter` | 保留谓词为真的元素 | 闭包收到的是 `&Item`，比较时通常要解引用 |
| `filter_map` | 一边过滤一边转换 | 闭包返回 `Option`，`None` 被丢掉 |
| `flat_map` | 每个元素展开成一段 | 一对多，结果会被拍平 |
| `take` / `skip` | 取前 n 个 / 跳过前 n 个 | 越界不 panic，取不够就是短 |
| `take_while` / `skip_while` | 按条件取 / 跳 | 条件一旦为假就停，不会再看后面 |
| `chain` | 首尾相接 | 两个迭代器的 Item 类型必须相同 |
| `zip` | 配对 | 短的耗尽就结束 |
| `enumerate` | 附下标 | 产出 `(usize, Item)` |

先把最常用的一条链写出来：

```rust
fn main() {
    let text = "the quick brown fox jumps over the lazy dog";
    let words: Vec<&str> = text.split_whitespace().collect();
    println!("{} 个词", words.len());

    let long: Vec<String> = words
        .iter()
        .filter(|w| w.len() > 4)
        .map(|w| w.to_uppercase())
        .collect();
    println!("{:?}", long);

    let longest = words.iter().max_by_key(|w| w.len()).unwrap();
    println!("最长的词：{}", longest);
}
```

```text
9 个词
["QUICK", "BROWN", "JUMPS"]
最长的词：jumps
```

注意 `filter` 的闭包参数 `w` 类型是 `&&str`：`words.iter()` 产出 `&&str`，`filter` 又把元素以引用形式交给谓词，于是多了一层。`w.len()` 能直接写是因为自动解引用帮你剥掉了两层；要比较内容时就得写 `**w == "the"` 或者干脆用 `filter(|w| *w == "the")` 让编译器自己剥。这条"引用叠引用"的规律是所有适配器的共同点：**`map` 拿到的是 `Item`，`filter` 拿到的是 `&Item`**。

### map 和 flat_map 的区别

`map` 一对一，`flat_map` 一对多并把结果拍平。拆 `["a b", "c d e"]` 时，`map(|l| l.split(' ').collect())` 得到嵌套的 `Vec<Vec<&str>>`，也就是 `[["a", "b"], ["c", "d", "e"]]`；`flat_map(|l| l.split(' '))` 直接得到 `["a", "b", "c", "d", "e"]`。要"把每行拆成词再合成一个列表"，`flat_map` 一步到位；用 `map` 还得再来一次 `flatten`。同理，解析字符串时常用 `filter_map`：转换失败的元素直接消失，不用先 `filter` 再 `map`——`["1", "x", "3"].iter().filter_map(|s| s.parse().ok()).collect::<Vec<i32>>()` 得到 `[1, 3]`。

### enumerate 与 zip

这两个是"带着额外信息遍历"的入口。手写索引循环最常见的两个场景，都可以用它们替掉：

```rust
fn main() {
    let names = ["ana", "bo", "cy"];
    let scores = [90, 80, 70];

    for (i, name) in names.iter().enumerate() {
        println!("{}: {}", i, name);
    }
    for (name, score) in names.iter().zip(scores.iter()) {
        println!("{} -> {}", name, score);
    }
}
```

`enumerate` 的 `i` 是 `usize`，从 0 开始。`zip` 在短的耗尽时结束，所以长度不同的两个切片不会 panic，多出来的部分被静默丢弃——这一点要留心，需要"长度不等就报错"时得自己先检查 `len()`。

### 常用消费者

| 消费者 | 返回 | 备注 |
| --- | --- | --- |
| `collect` | 任意 `FromIterator` 类型 | 目标类型常要标注，如 `Vec<_>`、`String` |
| `sum` / `product` | 元素类型（或标注的类型） | 空迭代器返回 0 与 1 |
| `count` | `usize` | 会走完整条链 |
| `fold` | 累积器的类型 | 最通用，能表达上面大半 |
| `any` / `all` | `bool` | 短路，空迭代器分别是 false / true |
| `find` | `Option<Item>` | 短路，返回第一个满足的**元素** |
| `position` | `Option<usize>` | 短路，返回**下标** |
| `max_by_key` / `min_by_key` | `Option<Item>` | 相等的元素取最后一个 |
| `last` / `nth` | `Option<Item>` | 都要把前面的元素走完 |

```rust
fn main() {
    let v = vec![1, 2, 3, 4];

    let sum = v.iter().fold(0, |acc, x| acc + x);
    let count = v.iter().fold(0, |acc, _| acc + 1);
    let (even, odd): (Vec<&i32>, Vec<&i32>) = v.iter().partition(|x| *x % 2 == 0);

    println!("{} {} {:?} {:?}", sum, count, even, odd);
}
```

输出 `10 4 [2, 4] [1, 3]`。`fold` 的两个参数是初始值和 `|累积器, 元素|` 闭包，累积器的类型可以和元素类型不同——上面第二个 `fold` 用 `usize` 累积器数个数，和 `count()` 等价。需要一边求和一边计数时，把累积器写成元组 `(i32, usize)` 就行，这是 `fold` 比 `sum`/`count` 强的地方。

`partition` 值得单独记一笔：它一次遍历把元素分成两堆，返回两个集合，比写两遍 `filter` 省一半时间。

## 13.4 闭包：捕获环境的三种深度

闭包的语法是两根竖线包住参数，后面跟表达式或代码块：

```rust
fn main() {
    let double = |x: i32| x * 2;
    let add = |a, b| a + b;          // 参数类型可以省略，靠调用点推断
    println!("{} {}", double(3), add(2, 5));
}
```

它看起来像函数，但和函数有一个本质区别：**闭包会捕获定义处可见的变量**。编译器为每个闭包字面量生成一个独一无二的匿名结构体，被捕获的变量就是它的字段，函数体变成这个结构体的一个方法。这也是闭包类型无法命名的原因——每个字面量都是一个新类型。

### Fn、FnMut、FnOnce

捕获之后，闭包要回答一个问题：body 打算怎么用这些字段？答案决定它实现哪个 trait，也决定它能被调用几次。

| body 对捕获值做的事 | 捕获方式 | 实现的 trait | 最多调用次数 |
| --- | --- | --- | --- |
| 只读（打印、比较） | 共享借用 `&T` | `Fn` | 无限次 |
| 修改（`+=`、`push`） | 可变借用 `&mut T` | `FnMut` | 无限次，但闭包变量要 `mut` |
| 移动出去（返回它、交给别的函数） | 按值 `T` | `FnOnce` | 一次 |

```rust
fn main() {
    let greeting = String::from("你好");
    let say = || println!("{}", greeting);
    say();
    say();
    println!("greeting 还在：{}", greeting);

    let mut count = 0;
    let mut bump = || {
        count += 1;
        println!("第 {} 次", count);
    };
    bump();
    bump();

    let owned = String::from("一次性");
    let consume = move || owned;
    println!("{}", consume());
}
```

三个 trait 之间是继承关系：`Fn` 是 `FnMut` 的子 trait，`FnMut` 是 `FnOnce` 的子 trait。所以 `F: FnOnce` 的位置三种闭包都能传，`F: Fn` 的位置只接受完全不改状态的闭包。函数如果只调用一次，就该把约束写到最宽的 `FnOnce`；写成 `Fn` 是在限制调用者。调用 `FnMut` 闭包需要一个 `&mut` 接收者，所以 `bump` 必须声明成 `let mut bump`。忘了这个 `mut`，报错会落在调用点（`cannot borrow as mutable`），而不是定义点——和结构体方法那次一样，别去改闭包体。

### 捕获方式由编译器推断

你写的代码里没有 `&T` 或 `T` 的字样，捕获方式是编译器**按 body 的实际用法选出来的最小方案**：能借就不借 mut，能借 mut 就不搬走。这就是为什么 `say` 借用了 `greeting`，之后 `greeting` 还能打印。从 2021 edition 起，捕获精确到**字段**而不是整个变量。`|| self.name` 只借用 `self.name`，不再连带借用整个 `self`，这让很多以前需要先解构再进闭包的写法变得不必要。

`move` 关键字强制"按值捕获"：无论 body 怎么用，都先把变量搬进闭包。两个典型用途：

```rust
fn make_adder(delta: i32) -> impl Fn(i32) -> i32 {
    move |x| x + delta
}

fn main() {
    let add5 = make_adder(5);
    println!("{}", add5(10));
}
```

不加 `move` 时闭包借的是 `delta`，而 `delta` 在函数返回时就被销毁了，闭包会持有一个悬垂借用，编译器直接拒绝。`move` 把 `delta` 的所有权搬进闭包，闭包走到哪都带着它。第二个用途是跨线程：`thread::spawn` 要求闭包 `'static`，只能靠 `move` 把数据带过去（第 21 章展开）。代价是原变量就没法用了，连"借出去的"都不行——这正是下面案例 1 的场景。

### 把闭包写进函数签名

闭包类型没有名字，所以签名里只能写 trait：

```rust
fn apply_twice<F: Fn(i32) -> i32>(f: F, x: i32) -> i32 {
    f(f(x))
}
fn repeat<F: FnMut()>(mut f: F, times: usize) {
    for _ in 0..times {
        f();
    }
}
fn main() {
    println!("{}", apply_twice(|x| x * 3, 2));
    let mut n = 0;
    repeat(|| n += 1, 3);
    println!("{}", n);
}
```

输出 `18` 和 `3`。`F: Fn(i32) -> i32` 是泛型参数加 trait bound，编译器会为每个实际传入的闭包类型生成一份 `apply_twice`，调用点完全内联——这是静态分派，零成本。参数写成 `impl Fn(i32) -> i32` 是同一件事的另一种写法，适合只有一个闭包参数时用。

如果状态真的不兼容，rustc 会明确告诉你差在哪一级。比如把会修改 `count` 的闭包传给要求 `Fn` 的函数，报错是 E0525：expected a closure that implements the Fn trait, but this closure only implements FnMut，并指到具体是哪个变量让它降级。看到这条，先问"这个闭包是不是必须修改环境"，再决定改约束还是改闭包。

要**返回**闭包，返回值位置写 `impl Fn(...)`（上面 `make_adder` 就是）。要把不同类型、不同捕获的闭包放进同一个容器，就只能退到 trait 对象：`Vec<Box<dyn Fn(i32) -> i32>>`，每个元素 `Box::new(闭包)`。这是动态分派，每次调用过一次虚表，而且挡住内联。判据只有一条——**闭包类型在编译期固定就用泛型，需要运行时才能决定放哪个才用 trait 对象**。第 15 章会从 vtable 布局的角度再讲一次这个取舍。

## 13.5 深入：零成本抽象是怎么成立的

"迭代器链和手写循环一样快"这句话经常被当成口号。这一节把它拆开，看它靠什么成立，以及什么情况下不成立。

### 单态化把链摊平成循环

三层机制叠在一起：

1. **每个适配器是一个泛型结构体**。`map` 返回 `Map<上游迭代器, 闭包类型>`，`filter` 返回 `Filter<...>`。链条有多长，类型就有多深，但每个类型在编译期都是确定的。
2. **单态化**（monomorphization）为每个具体类型组合生成一份专用代码。`Map<Iter<i32>, {closure}>` 的 `next` 就是"调用上游 `next`，把结果交给闭包"这么一小段普通函数，没有虚表、没有类型判断。
3. **内联**。这一小段函数被内联进调用者，适配器之间的边界消失，整条链退化成一个循环。

`(0..N).filter(|i| i % 3 == 0).map(|i| i * 2).sum()` 经过这三步，等价于：

```text
let mut acc = 0;
for i in 0..N {
    if i % 3 == 0 {
        acc += i * 2;
    }
}
```

在 release 模式下用 `std::time::Instant` 分别给两种写法计时（两千万个数），rustc 1.100 的实测输出：

```text
133333326666666 133333326666666
chain 14.222ms hand 14.2683ms
```

两者差 0.3%，在测量噪声内。想看更硬的证据，用 `cargo asm` 或 `objdump -d` 反汇编，两个版本的主循环指令序列是一样的。

**"零成本"有三个前提**，缺一个就不成立：

- **优化构建**。`cargo build` 默认是 debug，内联被关掉，链式代码会明显慢于手写循环。所有性能结论都必须用 `--release` 测。
- **静态分派**。`Box<dyn Iterator>` 或 `Box<dyn Fn>` 挡住了内联，每步多一次间接调用和一次无法预测的跳转。把 `dyn` 放进热循环，代价立刻可见。
- **单态化本身有开销**。每个闭包类型都生成一份代码，泛型用得越花，编译越慢、二进制越大。第 14 章会把这个代价算清楚。

### collect 靠 size_hint 预分配

每个迭代器都可以提供 `size_hint()`，返回剩余元素个数的下界和上界：

```rust
fn main() {
    let v: Vec<i32> = (1..=10).collect();

    let mapped = v.iter().map(|x| x * 2);
    println!("{:?}", mapped.size_hint());     // (10, Some(10))

    let filtered = v.iter().filter(|x| **x % 2 == 0);
    println!("{:?}", filtered.size_hint());   // (0, Some(10))
}
```

`Vec::from_iter` 的实现分两段：先按**下界**调 `with_capacity`，然后边走边 `push`，容量不够时按翻倍策略扩容。所以 `map` 这种"元素个数不变"的适配器能让 `collect` 一次分配到位，而 `filter` 的下界只能是 0——它不知道能留下几个，`collect` 只能从零开始扩容，这就是 `filter(...).collect()` 常常触发好几次 realloc 的原因；量级已知且在意这点开销时，可以先 `Vec::with_capacity(n)` 再 `extend`，把分配次数压到一次。自己实现迭代器时，`size_hint` 是**可选**的，默认返回 `(0, None)`。但只要你能算出准确值就该实现它——这是免费的优化入口，13.6 的 `Chunks` 会用到。

### FnMut() 其实是泛型 trait 的语法糖

`Fn` 家族的真正定义长这样（简化）：

```rust,noplayground
pub trait FnMut<Args>: FnOnce<Args> {
    extern "rust-call" fn call_mut(&mut self, args: Args) -> Self::Output;
}
```

写 `FnMut(i32) -> i32` 时，编译器把它展开成 `FnMut<(i32,), Output = i32>`：**参数被打包成一个元组，作为 trait 的泛型参数**；返回类型是关联类型 `Output`。

这解释了一个看起来不一致的现象：`dyn Iterator` 不用写类型参数，`dyn Fn(i32) -> i32` 却必须写。因为 `Iterator` 只靠关联类型 `Item` 描述产出，而 `Fn` 家族把"能接受什么参数"放在泛型参数上——trait 对象必须把泛型参数填死，否则编译器不知道要为哪个签名生成虚表。也正因为参数是泛型，手写 `impl Fn for MyType` 需要 `fn_traits` 和 `unboxed_closures` 两个不稳定特性。稳定 Rust 里，自定义"可调用"类型只有两条路：包一个闭包字段并转发，或者实现 `Deref` 指向闭包。

## 13.6 自己实现迭代器

读懂了 `next` 的契约，就可以自己造迭代器。这一节写两个：一个是无限序列，一个是把切片切块的适配器。

### Fibonacci：无限迭代器

斐波那契数列的每一项只依赖前两项，状态正好是两个字段：

```rust
struct Fibonacci {
    a: u64,
    b: u64,
}

impl Fibonacci {
    fn new() -> Self {
        Fibonacci { a: 0, b: 1 }
    }
}

impl Iterator for Fibonacci {
    type Item = u64;

    fn next(&mut self) -> Option<u64> {
        let current = self.a;
        self.a = self.b;
        self.b = current + self.b;
        Some(current)
    }
}

fn main() {
    let first: Vec<u64> = Fibonacci::new().take(10).collect();
    println!("{:?}", first);

    let sum: u64 = Fibonacci::new().take(10).sum();
    println!("{}", sum);

    let over: Vec<u64> = Fibonacci::new().take_while(|x| *x < 100).collect();
    println!("{:?}", over);
}
```

```text
[0, 1, 1, 2, 3, 5, 8, 13, 21, 34]
88
[0, 1, 1, 2, 3, 5, 8, 13, 21, 34, 55, 89]
```

两点值得注意。

**`next` 永远返回 `Some`。** 这个迭代器没有终点，它是**无限迭代器**，单独 `for x in Fibonacci::new()` 会永远转下去，必须配 `take`、`take_while` 之类的边界，否则程序挂死。状态放在字段里，`next` 只做一次推进——别在 `next` 里维护局部变量，函数返回后就没了，这也是 `next` 收 `&mut self` 的原因。

**`type Item = u64` 写在 impl 里。** 关联类型由实现者指定，调用者不能改。写错类型，`collect` 那一行会以 E0277 的形式报出来，而且错误指向调用点而不是 impl——案例 3 就是这个形状。

### Chunks：带生命周期的适配器

标准库的 `slice::chunks` 把切片按固定大小分组。自己写一遍能看到两个新问题：切片借用怎么标注，`size_hint` 怎么算：

```rust
struct Chunks<'a, T> {
    rest: &'a [T],
    size: usize,
}

impl<'a, T> Chunks<'a, T> {
    fn new(slice: &'a [T], size: usize) -> Self {
        assert!(size > 0, "size 必须大于 0");
        Chunks { rest: slice, size }
    }
}

impl<'a, T> Iterator for Chunks<'a, T> {
    type Item = &'a [T];

    fn next(&mut self) -> Option<&'a [T]> {
        if self.rest.is_empty() {
            return None;
        }
        let n = self.size.min(self.rest.len());
        let (head, tail) = self.rest.split_at(n);
        self.rest = tail;
        Some(head)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let n = self.rest.len().div_ceil(self.size);
        (n, Some(n))
    }
}

fn main() {
    let data = [1, 2, 3, 4, 5, 6, 7];
    let groups: Vec<&[i32]> = Chunks::new(&data, 3).collect();
    println!("{:?}", groups);
    println!("{:?}", Chunks::new(&data, 3).size_hint());
}
```

```text
[[1, 2, 3], [4, 5, 6], [7]]
(3, Some(3))
```

**`Item = &'a [T]` 里的 `'a` 是原始切片的生命周期**，不是 `&self` 的。迭代器本身只管记录"还剩哪一段"，产出的每一块都直接指向原始数据，一次复制都没有。写成 `Item = &[T]` 会被推断成借用 `self`，`collect` 立刻报"借用活得不够久"。第 17 章会把这个规则讲完整，这里只要抓住一点：**产出物借的是数据，不是迭代器**。

**`size_hint` 让 `collect` 一次分配到位。** 剩下 7 个元素、每块 3 个，需要 `ceil(7/3) = 3` 个槽位；`collect` 拿到下界 3 就直接开 3 个位置，全程不扩容。注释掉 `size_hint` 也能跑，只是会多几次 realloc。

**`assert!` 是必需的防御。** `size == 0` 时 `div_ceil` 会 panic，`split_at(0)` 则会让 `next` 永远不推进，`collect` 直接挂死。构造函数是检查参数最好的位置。

标准库已经有 `slice::chunks`、`chunks_exact`、`windows`、`split`，绝大多数需求轮不到你动手。本节的目的不是替代它们，而是让你在 `collect` 报出一个二十层深的类型名时，知道那每层是什么——以及写自己的适配器时，别忘了 `size_hint` 和参数检查这两件"可选的、但该做的事"。想让自定义容器支持 `for`，实现 `IntoIterator` 的三个关联项（`Item`、`IntoIter`、`into_iter`）即可，13.1 那张表就是这么来的。

## 典型错误案例

下面五个报错，是写完一段迭代器加闭包的代码后最常撞上的。每个都给出真实的 rustc 输出（节选，去掉了与本例无关的绝对路径行），重点看**错误指向哪一行**——很多人的第一反应会修错地方。

### 案例 1：move 闭包之后又用原变量——E0382

**错误代码**

```rust,editable
fn main() {
    let name = String::from("Ana");
    let greet = move || println!("hi {}", name);
    greet();
    println!("{}", name);
}
```

**rustc 怎么说**

```text
error[E0382]: borrow of moved value: `name`
 --> e1.rs:5:20
  |
2 |     let name = String::from("Ana");
  |         ---- move occurs because `name` has type `String`, which does not implement the `Copy` trait
3 |     let greet = move || println!("hi {}", name);
  |                 -------                   ---- variable moved due to use in closure
  |                 value moved into closure here
4 |     greet();
5 |     println!("{}", name);
  |                    ^^^^ value borrowed here after move
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**

`move` 在**创建闭包的那一刻**就把 `name` 的所有权搬进了闭包结构体，不是等到调用才搬。所以第 5 行报错不是因为 `greet()` 执行过——把第 4 行删掉，错误一模一样。闭包体只是 `println!`，它其实只需要一个 `&String`，是 `move` 强行升级成了按值捕获。

**修复**

闭包不需要活过 `main` 的话，去掉 `move`，让它借用：

```rust,editable
fn main() {
    let name = String::from("Ana");
    let greet = || println!("hi {}", name);
    greet();
    println!("{}", name);
}
```

如果闭包确实要离开当前作用域（返回出去、交给 `thread::spawn`），那就得两份数据：先 `clone` 一份搬进闭包，原变量留给外面用。

</details>

### 案例 2：map 里把捕获的 String 搬走——E0507

**错误代码**

```rust,editable
fn main() {
    let name = String::from("Ana");
    let nums = vec![1, 2];

    let out: Vec<String> = nums.iter().map(|_| name).collect();
    println!("{:?}", out);
}
```

**rustc 怎么说**

```text
error[E0507]: cannot move out of `name`, a captured variable in an `FnMut` closure
   --> e2.rs:4:48
    |
  2 |     let name = String::from("Ana");
    |         ----   ------------------- move occurs because `name` has type `String`, which does not implement the `Copy` trait
    |         |
    |         captured outer variable
  3 |     let nums = vec![1, 2];
  4 |     let out: Vec<String> = nums.iter().map(|_| name).collect();
    |                                            --- ^^^^ `name` is moved here
    |                                            |
    |                                            captured by this `FnMut` closure
    |
help: `Fn` and `FnMut` closures require captured values to be able to be consumed multiple times, but `FnOnce` closures may consume them only once
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**

`map` 的签名要求 `F: FnMut(Self::Item) -> B`。`FnMut` 意味着"可以被调用任意多次"，而闭包体把 `name` 返回出去，等于每次调用都要搬走一次所有权——第二次调用时 `name` 已经空了。编译器因此把闭包降级成 `FnOnce`（只能调一次），与 `map` 的要求冲突，E0507 就是这么来的。

注意错误位置在第 4 行的 `name`，不是 `map` 本身，也不是 `collect`。

**修复**

按提示 clone 一份，每次产出一个新的 `String`：

```rust,editable
fn main() {
    let name = String::from("Ana");
    let nums = vec![1, 2];

    let out: Vec<String> = nums.iter().map(|_| name.clone()).collect();
    println!("{:?}", out);
    println!("{}", name);
}
```

不过 `map(|_| x)` 这个形状本身值得怀疑：它对每个元素都返回同一个东西，通常说明想用的是 `vec![x; n]` 或 `std::iter::repeat(x).take(n)`。真需要"消耗型"闭包时，用 `for_each` 之类的 `FnMut` 之外的接口，或者干脆写 `for` 循环。

</details>

### 案例 3：collect 的元素类型对不上——E0277

**错误代码**

```rust,editable
fn main() {
    let v = vec![1, 2, 3];
    let out: Vec<String> = v.iter().collect();
    println!("{:?}", out);
}
```

**rustc 怎么说**

```text
error[E0277]: a value of type `Vec<String>` cannot be built from an iterator over elements of type `&{integer}`
    --> e3.rs:3:37
     |
   3 |     let out: Vec<String> = v.iter().collect();
     |                                     ^^^^^^^ value of type `Vec<String>` cannot be built from `std::iter::Iterator<Item=&{integer}>`
     |
help: the trait `FromIterator<&{integer}>` is not implemented for `Vec<String>`
      but trait `FromIterator<String>` is implemented for it
note: the method call chain might not have had the expected associated types
    --> e3.rs:3:30
     |
   2 |     let v = vec![1, 2, 3];
     |             ------------- this expression has type `Vec<{integer}>`
   3 |     let out: Vec<String> = v.iter().collect();
     |                              ^^^^^^ `Iterator::Item` is `&{integer}` here
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**

`collect` 只能把 `Item` 塞进目标容器。`v.iter()` 的 `Item` 是 `&i32`，`Vec<String>` 只接受 `String`，中间缺一次转换。这个错误链里三段 note 很有用：第一段说明 `Item` 到底是谁，第二段说明 `collect` 的约束从哪来。

**修复**

补上转换，显式写出"每个元素怎么变成 `String`"：

```rust,editable
fn main() {
    let v = vec![1, 2, 3];
    let out: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    println!("{:?}", out);
}
```

注意这里 `x` 是 `&i32`，`to_string` 通过自动解引用直接可用。另一种写法是 `v.into_iter()` 拿 `i32` 再 `to_string`，但那样 `v` 就被搬走了。

</details>

### 案例 4：在 &str 上调 iter——E0599

**错误代码**

```rust,editable
fn main() {
    let s = "hello";
    let out: Vec<char> = s.iter().collect();
    println!("{:?}", out);
}
```

**rustc 怎么说**

```text
error[E0599]: no method named `iter` found for reference `&str` in the current scope
 --> e4.rs:3:28
  |
3 |     let out: Vec<char> = s.iter().collect();
  |                            ^^^^ method not found in `&str`
  |
help: because of the in-memory representation of `&str`, to obtain an `Iterator` over each of its codepoint use method `chars`
  |
3 -     let out: Vec<char> = s.iter().collect();
3 +     let out: Vec<char> = s.chars().collect();
  |
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**

`iter` 是切片的固有方法，`&str` 上没有。字符串和 `Vec<T>` 长得像，但 `&str` 是 UTF-8 字节序列，不是 `[char]`，所以没法像数组那样按"元素"索引——按字节切会切坏多字节字符。Rust 给它的入口是 `chars`（按码点）、`bytes`（按字节）、`split_whitespace` / `split`（按分隔符）。

编译器在这里给了准确的建议，照做即可。注意 `s.chars()` 的 `Item` 是 `char`，正好能进 `Vec<char>`。

**修复**

```rust,editable
fn main() {
    let s = "hello";
    let out: Vec<char> = s.chars().collect();
    println!("{:?}", out);
}
```

需要按字节处理（比如写二进制协议）就用 `bytes`；需要人眼可见的字符就用 `chars`。两者长度不同——`"你好"` 是 2 个 `char`、6 个 `u8`。

</details>

### 案例 5：if 两个分支返回不同适配器——E0308

**错误代码**

```rust,editable
fn main() {
    let v = vec![1, 2, 3];
    let big = true;

    let it = if big {
        v.iter().map(|x| x + 1)
    } else {
        v.iter().filter(|x| **x > 1)
    };
    let out: Vec<&i32> = it.collect();
    println!("{:?}", out);
}
```

**rustc 怎么说**

```text
error[E0308]: `if` and `else` have incompatible types
 --> e5.rs:7:9
  |
4 |       let it = if big {
  |  ______________-
5 | |         v.iter().map(|x| x + 1)
  | |         -----------------------
  | |         |            |
  | |         |            the expected closure
  | |         expected because of this
6 | |     } else {
7 | |         v.iter().filter(|x| **x > 1)
  | |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^ expected `Map<Iter<'_, {integer}>, {closure@...}>`, found `Filter<Iter<'_, {integer}>, {closure@...}>`
8 | |     };
  | |_____- `if` and `else` have incompatible types
  |
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**

这是 13.5 那套"每个适配器是一个结构体"的直接后果。`map` 返回 `Map<...>`，`filter` 返回 `Filter<...>`，两个分支的表达式类型不同，`if` 表达式就无从确定类型。而且就算两个分支都写 `map`，只要闭包不同，两个匿名闭包类型也不同——错误信息里 `{closure@e5.rs:5:22: 5:25}` 那个位置标记就是在告诉你"这是两个不同的类型"。

**修复**

要保留"运行时二选一"，就得把具体类型擦掉，用 `Box<dyn Iterator>` 做动态分派；同时把两个分支的 `Item` 统一：

```rust,editable
fn main() {
    let v = vec![1, 2, 3];
    let big = true;

    let it: Box<dyn Iterator<Item = i32>> = if big {
        Box::new(v.iter().map(|x| x + 1))
    } else {
        Box::new(v.iter().copied().filter(|x| *x > 1))
    };

    let out: Vec<i32> = it.collect();
    println!("{:?}", out);
}
```

代价是每个元素多一次虚表调用，而且挡住了内联（13.5 讲过）。如果两个分支本来就该各自 `collect`，让两边都产出 `Vec<i32>` 即可，`Box` 都不需要——判据是：**分支的结果马上要被消费，就在分支内消费掉；后面还要继续接适配器，才需要 `Box<dyn Iterator>`**。

</details>

## 动手练习

**练习 1（基础）** 用一条迭代器链，把一段文本里的单词按长度分组，得到 `BTreeMap<usize, Vec<String>>`。要求只遍历一次输入，不许先 `collect` 到 `Vec` 再循环。

提示：`fold` 的累积器就是那个 `BTreeMap`，闭包里用 `entry(len).or_default().push(...)`。写完检查 `map` 的返回值是不是累积器本身——漏了它，`fold` 的下一轮会拿到 `()`。

**练习 2（适配器）** 实现一个 `Pairs` 适配器，作用于 `&[i32]`，依次产出相邻两个元素组成的 `(i32, i32)`，效果等于 `slice::windows(2)` 的迭代器版本。要求：

- 元素不足两个时立刻返回 `None`，之后保持 `None`
- 实现 `size_hint`，让 `collect` 能一次分配到位
- 用 `[1, 2, 3, 4]` 测出 `[(1, 2), (2, 3), (3, 4)]`

提示：结构体里存 `rest: &'a [i32]`，`next` 里先取 `rest[0]`、`rest[1]`，再把 `rest` 前进一位。`size_hint` 是 `len.saturating_sub(1)`——注意空切片和单元素切片都要得到 0，别让减法下溢。

**练习 3（性能）** 给 13.6 的 `Fibonacci` 实现 `nth`，要求 `.nth(1_000_000)` 在一瞬间返回，而默认实现要迭代一百万次。验证 `Fibonacci::new().nth(10)` 和 `Fibonacci::new().skip(10).next()` 结果相同。

提示：`nth(n)` 的契约是"消费掉前 n 个元素并返回第 n 个"，直接跳到目标状态即可，别去循环调 `next`。可以用矩阵快速幂或 fast doubling，也可以老老实实从当前 `(a, b)` 推进 n 步——先让正确性过，再谈 `O(log n)`。

**练习 4（闭包 trait）** 把 13.4 的 `repeat` 改成只接受 `FnOnce()`，观察编译错误，并说明为什么它不能循环调用。然后写一个 `once` 函数，签名 `fn once<F: FnOnce()>(f: F)`，只调用一次。

**练习 5（验证零成本）** 复现 13.5 的对比程序，分别在 `cargo build` 和 `cargo build --release` 下运行，记录两组耗时；再用 `cargo asm`（或 `objdump -d`）对比两种写法生成的主循环。

## 小结

- `Iterator` 只有一个必选方法 `next`，`Item` 是关联类型；`None` 之后必须继续返回 `None`，这条契约编译器不检查，得自己守。
- `for` 是 `IntoIterator::into_iter` 加 `loop { match it.next() }` 的语法糖；`into_iter`/`iter`/`iter_mut` 决定元素是 `T`、`&T` 还是 `&mut T`，也决定原集合还能不能用。
- 适配器只搭管道、不计算，消费者才拉动整条链；所以链条可以无限长，也可以只在真正需要时才干活。
- `any`、`all`、`find`、`position`、`take` 都会短路，副作用放进 `map` 里就别指望它一定执行。
- 闭包捕获哪种方式由 body 的用法推断：只读是 `Fn`，修改是 `FnMut`，搬走是 `FnOnce`；`Fn: FnMut: FnOnce`，约束按实际需要写，别一律写 `Fn`。
- `move` 使闭包按值捕获用到的变量或字段。`Copy` 值会复制，其他值移入闭包；`Fn`、`FnMut`、`FnOnce` 取决于闭包体如何使用捕获值。
- 函数签名里用 `F: Fn(..)` 或 `impl Fn(..)` 是静态分派，闭包类型在编译期固定；要装进同一个容器才用 `Box<dyn Fn>`。
- 零成本抽象靠"适配器是泛型结构体 + 单态化 + 内联"三步成立，前提是优化构建和静态分派；debug 构建下它并不零成本。
- `collect` 按 `size_hint` 的下界预分配，`filter` 的下界是 0 所以会扩容；自己写迭代器时尽量给出准确的 `size_hint`，并在构造函数里挡住非法参数。
