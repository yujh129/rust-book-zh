# 智能指针：Box、Rc、RefCell

> **本章目标**
>
> - 搞清"智能指针"和"引用"的本质区别：前者**拥有**数据，后者**借用**数据
> - 掌握 `Box<T>`：把值放到堆上，从而让递归类型有确定的体积
> - 掌握 `Rc<T>`：用引用计数实现"一份数据、多个所有者"
> - 用 `Weak<T>` 打破 `Rc` 之间的循环引用，避免内存泄漏
> - 掌握 `RefCell<T>`：把借用检查从编译期挪到运行期，实现"内部可变性"
> - 理解 `Rc<RefCell<T>>` 这个组合为什么能建模树和图
> - 知道 `Rc` 与 `Arc` 的差别，以及为什么 `Rc` 不能跨线程

前面几章的所有权模型有一个隐含假设：**一份数据只有一个所有者**。这在绝大多数场景下是正确的，但有些数据结构天生需要"多个地方都持有同一份数据"——双向链表、图、GUI 的节点树、浏览器里的 DOM。这一章介绍的工具，就是为这些场景开的口子。

它们都实现了一组相同的 trait，这组 trait 让它们"用起来像引用"：

```rust
use std::ops::Deref;

struct MyBox<T>(T);

impl<T> MyBox<T> {
    fn new(x: T) -> MyBox<T> {
        MyBox(x)
    }
}

impl<T> Deref for MyBox<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

fn main() {
    let x = 5;
    let y = MyBox::new(x);

    assert_eq!(5, x);
    assert_eq!(5, *y);
}
```

`*y` 能工作，是因为编译器自动把 `*y` 展开成 `*(y.deref())`。这就是"智能指针"的定义：**实现了 `Deref`（解引用）和 `Drop`（离开作用域时自动清理）的结构体**。

## 18.1 Box&lt;T&gt;：唯一所有权、堆分配

`Box<T>` 是最简单的智能指针：它在堆上分配一块内存存放 `T`，`Box` 自己（一个指针，8 字节）在栈上。所有权规则完全不变——`Box` 是唯一所有者，它被移动、被 drop 时，堆上的数据跟着走。

```rust
fn main() {
    let b = Box::new(5);
    println!("b = {}", b);
    // b 离开作用域时，堆上的 5 被释放
}
```

### 用途一：把"无限大小"变成已知大小

递归类型是 `Box` 最经典的用武之地。考虑一个链表：

```rust,noplayground
enum List {
    Cons(i32, List),
    Nil,
}
```

编译器会拒绝：`error[E0072]: recursive type List has infinite size`。原因很直白——要算出 `List` 的大小，得先算出它字段 `List` 的大小，无穷递归。

加上 `Box` 就断了这个循环：

```rust
#[derive(Debug)]
enum List {
    Cons(i32, Box<List>),
    Nil,
}

fn main() {
    let list = List::Cons(1, Box::new(List::Cons(2, Box::new(List::Nil))));
    println!("{:?}", list);
}
```

现在 `Cons` 的大小是 `i32 + 8 字节指针`，而指针指向的 `List` 在堆上，大小可以"以后再算"。用 `std::mem::size_of` 验证：

```rust
#[derive(Debug)]
enum List {
    Cons(i32, Box<List>),
    Nil,
}

fn main() {
    println!("{}", std::mem::size_of::<List>());
}
```

输出 `16`：这是本机 64 位目标上的测量值，包括变体标记、有效载荷和可能的填充，实际布局由编译器优化，但**是一个固定的小数字**，不再是"无限"。

### 用途二：把大对象搬出栈

栈空间默认只有 1–8 MB（Linux 常见 8 MB，Windows 默认 1 MB），而栈溢出直接 abort，无法恢复。一个 `[u8; 10_000_000]` 放在栈上会立刻崩溃：

```rust
fn main() {
    let big = vec![0u8; 10_000_000].into_boxed_slice();
    println!("{}", big.len());
}
```

这里先让 `Vec` 为元素分配堆缓冲区，再取得定长的 `Box<[u8]>`。直接写 `Box::new([0; 10_000_000])` 可能先在栈上构造临时数组，尤其在调试构建中；不能依赖优化来避免栈溢出。

### 用途三：trait 对象需要 `Box<dyn Trait>`

```rust
trait Shape {
    fn area(&self) -> f64;
}

struct Circle {
    r: f64,
}

struct Square {
    side: f64,
}

impl Shape for Circle {
    fn area(&self) -> f64 {
        3.14159 * self.r * self.r
    }
}

impl Shape for Square {
    fn area(&self) -> f64 {
        self.side * self.side
    }
}

fn main() {
    let shapes: Vec<Box<dyn Shape>> = vec![
        Box::new(Circle { r: 1.0 }),
        Box::new(Square { side: 2.0 }),
    ];
    let total: f64 = shapes.iter().map(|s| s.area()).sum();
    println!("{:.2}", total);
}
```

`dyn Shape` 是**不定长类型**（大小取决于具体实现），不能直接放进 `Vec`，必须加一层间接（`Box` 或 `&`）。

### Box 的成本

- 分配：一次 `malloc`（约几十纳秒，取决于分配器）
- 访问：通过指针取得值；缓存命中取决于数据位置和访问模式，`Rc` 的引用计数开销主要在克隆和释放时产生
- 没有额外计数，drop 就是一次 `free`

判断标准：**如果你只需要"唯一所有者 + 间接" ，就用 Box**，不要因为它"不够智能"而换 Rc。

## 18.2 Rc&lt;T&gt;：共享所有权

`Rc<T>`（Reference Counted）在堆上存两样东西：数据本身，以及两个计数器——强引用数（`strong`）和弱引用数（`weak`）。`Rc::clone` 只是把 `strong` 加一，不复制数据；最后一个强引用 drop 时数据被释放。

```rust
use std::rc::Rc;

fn main() {
    let a = Rc::new(String::from("共享的字符串"));
    println!("a 的引用计数：{}", Rc::strong_count(&a));

    let b = Rc::clone(&a);
    println!("a 的引用计数：{}", Rc::strong_count(&a));

    {
        let c = Rc::clone(&a);
        println!("a 的引用计数：{}", Rc::strong_count(&a));
        println!("c = {}", c);
    }

    println!("a 的引用计数：{}", Rc::strong_count(&a));
    println!("b = {}", b);
}
```

输出：

```text
a 的引用计数：1
a 的引用计数：2
a 的引用计数：3
c = 共享的字符串
a 的引用计数：2
b = 共享的字符串
```

注意 `Rc::clone(&a)` 而不是 `a.clone()`。两者效果一样（后者调用的是 `Rc` 的 `clone` 实现），但写 `Rc::clone` 是一个**约定**：告诉读者"这里只加计数，没有深拷贝数据"。

### Rc 不能修改数据

`Rc<T>` 只实现 `Deref`，不实现 `DerefMut`：

```rust,noplayground
use std::rc::Rc;

fn main() {
    let x = Rc::new(5);
    *x = 6;
}
```

报错 `error[E0594]: cannot assign to data in an Rc`，并提示 `trait DerefMut is required`。这是**故意的**：如果多个 `Rc` 都能改同一份数据，那就回到了 C++ 的 `shared_ptr` 别名问题，Rust 的借用规则就形同虚设。

要在共享的同时修改，需要 `RefCell`（18.4）或 `Cell`。这就是 `Rc<RefCell<T>>` 组合的由来。

### Rc 不能跨线程

`Rc` 的计数是**非原子**的自增自减，两个线程同时 `clone` 会丢计数：

```rust,noplayground
use std::rc::Rc;
use std::thread;

fn main() {
    let data = Rc::new(vec![1, 2, 3]);
    let d = Rc::clone(&data);
    thread::spawn(move || {
        println!("{}", d.len());
    }).join().unwrap();
}
```

报错 `error[E0277]: Rc<Vec<i32>> cannot be sent between threads safely`，`help: the trait Send is not implemented for Rc<Vec<i32>>`。跨线程共享请用 `Arc<T>`（Atomic Rc），它是 `Rc` 的原子版本，代价是每次 clone/drop 有一次原子操作（在 x86 上约 10–20 ns）。

### 共享数据的经典场景：多个结构体指向同一份配置

```rust
use std::rc::Rc;

struct Config {
    name: String,
    port: u16,
}

struct Server {
    config: Rc<Config>,
}

struct Client {
    config: Rc<Config>,
}

fn main() {
    let config = Rc::new(Config {
        name: String::from("my-app"),
        port: 8080,
    });

    let server = Server { config: Rc::clone(&config) };
    let client = Client { config: Rc::clone(&config) };

    println!("{}:{}", server.config.name, server.config.port);
    println!("{}:{}", client.config.name, client.config.port);
    println!("引用计数：{}", Rc::strong_count(&config));
}
```

用 `Clone` 也能实现，但那样 `Config` 会被复制两份；如果它内部有一个 `Vec` 或连接池，复制成本就很高了。

## 18.3 Weak&lt;T&gt;：打破循环引用

`Rc` 有一个致命弱点：**循环引用会泄漏内存**。两个 `Rc` 互相指向对方，计数永远不归零，数据永远不释放。

```rust
use std::cell::RefCell;
use std::rc::Rc;

struct Node {
    value: i32,
    next: RefCell<Option<Rc<Node>>>,
}

fn main() {
    let a = Rc::new(Node {
        value: 1,
        next: RefCell::new(None),
    });
    let b = Rc::new(Node {
        value: 2,
        next: RefCell::new(Some(Rc::clone(&a))),
    });
    *a.next.borrow_mut() = Some(Rc::clone(&b));

    println!("a 的计数：{}", Rc::strong_count(&a));
    println!("b 的计数：{}", Rc::strong_count(&b));
}
```

输出：

```text
a 的计数：2
b 的计数：2
```

`main` 结束时，`a` 和 `b` 两个局部变量被 drop，计数各减到 1——但它们的 `next` 字段仍然互相持有对方，没有任何人再持有 `a` 或 `b` 的可达路径，**数据却永远不会释放**。这就是内存泄漏，而且是 rustc 完全检测不出来的（`Rc` 的泄漏是"安全"的，不违反内存安全，只是浪费）。

### Weak 的作用：只观察、不拥有

`Weak<T>` 是 `Rc<T>` 的"弱"版本：它指向同一份数据，但**不增加强引用计数**。数据只在强引用归零时释放；`Weak` 通过 `upgrade()` 尝试拿到 `Rc`，如果数据已经没了就返回 `None`。

修复上面的循环，只需要把"反向边"改成 `Weak`：

```rust
use std::cell::RefCell;
use std::rc::{Rc, Weak};

struct Node {
    value: i32,
    parent: RefCell<Weak<Node>>,
    children: RefCell<Vec<Rc<Node>>>,
}

fn main() {
    let leaf = Rc::new(Node {
        value: 3,
        parent: RefCell::new(Weak::new()),
        children: RefCell::new(vec![]),
    });

    let branch = Rc::new(Node {
        value: 5,
        parent: RefCell::new(Weak::new()),
        children: RefCell::new(vec![Rc::clone(&leaf)]),
    });

    *leaf.parent.borrow_mut() = Rc::downgrade(&branch);

    println!("branch 强引用：{}，弱引用：{}",
        Rc::strong_count(&branch), Rc::weak_count(&branch));
    println!("leaf 强引用：{}", Rc::strong_count(&leaf));

    let parent = leaf.parent.borrow().upgrade();
    match parent {
        Some(p) => println!("leaf 的父节点是 {}", p.value),
        None => println!("父节点已被释放"),
    }
}
```

输出：

```text
branch 强引用：1，弱引用：1
leaf 强引用：1
leaf 的父节点是 5
```

`branch` 的强引用只有 1（来自局部变量），弱引用 1（来自 `leaf.parent`）。当 `branch` 被 drop，强引用归零，`branch.value` 被释放；`leaf.parent` 里的 `Weak` 仍在，但 `upgrade()` 会返回 `None`。

### `Weak::upgrade` 为什么返回 `Option`

因为"弱引用总是存在"和"数据还存在"是两件事。`upgrade` 在内部做了一次原子/非原子检查：如果强计数 > 0，就加一并返回 `Some`；否则返回 `None`。这个检查和自增必须是原子的（在 `Arc` 中），否则会和另一个线程的 drop 竞争。

### 什么时候用 Weak

- 父节点持有子节点用 `Rc`，子节点指回父节点用 `Weak`
- 观察者模式：订阅者表用 `Weak` 保存观察者，观察者销毁后自动失效
- 缓存：值用 `Weak` 存，没人用了就让 GC 自然回收
- 任何"反向指针"或"非拥有关系"

## 18.4 RefCell&lt;T&gt;：运行期借用检查

前面说过 `Rc<T>` 不能改数据。但很多场景确实需要"共享 + 修改"：GUI 里两个组件都持有同一份状态，都要改。`RefCell<T>` 提供了这个能力——它把借用检查从**编译期**搬到**运行期**。

```rust
use std::cell::RefCell;

fn main() {
    let c = RefCell::new(5);
    println!("初始：{}", c.borrow());

    *c.borrow_mut() += 1;
    println!("修改后：{}", c.borrow());

    // 这里 a 和 b 的作用域不重叠，所以不会 panic
    {
        let a = c.borrow();
        println!("a = {}", a);
    }
    {
        let mut b = c.borrow_mut();
        *b += 10;
    }
    println!("最终：{}", c.borrow());
}
```

`borrow()` 返回 `Ref<T>`，`borrow_mut()` 返回 `RefMut<T>`。这两个守卫对象在 drop 时把计数还回去。

### 违反规则会 panic，不是编译错误

```rust,noplayground
use std::cell::RefCell;

fn main() {
    let c = RefCell::new(5);
    let a = c.borrow();
    let b = c.borrow_mut();
    println!("{} {}", a, b);
}
```

这段代码**能编译通过**，运行时报：

```text
thread 'main' panicked at t5.rs:6:15:
RefCell already borrowed
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

这就是 `RefCell` 的代价：检查推迟到运行期，违反规则时是 panic（不是 UB，所以仍然安全），程序可能因此崩溃。

### 用 `try_borrow` 优雅处理

不想 panic，就用 `try_borrow` / `try_borrow_mut`，它们返回 `Result`：

```rust
use std::cell::RefCell;

fn main() {
    let c = RefCell::new(5);
    let a = c.borrow();

    match c.try_borrow_mut() {
        Ok(mut b) => {
            *b += 1;
            println!("修改成功：{}", b);
        }
        Err(e) => println!("借用失败：{}", e),
    }

    println!("a 仍然有效：{}", a);
}
```

### RefCell 的适用范围

- **单线程**。`RefCell` 的内部计数是 `Cell<isize>`，不是原子的，因此 `RefCell<T>` 不是 `Sync`。多线程共享可改状态请用 `Mutex<T>` 或 `RwLock<T>`
- **不改变所有权**。`RefCell` 只是绕过借用检查，它自己仍然遵循所有权规则
- **适合"内部可变性"**：结构体对外是 `&`, 但内部需要改某个字段

```rust
use std::cell::RefCell;

struct Counter {
    count: RefCell<u32>,
}

impl Counter {
    fn new() -> Self {
        Counter { count: RefCell::new(0) }
    }

    // 注意：&self 而不是 &mut self
    fn increment(&self) -> u32 {
        let mut c = self.count.borrow_mut();
        *c += 1;
        *c
    }

    fn get(&self) -> u32 {
        *self.count.borrow()
    }
}

fn main() {
    let c = Counter::new();
    println!("{}", c.increment());
    println!("{}", c.increment());
    println!("{}", c.get());
}
```

### `Cell<T>`：更轻量的内部可变性

如果 `T: Copy`，用 `Cell<T>` 更便宜——它没有借用计数，`get` 直接复制，`set` 直接覆盖：

```rust
use std::cell::Cell;

fn main() {
    let c = Cell::new(10);
    c.set(c.get() + 5);
    println!("{}", c.get());
}
```

`Cell` 的问题是拿不到 `&T`——只能整体读写。要"借用内部"，就得 `RefCell`。

## 18.5 深入：Rc 的内存布局与释放顺序

`Rc<T>` 在堆上的实际布局是：

```text
RcBox<T> {
    strong: Cell<usize>,   // 强引用计数
    weak:   Cell<usize>,   // 弱引用计数（含 1 个"隐式弱引用"，见下）
    value:  T,             // 真正的数据
}
```

`Rc<T>` 本体就是一个指向 `RcBox` 的指针。注意 **`weak` 的初始值是 1，不是 0**：这个 1 由所有强引用"共同"持有，代表"还有一个强引用存在"。所以：

- 强引用归零时：先 `drop(value)`，然后把 `strong` 标记为 0，`weak` 减 1（释放隐式弱引用）
- 弱引用归零时：`RcBox` 本身被释放

这解释了两个现象：

1. `Rc::strong_count` 只数强引用，`Rc::weak_count` 只数显式 `Weak`
2. 数据释放得比 `RcBox` 早——这就是为什么 `Weak::upgrade` 要检查 `strong > 0`

### 在同一个线程里，drop 顺序是可以观察的

```rust
use std::rc::Rc;

struct Loud(&'static str);

impl Drop for Loud {
    fn drop(&mut self) {
        println!("drop {}", self.0);
    }
}

fn main() {
    let a = Rc::new(Loud("inner"));
    println!("a 计数：{}", Rc::strong_count(&a));

    {
        let b = Rc::clone(&a);
        println!("a 计数：{}", Rc::strong_count(&a));
        drop(b);
    }

    println!("准备离开 main");
}
```

输出：

```text
a 计数：1
a 计数：2
准备离开 main
drop inner
```

`Loud` 只在**最后一个** `Rc` drop 时被打印，且发生在 `main` 结束时。这说明 `Rc` 不复制 `T`，也不提前释放。

### `Rc` 与 `Arc` 的成本差

`Arc` 的 `clone` 是一次 `lock xadd`，`drop` 是一次 `lock xsub`（或 `fetch_sub` + 判断），在 x86-64 上大约 10–20 ns 一次，且会阻止编译器重排。因此：

- 单线程内共享：用 `Rc`，不要用 `Arc`
- 跨线程共享：只能用 `Arc`
- 高频 clone/drop 的路径上：考虑用 `&T` 或 `Arc` 只 clone 一次再传引用

### `Rc::make_mut`：写时复制

`Rc` 有一个巧妙的方法：如果当前只有你一个强引用，`make_mut` 直接返回 `&mut T`（零拷贝）；否则先 clone 一份数据。这叫 **copy-on-write**：

```rust
use std::rc::Rc;

fn main() {
    let mut a = Rc::new(vec![1, 2, 3]);
    let b = Rc::clone(&a);

    // 有两个强引用，make_mut 会复制一份
    Rc::make_mut(&mut a).push(4);

    println!("a = {:?}", a);
    println!("b = {:?}", b);
}
```

输出 `a = [1, 2, 3, 4]`、`b = [1, 2, 3]`。这是 `im`、`rpds` 这类持久化数据结构的核心技巧。

## 18.6 组合：用 `Rc&lt;RefCell&lt;T&gt;&gt;` 建模树

把 `Rc`（共享所有权）和 `RefCell`（内部可变）组合起来，就得到"多个所有者 + 都能改"的能力。这是 Rust 里建模图的常见手段。

下面的例子实现一棵简单树，支持往节点里加子节点，并且子节点能拿到父节点：

```rust
use std::cell::RefCell;
use std::rc::{Rc, Weak};

#[derive(Debug)]
struct TreeNode {
    name: String,
    parent: RefCell<Weak<TreeNode>>,
    children: RefCell<Vec<Rc<TreeNode>>>,
}

impl TreeNode {
    fn new(name: &str) -> Rc<TreeNode> {
        Rc::new(TreeNode {
            name: name.to_string(),
            parent: RefCell::new(Weak::new()),
            children: RefCell::new(Vec::new()),
        })
    }

    fn add_child(parent: &Rc<TreeNode>, child: Rc<TreeNode>) {
        *child.parent.borrow_mut() = Rc::downgrade(parent);
        parent.children.borrow_mut().push(child);
    }

    fn path_to_root(node: &Rc<TreeNode>) -> Vec<String> {
        let mut path = vec![node.name.clone()];
        let mut cur = node.parent.borrow().upgrade();
        while let Some(n) = cur {
            path.push(n.name.clone());
            cur = n.parent.borrow().upgrade();
        }
        path.reverse();
        path
    }
}

fn main() {
    let root = TreeNode::new("root");
    let a = TreeNode::new("a");
    let b = TreeNode::new("b");

    TreeNode::add_child(&root, Rc::clone(&a));
    TreeNode::add_child(&root, Rc::clone(&b));
    TreeNode::add_child(&a, TreeNode::new("a1"));

    println!("root 子节点数：{}", root.children.borrow().len());
    println!("a 的路径：{:?}", TreeNode::path_to_root(&a));
    println!(
        "root 强引用 {} 弱引用 {}",
        Rc::strong_count(&root),
        Rc::weak_count(&root)
    );
}
```

关键点：

- `children: RefCell<Vec<Rc<TreeNode>>>` —— 父节点**拥有**子节点，用 `Rc`
- `parent: RefCell<Weak<TreeNode>>` —— 子节点**不拥有**父节点，用 `Weak`，避免循环
- 所有方法都只拿 `&self`（因为内部是 `RefCell`），调用方不需要 `&mut`

### 这个组合的代价

- **运行期 panic 风险**：`borrow_mut` 冲突时会崩，而不是编译错误。要格外注意"借用期间不要回调用户代码"
- **调试困难**：类型写成 `Rc<RefCell<Vec<Rc<RefCell<Node>>>>>` 时，报错信息会非常长
- **性能**：每次访问多一次计数检查和指针跳转，比 `Box` 慢
- **不是并发安全的**：需要跨线程就用 `Arc<Mutex<T>>`

**经验法则**：能用 `Box` 就用 `Box`；确实需要共享再加 `Rc`；确实需要修改再加 `RefCell`；确实需要跨线程才用 `Arc<Mutex>`。层层加码，不要一上来就"最灵活的写法"。


## 典型错误案例

### 案例 1：递归类型没有间接层——E0072

```rust,editable
enum List {
    Cons(i32, List),
    Nil,
}

fn main() {
    let l = List::Cons(1, List::Nil);
    let _ = l;
}
```

```text
error[E0072]: recursive type `List` has infinite size
 --> src/main.rs:1:1
  |
1 | enum List {
  | ^^^^^^^^^
2 |     Cons(i32, List),
  |               ---- recursive without indirection
  |
help: insert some indirection (e.g., a `Box`, `Rc`, or `&`) to break the cycle
  |
2 |     Cons(i32, Box<List>),
  |               ++++    +

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`List` 的大小 = `i32` + `List` 的大小 + 判别式，递归定义导致编译器无法算出有限大小。rustc 的 help 甚至直接给出了 `Box<List>` 这个补丁。

**修复**：插入一层间接（`Box` 或 `Rc`），把"值"变成"指针"：

```rust,editable
#[derive(Debug)]
enum List {
    Cons(i32, Box<List>),
    Nil,
}

fn main() {
    let l = List::Cons(1, Box::new(List::Cons(2, Box::new(List::Nil))));
    println!("{:?}", l);
}
```

</details>

### 案例 2：试图修改 `Rc` 里的数据——E0594

```rust,editable
use std::rc::Rc;

fn main() {
    let x = Rc::new(5);
    *x = 6;
    println!("{}", x);
}
```

```text
error[E0594]: cannot assign to data in an `Rc`
 --> src/main.rs:5:5
  |
5 |     *x = 6;
  |     ^^^^^^ cannot assign
  |
  = help: trait `DerefMut` is required to modify through a dereference, but it is not implemented for `Rc<i32>`

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Rc<T>` 只实现 `Deref`，不实现 `DerefMut`。编译器用 `E0594`（而不是借用检查的 `E0596`）表达"这个类型根本不支持可变解引用"。

**修复**：用 `RefCell` 把可变性放进内部；或者如果只有一个所有者，改用 `Box`：

```rust,editable
use std::cell::RefCell;
use std::rc::Rc;

fn main() {
    let x = Rc::new(RefCell::new(5));
    *x.borrow_mut() = 6;
    println!("{}", x.borrow());
}
```

</details>

### 案例 3：把 `Rc` 送进另一个线程——E0277

```rust,editable
use std::rc::Rc;
use std::thread;

fn main() {
    let data = Rc::new(vec![1, 2, 3]);
    let d = Rc::clone(&data);
    thread::spawn(move || {
        println!("{}", d.len());
    }).join().unwrap();
}
```

```text
error[E0277]: `Rc<Vec<i32>>` cannot be sent between threads safely
 --> src/main.rs:7:19
  |
7 |     thread::spawn(move || {
  |     ------------- ^------
  |     |             |
  |     |             within this `{closure@src/main.rs:7:19: 7:26}`
  |     |             required by a bound introduced by this call
  |
  = help: within `{closure@src/main.rs:7:19: 7:26}`, the trait `Send` is not implemented for `Rc<Vec<i32>>`
note: required because `d` is used within this closure
 --> src/main.rs:8:24
  |
8 |         println!("{}", d.len());
  |                        ^
note: required by a bound in `spawn`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`thread::spawn` 要求闭包 `F: Send + 'static`。`Rc` 的计数非原子，若允许跨线程，两个线程同时 clone 会导致计数错乱、双重释放。因此 `Rc<T>` 没有 `Send`。

**修复**：跨线程用 `Arc<T>`：

```rust,editable
use std::sync::Arc;
use std::thread;

fn main() {
    let data = Arc::new(vec![1, 2, 3]);
    let d = Arc::clone(&data);
    let h = thread::spawn(move || d.len());
    println!("{}", h.join().unwrap());
    println!("{}", data.len());
}
```

如果还需要修改，用 `Arc<Mutex<Vec<i32>>>`。

</details>

### 案例 4：从 `Rc` 里移出数据——E0507

```rust,editable
use std::rc::Rc;

fn main() {
    let s = Rc::new(String::from("hello"));
    let t = *s;
    println!("{}", t);
}
```

```text
error[E0507]: cannot move out of an `Rc`
 --> src/main.rs:5:13
  |
5 |     let t = *s;
  |             ^^ move occurs because value has type `String`, which does not implement the `Copy` trait
  |
help: consider removing the dereference here
  |
5 -     let t = *s;
5 +     let t = s;
  |
help: consider cloning the value if the performance cost is acceptable
  |
5 -     let t = *s;
5 +     let t = s.clone();
  |

error: aborting due to 1 previous error
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`*s` 得到的是 `String` 类型的位置，但 `Rc` 只是**借用**式地解引用——数据可能还有其他所有者，不可能把它 move 走。`Rc` 永远不实现"可移动解引用"。

**修复**：要么 clone，要么改成借用 `&String`：

```rust,editable
use std::rc::Rc;

fn main() {
    let s = Rc::new(String::from("hello"));

    // 方案一：clone 出一份独立数据
    let t = (*s).clone();
    println!("{}", t);

    // 方案二：只借用
    let u: &String = &s;
    println!("{}", u);
}
```

如果确定只有你一个所有者，用 `Rc::try_unwrap`：

```rust,editable
use std::rc::Rc;

fn main() {
    let s = Rc::new(String::from("hello"));
    match Rc::try_unwrap(s) {
        Ok(v) => println!("拿到了所有权：{}", v),
        Err(rc) => println!("还有 {} 个所有者", Rc::strong_count(&rc)),
    }
}
```

</details>

### 案例 5：`RefCell` 同时借用与可变借用——运行期 panic

```rust,editable
use std::cell::RefCell;

fn main() {
    let c = RefCell::new(5);
    let a = c.borrow();
    let b = c.borrow_mut();
    println!("{} {}", a, b);
}
```

```text
thread 'main' panicked at src/main.rs:6:15:
RefCell already borrowed
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：这段代码**编译完全通过**——`RefCell` 的借用检查在运行期。`borrow()` 返回 `Ref` 守卫并标记"有共享借用"，此时 `borrow_mut()` 检测到冲突，直接 panic。注意它不是 UB：panic 是安全的失败方式。

**修复**：缩小借用作用域，或用 `try_borrow_mut` 处理失败：

```rust,editable
use std::cell::RefCell;

fn main() {
    let c = RefCell::new(5);

    {
        let a = c.borrow();
        println!("先读：{}", a);
    }

    let result = c.try_borrow_mut();
    match result {
        Ok(mut b) => {
            *b += 1;
            println!("再改：{}", b);
        }
        Err(e) => println!("借用冲突：{}", e),
    }
}
```

生产代码里，`RefCell` 的 panic 常常出现在"借用期间调用了会再借用的方法"——比如在 `for item in list.borrow()` 循环体里调用 `list.borrow_mut()`。

</details>

## 动手练习

**练习 1**：用 `Box` 实现一个单向链表 `List<T>`，提供 `push`（头插）、`pop`、`len`、`iter`。然后把它改成 `Rc<List<T>>` 版本，讨论为什么 `Rc` 版本写 `pop` 会更难。

**练习 2**：实现一个 `SharedCounter`，用 `Rc<Cell<u32>>` 让多个结构体共享一个计数器并能自增。再改成 `Rc<RefCell<Vec<String>>>`，让多个结构体往同一个日志里追加内容。

**练习 3**：用 `Weak` 实现观察者模式：`Subject` 持有 `Vec<Weak<dyn Observer>>`，`notify` 时跳过 `upgrade()` 失败的观察者。验证当某个观察者被 drop 后，它不再收到通知。

**练习 4**：下面的代码能编译，但运行时会 panic。找出原因并改对（提示：`borrow` 守卫活到了循环体内部）。

```rust,noplayground
use std::cell::RefCell;

fn main() {
    let v = RefCell::new(vec![1, 2, 3]);
    for x in v.borrow().iter() {
        v.borrow_mut().push(*x);
    }
}
```

**练习 5**：用 `Rc<RefCell<TreeNode>>` 建一棵树，写一个函数 `count_nodes` 和 `max_depth`。然后故意造一个循环引用，用 `Rc::strong_count` 观察泄漏，再用 `Weak` 修好。

## 小结

- 智能指针 = `Deref` + `Drop`；它们拥有数据，而引用只借用数据
- `Box<T>`：唯一所有权 + 堆分配。递归类型、`dyn Trait`、超大对象都靠它
- 递归类型必须有一层间接（`Box`/`Rc`/`&`），否则 `E0072`
- `Rc<T>`：引用计数实现共享所有权，`Rc::clone` 只加计数不复制数据
- `Rc` 不实现 `DerefMut`，不能直接修改；也不能跨线程（无 `Send`），跨线程用 `Arc`
- `Weak<T>` 不增加强计数，用 `upgrade()` 拿到 `Option<Rc<T>>`，是打破循环引用的标准手段
- `RefCell<T>` 把借用检查推迟到运行期，违规时 panic 而非编译错误；单线程、非 `Sync`
- `Cell<T>` 在 `T: Copy` 时更轻量，但只能整体读写
- `Rc<RefCell<T>>` 是"共享 + 可改"的经典组合；`Arc<Mutex<T>>` 是它的并发版本
- 选型要克制：能 `Box` 就别 `Rc`，能 `Rc` 就别 `RefCell`，能单线程就别 `Arc<Mutex>`
