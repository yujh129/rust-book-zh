# 第 37 章 设计模式在 Rust 中的落地

> **本章目标**
>
> - 说清为什么 GoF 的二十三种模式到 Rust 里只剩下一小半：哪些被语言特性吸收，哪些被所有权否定。
> - 用 newtype 把单位与语义写进类型，并在不违反孤儿规则的前提下为外部类型实现外部 trait。
> - 用类型参数与 PhantomData 把状态机编码进类型，让非法状态转换在编译期就不存在。
> - 会区分泛型单态化与 trait 对象两种策略实现，并按热路径、异构集合、二进制体积给出选择判据。
> - 用通道与枚举分别替换观察者模式和命令模式，理解为什么回调列表在 Rust 里很难写对。
> - 会用扩展 trait 给外部类型加方法，用 sealed trait 封死外部实现，并从编译产物角度解释这些手法的运行时开销。

1994 年，四名作者把当时面向对象编程的经验总结成二十三个设计模式，写进《设计模式：可复用面向对象软件的基础》。那本书用的语言是 C++ 和 Smalltalk，前提是对象之间共享可变状态、通过继承复用实现、用虚函数完成分派。三十年后的今天，这些前提在 Rust 里有的被语言直接吸收，有的被所有权规则明确禁止。

结果是容易被误解的现象：GoF 的二十三个模式，在 Rust 里能原样照搬的不到一半。这不是 Rust 的缺陷，而是设计模式本身的定位——它们大多是语言缺口的补丁。当一个语言补上缺口，对应的模式就退化成普通语法。

## 37.1 为什么 GoF 模式在 Rust 里只剩一半

设计模式解决的问题，多数可以还原成一句话：这个语言缺了某个能力，于是用一套约定绕过去。策略模式（Strategy）用来在运行时换算法，是因为 C++ 没有一等函数；命令模式（Command）把操作包成对象，是为了把要做的事存起来、排队或撤销；观察者模式（Observer）维护一份回调列表，是因为没有别的机制能让对象在状态变化时通知别人；单例模式（Singleton）把全局状态藏进一个类，是因为没有更好的全局变量管理办法。

Rust 同时提供了三样东西：代数数据类型（enum）、一等闭包、trait 系统。前两样让策略和命令坍缩成值，第三样让一组行为既能静态分派也能动态分派。更关键的是所有权与借用检查器：它要求任何一个时刻，对同一份数据要么有多个只读借用，要么只有一个可变借用。GoF 里大量模式默认多个对象同时持有并修改共享状态，这个前提在 Rust 里直接不成立。观察者模式最典型：主题在遍历回调列表时，回调往往要回头修改主题，这构成对同一个值的两个可变借用，编译器直接拒绝。

所以正确的态度不是「Rust 没有设计模式」，而是先问这个模式在补哪个缺口，再看 Rust 是否已经补上。下面的表是一次粗略清点，表格里的代码用单引号表示。

| GoF 模式 | 在 Rust 中的归宿 |
| --- | --- |
| Strategy / State | 泛型参数、trait 对象，或直接写成 enum 加 match |
| Command | enum 加方法，或闭包 |
| Observer | 通道（mpsc）或事件队列 |
| Singleton | 静态变量加 OnceLock，或干脆不要全局状态 |
| Factory Method | 关联函数 new、From 或 Default |
| Template Method | trait 的默认方法 |
| Adapter | newtype 加 Deref，或直接实现 trait |
| Iterator | 标准库 Iterator trait |
| Visitor | enum 加迭代器，多数情况被 enum 取代 |

剩下的模式，价值不再依赖语言缺陷，而是表达真实的设计意图：newtype 表达「这个 u64 是用户 ID，不是订单号」；Builder 表达「这个类型字段多、构造有必填项」；RAII 表达「资源在离开作用域时释放」；类型状态（typestate）表达「状态转换有顺序」。本章要讲的就是这些在 Rust 里依然成立的手法，以及它们背后的编译期与运行期代价。

先看一个「策略坍缩成枚举」的最小例子：

```rust
enum Sort {
    Asc,
    Desc,
}

fn sort_by(mode: Sort, data: &mut [i32]) {
    match mode {
        Sort::Asc => data.sort(),
        Sort::Desc => data.sort_by(|a, b| b.cmp(a)),
    }
}

fn main() {
    let mut v = vec![3, 1, 2];
    sort_by(Sort::Desc, &mut v);
    println!("{:?}", v);
}
```

这段代码替代了「抽象基类加两个子类」的写法。Sort 是枚举，两个变体共享同一个函数，没有虚表，没有堆分配，编译器还能对 match 做穷尽性检查。

## 37.2 newtype：给类型加语义

第一个要留下的模式是 newtype：用一个只有一个字段的结构体包住已有类型，给它在类型系统里一个新的身份。动机来自最普通的错误。

```rust
#[derive(Debug, Clone, Copy, PartialEq)]
struct Meters(f64);

#[derive(Debug, Clone, Copy, PartialEq)]
struct Feet(f64);

impl Meters {
    fn to_feet(self) -> Feet {
        Feet(self.0 * 3.28084)
    }
}

fn main() {
    let d = Meters(100.0);
    println!("{:?} = {:?}", d, d.to_feet());
}
```

Meters 和 Feet 的底层都是 f64，但编译器把它们当作两个类型。任何把 Feet 传进只要 Meters 的函数都会报 E0308，这就是把错误提前到编译期。这种包装在运行期没有额外开销：编译器知道 newtype 的结构，生成的机器码与直接传 f64 相同，加 `#[repr(transparent)]` 更是从语言层面保证布局一致。

newtype 的第二个用途是绕过孤儿规则（orphan rule）。孤儿规则规定：只有当 trait 或类型至少有一个定义在当前 crate 时，才能为它写 impl。想给标准库的 Vec 实现标准库的 Display，两个都是外部的，编译器不允许。包一层本地类型，类型在本 crate 这个条件就满足了。

```rust
use std::fmt;

struct Hex(Vec<u8>);

impl fmt::Display for Hex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        Ok(())
    }
}

fn main() {
    let data = Hex(vec![0xde, 0xad, 0xbe, 0xef]);
    println!("{}", data);
}
```

这里不能写成 `impl fmt::Display for Vec<u8>`，即使那样更直接。原因就是孤儿规则：Display 和 Vec 都来自 std。newtype 是不引入包装层就无法回避时的标准解法。

newtype 不是免费的，有三处代价。第一，所有方法都要转发，字段访问从 `.0` 变成更深的层级；可以给它实现 Deref 减少样板，但 Deref 的本意是为智能指针提供自动解引用，把它当继承用会让方法解析难以预测。第二，与外部 crate 交互时可能需要额外的转换，序列化库通常提供 `#[serde(transparent)]` 之类的开关去掉包装层。第三，包得太多会让类型名冗长、API 难用。经验规则是：当两种值可能被混用、且混用会产生难以发现的错误时才包；为了看起来面向对象而给每个字段都包一层，是过度设计。

## 37.3 typestate：把状态机编码进类型

很多对象在生命周期里有明确的状态：TCP 连接有「未连接 / 已连接 / 已认证」，文件有「已打开 / 已关闭」，订单有「待支付 / 已支付 / 已发货」。在多数语言里，这些状态是运行期的一个字段，调用顺序错了只能在运行时报错，或者靠断言兜底。Rust 可以把它提到类型层：让每个状态是一个类型，把合法转换写成只对某个状态存在的方法，非法转换在类型层面根本不存在。

做法是把状态标记成零大小的类型，用类型参数把它们挂到主类型上，再用 PhantomData 占位：

```rust
use std::marker::PhantomData;

struct Closed;
struct Open;

struct Connection<State> {
    addr: String,
    _state: PhantomData<State>,
}

impl Connection<Closed> {
    fn connect(addr: &str) -> Connection<Open> {
        println!("连接到 {}", addr);
        Connection {
            addr: addr.to_string(),
            _state: PhantomData,
        }
    }
}

impl Connection<Open> {
    fn send(&self, msg: &str) {
        println!("发送到 {}: {}", self.addr, msg);
    }

    fn close(self) {
        println!("关闭 {}", self.addr);
    }
}

fn main() {
    let conn = Connection::connect("127.0.0.1:8080");
    conn.send("ping");
    conn.close();
}
```

`PhantomData<State>` 不占任何字节，它只是告诉编译器「这个结构体在类型上依赖 State」；状态标记本身也是零大小类型。所以整个 Connection 的布局只比一个 String 多一个空字段，实际上等价于只有 String。typestate 是纯编译期的约束：运行期付的钱是零。

它的边界也要说清。第一，状态类型不能携带运行期数据，别指望在 `Connection<State>` 里从 State 读出值；状态只是标签。第二，状态转换必须消费 `self` 并返回新类型，因此每次转换都会移动对象。对栈上的小对象没问题，对很大的对象要考虑改成 `&mut self` 加更复杂的类型参数。第三，状态类型会出现在公开 API 里，调用方被迫写 `Connection<Open>`；必要时用类型别名隐藏细节。

Builder 模式经常和 typestate 结合，用来保证必填字段。先看没有类型状态的版本：字段都是 `Option`，链式方法把值填进去，`build` 在运行期检查，缺字段就返回 `Err`。这个版本写起来顺手，但漏填字段要到运行时才知道。

把「填了没有」编码成两个状态类型，就能让编译器替你检查：

```rust
use std::marker::PhantomData;

struct Missing;
struct Present;

struct RequestBuilder<Url, Method> {
    url: Option<String>,
    method: Option<String>,
    _state: PhantomData<(Url, Method)>,
}

impl RequestBuilder<Missing, Missing> {
    fn new() -> Self {
        RequestBuilder {
            url: None,
            method: None,
            _state: PhantomData,
        }
    }
}

impl<M> RequestBuilder<Missing, M> {
    fn url(mut self, url: &str) -> RequestBuilder<Present, M> {
        self.url = Some(url.to_string());
        RequestBuilder {
            url: self.url,
            method: self.method,
            _state: PhantomData,
        }
    }
}

impl<U> RequestBuilder<U, Missing> {
    fn method(mut self, method: &str) -> RequestBuilder<U, Present> {
        self.method = Some(method.to_string());
        RequestBuilder {
            url: self.url,
            method: self.method,
            _state: PhantomData,
        }
    }
}

impl RequestBuilder<Present, Present> {
    fn build(self) -> String {
        format!("{} {}", self.method.unwrap(), self.url.unwrap())
    }
}

fn main() {
    let req = RequestBuilder::new()
        .url("/api/users")
        .method("POST")
        .build();
    println!("{}", req);
}
```

现在如果只调 `.url()` 就 `.build()`，编译器报 E0599：`build` 只在 `RequestBuilder<Present, Present>` 上存在，当前类型是 `RequestBuilder<Present, Missing>`，方法根本不存在。这正是想要的效果：把「必填项没填」从运行期错误变成「方法不存在」。

代价是代码量明显增加，状态参数会让类型签名变长，字段一多就膨胀。实践中的取舍是：字段少、必填项清晰时用运行期 `Result` 版；字段多且漏填代价高（配置、协议消息、状态机）时用 typestate。两条路都能用，区别是错误在编译期还是运行期暴露。

## 37.4 RAII 与守卫对象：把清理交给 Drop

RAII（Resource Acquisition Is Initialization）不是 GoF 里的模式，而是一种资源管理方式。核心只有一句话：把资源的释放写进析构函数，让作用域结束这个动作自动触发它。Rust 的析构函数是 `Drop::drop`，编译器保证它在值离开作用域时被调用，包括函数提前 `return`、`?` 传播错误，以及 panic 展开（unwind）的时候。

任何「必须被释放」的东西都适合包成守卫（guard）：文件句柄、锁、数据库事务、临时目录、计时器。下面是把清理写进 Drop 的最小例子：

```rust
struct ScopeGuard {
    name: &'static str,
}

impl ScopeGuard {
    fn new(name: &'static str) -> Self {
        println!("进入 {}", name);
        ScopeGuard { name }
    }
}

impl Drop for ScopeGuard {
    fn drop(&mut self) {
        println!("离开 {}", self.name);
    }
}

fn main() {
    let _outer = ScopeGuard::new("outer");
    {
        let _inner = ScopeGuard::new("inner");
    }
    println!("内层已经结束");
}
```

输出顺序是进入 outer、进入 inner、离开 inner、内层已经结束、离开 outer。Drop 的调用顺序与声明顺序相反，这是保证「后创建的先释放」的栈式规则。注意 `_inner` 不能写成单独的 `_`：`_` 表示立即丢弃，会让 Drop 在那一行就执行。

标准库里最常遇到的守卫是 `MutexGuard`。`Mutex::lock` 返回 `LockResult<MutexGuard<T>>`，guard 解引用到内部数据，离开作用域时自动解锁。它不能 clone，也不能把内部数据搬走，因为一旦这样做，锁的释放时机就与数据的生命周期脱钩，安全前提就没了。想提前解锁，标准写法是 `drop(guard)`：把 guard 值交给 `std::mem::drop`，函数返回时值被销毁，Drop 随之执行。

```rust
use std::sync::Mutex;

fn main() {
    let data = Mutex::new(0);
    let guard = data.lock().unwrap();
    println!("读到 {}", *guard);
    drop(guard);

    let mut guard = data.lock().unwrap();
    *guard += 1;
    println!("现在 {}", *guard);
}
```

这里有一个常见误解：不能用 `guard.drop()` 显式调用析构。`Drop::drop` 是编译器在值销毁时自动调用的钩子，语言层面禁止手动调用，否则同一份数据可能被析构两次。这样写会得到 E0040（explicit use of destructor method）。要提前释放，唯一正确的方式是 `drop(guard)`：它把值移动进去，再由正常的销毁流程触发 Drop。两个写法看着只差一个函数调用，语义却完全不同——前者是「再析构一次」，后者是「交出所有权，让作用域正常结束」。

还有一个更隐蔽的边界。如果类型带泛型参数，`Drop` 的实现必须对所有参数统一，不能只在 `T: Debug` 时析构；而且结构体一旦实现了 Drop，就不能再把字段单独移出去，否则报 E0509（cannot move out of type which implements Drop）。这两条限制解释了为什么很多资源管理类型把真正持有的字段藏起来，只暴露方法。

RAII 与借用检查器配合，得到一个比多数语言更硬的保证：资源在使用期间一定有效，且一定被释放。这也是 37.6 里通道方案优于回调列表的原因之一——守卫对象的生命周期由作用域管理，不需要运行时引用计数，也不会因为忘记调用 close 而泄漏。

## 37.5 策略模式：单态化还是 dyn

策略模式（Strategy）的意图是「把可互换的算法从使用它的上下文里独立出来」。Rust 里同一个意图有三种写法，性能与灵活性各不相同。

策略模式在 Rust 里有两种主要的分派方式。第一种是把策略写成泛型参数（静态分派），编译器为每个具体类型生成一份专用代码，这个过程叫单态化（monomorphization）。第二种是把它装进 trait 对象（动态分派），运行期通过虚表（vtable）找方法。两者可以共享同一套 trait 和实现，区别只在调用处：

```rust
trait Discount {
    fn apply(&self, price: f64) -> f64;
}

struct NoDiscount;
struct PercentOff(f64);

impl Discount for NoDiscount {
    fn apply(&self, price: f64) -> f64 {
        price
    }
}

impl Discount for PercentOff {
    fn apply(&self, price: f64) -> f64 {
        price * (1.0 - self.0)
    }
}

fn checkout<D: Discount>(strategy: &D, price: f64) -> f64 {
    strategy.apply(price)
}

fn main() {
    println!("{}", checkout(&NoDiscount, 100.0));
    println!("{}", checkout(&PercentOff(0.2), 100.0));

    let rules: Vec<Box<dyn Discount>> = vec![
        Box::new(NoDiscount),
        Box::new(PercentOff(0.1)),
        Box::new(PercentOff(0.05)),
    ];
    let price = rules.iter().fold(100.0, |p, r| r.apply(p));
    println!("{}", price);
}
```

`checkout` 会被实例化成两份机器码，每份里 `apply` 都是直接调用，可以被内联。代价是二进制体积随策略数量线性增长，编译时间也变长；如果策略集合在运行期才确定，或者要放进同一个容器，泛型就无能为力，只能走 trait 对象。

注意 `dyn Discount` 要求 trait 是对象安全的（object safe）：方法不能返回 `Self`、不能有泛型方法、不能要求 `Self: Sized`。不满足时编译器报 E0038，提示这个 trait 不能变成 trait 对象。对象安全的具体条件在第 16 章《trait（二）：关联类型、dyn 与对象安全》里完整讲过。

第三种是闭包。如果策略只是一段无状态的计算，直接存 `Box<dyn Fn(f64) -> f64>`，或者用泛型 `F: Fn(f64) -> f64`，反而更轻——不用为每个策略定义类型和 impl。判据是：策略是否需要携带数据、是否需要被命名和复用。需要就定义 trait，不需要就用闭包。

三者的选择可以压缩成一张表：

| 写法 | 分派时机 | 适合场景 | 代价 |
| --- | --- | --- | --- |
| 泛型 D: Trait | 编译期 | 策略集合固定、在热路径上 | 代码膨胀、编译变慢 |
| dyn Trait | 运行期 | 策略在运行期选择、要装进同一容器 | 虚表间接、难以内联 |
| 闭包 Fn | 编译期或运行期 | 无状态、一次性算法 | 类型签名变复杂 |

经验规则是：API 的入参优先泛型，存储和异构集合用 dyn。也就是说，`fn run<S: Solver>(s: &S)` 给函数使用者最大的优化空间；而 `Vec<Box<dyn Solver>>` 用来保存一批在运行期决定的插件。37.8 会从代码生成和内存布局的角度把这两种分派的差别讲透。

## 37.6 观察者与命令：通道和枚举替代回调列表

观察者模式在 GoF 里是这样写的：主题（Subject）持有一份观察者列表，状态变化时遍历列表调用每个观察者的 `update`。把这个结构直译到 Rust 会遇到两个问题。第一，列表里要放不同类型的观察者，只能写 `Vec<Box<dyn Observer>>`，每次通知都是虚调用。第二，也是更致命的，`notify(&mut self)` 在遍历自己的列表时，观察者常常需要回头调用主题的方法，于是同一份数据被要求同时可变借用，借用检查器直接报 E0502。绕过它只能用 `RefCell` 把检查推迟到运行期，然后在某个时刻炸出 `already borrowed`。

Rust 里更顺手的做法是反转依赖：主题不持有观察者，只持有一个发送端，状态变化时把事件发出去；观察者各自从通道接收。事件的所有权随消息转移，主题不需要知道谁在监听，也不需要在可变借用期间调用外部代码。

```rust
use std::sync::mpsc;
use std::thread;

#[derive(Debug)]
enum Event {
    Started(String),
    Finished(u32),
}

fn main() {
    let (tx, rx) = mpsc::channel();

    let handle = thread::spawn(move || {
        for event in rx {
            match event {
                Event::Started(name) => println!("开始 {}", name),
                Event::Finished(code) => println!("结束 {}", code),
            }
        }
        println!("通道关闭");
    });

    let tx2 = tx.clone();
    tx.send(Event::Started(String::from("job-1"))).unwrap();
    tx2.send(Event::Finished(0)).unwrap();
    drop(tx);
    drop(tx2);

    handle.join().unwrap();
}
```

这段代码用了 `std::sync::mpsc`，也就是第 23 章《消息传递与通道》讲过的多生产者单消费者通道。它没有共享状态，因此不需要 `RefCell`，也没有运行期借用冲突。观察者的「注册」变成把 Sender 交给别人，「注销」变成 drop 掉对应的 Sender。代价是事件变成异步的：发送方不能立刻得到返回值。如果需要同步回应，用 oneshot 通道，或者干脆直接调用函数。

命令模式（Command）的直译是定义一个 `Command` trait，每个操作是一个实现它的结构体，调用方持有 `Box<dyn Command>` 并调用 `execute`。它的意图是「把操作变成一等值，可以排队、记录、撤销」。在 Rust 里，如果命令集合是封闭的（你知道所有可能的操作），用枚举比 trait 对象好得多——这正是第 9 章《枚举与模式匹配》的强项。

```rust
#[derive(Debug)]
enum Command {
    Deposit(u32),
    Withdraw(u32),
    Reset,
}

#[derive(Debug, Default)]
struct Account {
    balance: u32,
}

impl Account {
    fn execute(&mut self, cmd: &Command) -> Result<(), String> {
        match cmd {
            Command::Deposit(amount) => {
                self.balance += *amount;
                Ok(())
            }
            Command::Withdraw(amount) => {
                if *amount > self.balance {
                    return Err(String::from("余额不足"));
                }
                self.balance -= *amount;
                Ok(())
            }
            Command::Reset => {
                self.balance = 0;
                Ok(())
            }
        }
    }
}

fn main() {
    let mut account = Account::default();
    let commands = vec![Command::Deposit(100), Command::Withdraw(30)];
    for cmd in &commands {
        account.execute(cmd).unwrap();
        println!("{:?} -> {}", cmd, account.balance);
    }
}
```

枚举版的优势是实打实的：`match` 是穷尽的，加了新命令没处理，编译器报 E0004（non-exhaustive patterns）；命令可以被 Debug、被序列化、被写进日志；没有堆分配和虚表。如果命令集合必须对下游开放（第三方插件要注册自己的命令），才需要退回 trait 对象，并且通常配合注册表。这两种情形要分清：封闭集合用枚举，开放集合用 dyn。

命令模式还有一个变体值得记住：把「要做的事」存成闭包 `Vec<Box<dyn FnOnce()>>`，延迟执行。它和枚举的取舍，与 37.5 里闭包策略的取舍完全一致。

## 37.7 扩展 trait 与 sealed trait

扩展 trait（extension trait）解决的是「想给一个不属于自己的类型加方法」。比如你想给 `&str` 加一个 `shout` 方法，但 `str` 是标准库类型，不能改它的 impl。办法是在自己 crate 里定义一个 trait，为外部类型实现它。只要这个 trait 在使用点的作用域内，方法就能调用：

```rust
trait StrExt {
    fn shout(&self) -> String;
}

impl StrExt for str {
    fn shout(&self) -> String {
        format!("{}!", self.to_uppercase())
    }
}

fn main() {
    println!("{}", "hello".shout());
}
```

扩展 trait 是零成本的：方法是静态解析的，没有 vtable；但它引入了导入依赖——如果调用方没有 `use` 这个 trait，方法解析不到，会得到 E0599（no method found）。这也是为什么标准库的 `Iterator`、`Read` 等 trait 必须显式导入。社区惯例是给扩展 trait 起 `...Ext` 后缀，避免和类型自带的方法重名。

还有一个更微妙的要求：扩展 trait 与目标类型不能产生冲突。如果你自己的类型已经有一个同名方法，解析规则会优先选固有方法（inherent method），扩展 trait 的方法被遮蔽。这也是为什么不要给 `Option`、`Result` 加和标准库同名的扩展方法。

sealed trait（封闭 trait）是反过来的需求：你定义了一个 trait 给外面用，但不希望外面实现。典型场景是库的 API 既要暴露一个 trait 作为约束，又要保证所有实现都在库内部，这样将来加方法或改语义不会破坏下游。做法是给公开 trait 加一个私有模块里的 supertrait：

```rust
mod private {
    pub trait Sealed {}
}

pub trait Shape: private::Sealed {
    fn area(&self) -> f64;
}

pub struct Circle {
    radius: f64,
}

impl private::Sealed for Circle {}

impl Shape for Circle {
    fn area(&self) -> f64 {
        std::f64::consts::PI * self.radius * self.radius
    }
}

fn main() {
    let c = Circle { radius: 2.0 };
    println!("{}", c.area());
}
```

关键在 `private` 模块的可见性：模块本身没有 `pub`，外面无法命名 `private::Sealed` 这个路径，因此无法为外部类型实现它。`Shape: private::Sealed` 这个 supertrait 约束照常生效，编译器知道所有实现 `Shape` 的类型都实现了 `Sealed`。外部 crate 里写 `impl Shape for MyShape` 会得到 E0277，提示 `MyShape: private::Sealed` 不满足；而由于 `Sealed` 是私有的，这个错误实际上无法被修复——这正是「封闭」的含义。

sealed trait 的代价是：你确实堵死了扩展。用不用它取决于「trait 的语义是否承诺稳定」。对封闭的实现集合（比如「所有能在本库中注册的驱动」）用 sealed；对故意留给用户的扩展点（比如自定义序列化格式）不要用 sealed。API 设计上的这类取舍，第 27 章《错误处理与 API 设计》有更系统的讨论。

## 37.8 深入：单态化、vtable 与胖指针

这一节回答本章最实际的问题：这些模式在编译产物和内存里到底长什么样。理解了它，前面「该用泛型还是 dyn」的取舍就不再是经验口诀，而是可以自己推导的结论。

先看单态化。编译器遇到 `checkout::<NoDiscount>` 和 `checkout::<PercentOff>`，会为每个具体类型各生成一份函数体，把 `D::apply` 替换成对应实现。这个过程发生在 MIR 到 LLVM IR 的转换阶段，每个实例都是独立的、没有泛型参数的普通函数。结果是：泛型代码在运行期和手写具体类型一样快，甚至因为看见了具体类型而能更好地内联和向量化；代价是每个实例一份代码。一个被十个类型调用的泛型函数会产生十份机器码，这是 Rust 二进制比等价 C 程序大的常见原因之一，也是编译时间随泛型使用量增长的原因。

trait 对象则完全相反。`Box<dyn Discount>` 是一个胖指针（fat pointer）：第一个字是指向实际数据的指针，第二个字是指向虚表的指针。虚表里按固定顺序放着析构函数、大小、对齐和每个方法的函数指针。

```rust
use std::mem::size_of;

trait Discount {
    fn apply(&self, price: f64) -> f64;
}

struct NoDiscount;

impl Discount for NoDiscount {
    fn apply(&self, price: f64) -> f64 {
        price
    }
}

fn main() {
    println!("Box<NoDiscount>   = {} 字节", size_of::<Box<NoDiscount>>());
    println!("Box<dyn Discount> = {} 字节", size_of::<Box<dyn Discount>>());
    println!("&dyn Discount     = {} 字节", size_of::<&dyn Discount>());
}
```

在 64 位机器上输出是 8、16、16，多出来的第二个字就是虚表指针。这和「引用是一个字」并不矛盾：`&T` 是一个字，`&dyn Trait` 是两个，因为 `dyn Trait` 是未定大小类型（unsized），必须额外记录它的虚表。切片 `&[T]` 同样是两个字段（指针加长度），原理相同：编译期不知道大小的类型，引用时要补上缺失的信息。

typestate 的 PhantomData 则是另一个极端：它是零大小类型（ZST）。下面这段代码验证，加了一个状态参数后类型大小完全没有变化。

```rust
use std::marker::PhantomData;
use std::mem::size_of;

struct Closed;
struct Open;

struct Connection<State> {
    addr: String,
    _state: PhantomData<State>,
}

fn main() {
    println!("String             = {} 字节", size_of::<String>());
    println!("Connection<Closed> = {} 字节", size_of::<Connection<Closed>>());
    println!("Connection<Open>   = {} 字节", size_of::<Connection<Open>>());
}
```

三行都是 24。`PhantomData<State>` 大小为 0，不引入填充；它唯一的用途是在类型层面声明「我有 State 这个参数」，让编译器据此选择 impl 块、检查所有权，并在需要时施加 auto trait 约束（比如 `PhantomData<Rc<()>>` 会让类型不满足 Send）。所以 typestate 是纯编译期开销，运行期一分钱不花。

把两者放在一起，选择判据就很清楚了。泛型（单态化）适合调用点类型已知、需要内联、策略数量有限、在热路径上的场合。dyn 适合策略在运行期才确定、需要把不同类型放进同一个容器、希望控制二进制体积或缩短编译时间的场合。去虚化（devirtualization）有时能救回一点 dyn 的性能——当编译器能静态看到某个 trait 对象的具体类型时，可能把虚调用内联掉；但这不可依赖，尤其当对象来自其他 crate 或运行时输入时。

最后补一个容易忽略的点：`Box<dyn Trait>` 的析构也要查虚表。释放一个 trait 对象时，编译器不能仅凭 `Box<dyn Trait>` 就知道该调用哪个类型的析构函数，只能通过虚表里的第一项 `drop_in_place` 去找。这解释了为什么对象安全的 trait 一定要能放进虚表，也解释了为什么 `dyn Trait` 的 drop 会比具体类型的 drop 稍慢。

## 37.9 反模式：别把继承式设计硬套进 Rust

学完前几节，一个自然的错误倾向是：用 Rust 把 Java 或 C# 里的类层次结构照搬过来。基类、子类、共享可变字段、虚方法，这套结构在 Rust 里要么写不出来，要么要付出很大代价。这一节把最常见的三种硬套法列出来。

第一种是「用 trait 当抽象基类，把共享字段放进每个实现」。Java 的基类可以持有字段，Rust 的 trait 不能。于是有人写成 `Rc<RefCell<Base>>`，让每个子类型都持有一份共享状态。结果是：借用在运行期检查，`borrow_mut` 冲突变成 panic；引用计数带来额外的计数维护；更糟的是，一旦需要「在一个 borrow 还没结束时再借」，程序会在最不该崩的地方崩掉。正确的替代是组合加 trait：把共享数据放在一个独立结构体里，让各实现组合它（持有或借用），而不是继承它。

第二种是「到处 `Box<dyn Trait>`」。把每个字段、每个函数参数都写成 trait 对象，短期看很灵活，长期有三个后果：热路径上失去内联和单态化优化；错误信息从「类型不匹配」退化成「trait 对象不满足某个方法」；二进制里到处是虚表，性能分析变得困难。判据仍然是 37.8 的结论：存储和异构集合用 dyn，函数入参和热路径用泛型。

第三种是「为了复用代码而制造继承链」。Rust 里用 trait 默认方法可以做一部分代码复用，但默认方法不能访问实现类型的私有字段。当复用需求出现时，优先把公共逻辑抽成自由函数或独立类型，再让各个实现调用它，而不是把逻辑往上堆进越来越庞大的 trait。

最后给一个可执行的自检清单。如果你发现自己在做下面任何一件事，先停下来重新设计：

- 为一个类型同时实现三个 trait，而它们的方法互相调用、又都要求 `&mut self`；
- 在结构体里出现 `Rc<RefCell<...>>` 的嵌套，且不确定哪一层正在借用；
- 用 `Box<dyn Trait>` 表示一个内部只有两三个变体的选择；
- 为了让一个测试通过，把字段临时改成 `pub`。

这些信号说明你在用共享可变状态和继承的思路写 Rust，而不是用所有权和枚举。

## 典型错误案例

### 案例 1：泛型 impl 与具体 impl 重叠——E0119

```rust,editable
trait Describe {
    fn describe(&self) -> String;
}

impl<T: std::fmt::Debug> Describe for T {
    fn describe(&self) -> String {
        format!("{:?}", self)
    }
}

#[derive(Debug)]
struct User {
    name: String,
}

impl Describe for User {
    fn describe(&self) -> String {
        self.name.clone()
    }
}

fn main() {
    let u = User {
        name: String::from("ana"),
    };
    println!("{}", u.describe());
}
```

```text
error[E0119]: conflicting implementations of trait `Describe` for type `User`
  --> src/main.rs:16:1
   |
 5 | impl<T: std::fmt::Debug> Describe for T {
   | --------------------------------------- first implementation here
...
16 | impl Describe for User {
   | ^^^^^^^^^^^^^^^^^^^^^^ conflicting implementation for `User`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0119`.
```

**原因**：第一个 impl 是为所有满足 `T: Debug` 的类型实现的泛型 impl，第二个 impl 是为具体类型 `User` 实现的。由于 `User` 派生了 `Debug`，两个 impl 同时适用于 `User`，一致性规则（coherence）不允许同一个类型对同一个 trait 有两份实现，所以编译器报冲突。注意这和无辜的孤儿规则不是一回事：孤儿规则管的是「能不能为外部类型实现外部 trait」，报的是 E0117；这里管的是「两份 impl 是否重叠」，报的是 E0119。

**修复**：

```rust,editable
trait Describe {
    fn describe(&self) -> String;
}

#[derive(Debug)]
struct User {
    name: String,
}

impl Describe for User {
    fn describe(&self) -> String {
        self.name.clone()
    }
}

struct Product {
    sku: String,
}

impl Describe for Product {
    fn describe(&self) -> String {
        format!("商品 {}", self.sku)
    }
}

fn main() {
    let u = User {
        name: String::from("ana"),
    };
    let p = Product {
        sku: String::from("A-1"),
    };
    println!("{} / {}", u.describe(), p.describe());
}
```

去掉泛型 blanket impl，改为每个类型各写一份具体实现。如果确实需要「所有 Debug 类型都能 describe」的通用行为，应该另起一个 trait（例如 `DebugDescribe`），不要和具体类型的 impl 挤在同一个 trait 上。

### 案例 2：trait 实现漏掉必需方法——E0046

```rust,editable
trait Plugin {
    fn name(&self) -> &'static str;
    fn run(&self);
}

struct Logger;

impl Plugin for Logger {
    fn name(&self) -> &'static str {
        "logger"
    }
}

fn main() {
    let p = Logger;
    println!("{}", p.name());
}
```

```text
error[E0046]: not all trait items implemented, missing: `run`
 --> src/main.rs:8:1
  |
3 |     fn run(&self);
  |     -------------- `run` from trait
...
8 | impl Plugin for Logger {
  | ^^^^^^^^^^^^^^^^^^^^^^ missing `run` in implementation

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0046`.
```

**原因**：trait `Plugin` 声明了两个没有默认实现的方法，`impl` 里只提供了 `name`，`run` 缺失。编译器要求每个没有默认体的 trait 方法都在实现里出现，否则类型不完整，报 E0046。这也是 trait 与抽象类的一个区别：trait 的方法默认是「必须实现」，要提供默认行为必须显式写方法体。

**修复**：

```rust,editable
trait Plugin {
    fn name(&self) -> &'static str;
    fn run(&self);
}

struct Logger;

impl Plugin for Logger {
    fn name(&self) -> &'static str {
        "logger"
    }

    fn run(&self) {
        println!("{} 正在运行", self.name());
    }
}

fn main() {
    Logger.run();
}
```

补上 `run` 即可。如果多数实现的行为相同，也可以在 trait 里给 `run` 写默认方法体，实现方只在需要覆盖时才写。

### 案例 3：sealed trait 的私有 supertrait 没实现——E0277

```rust,editable
mod sealed {
    pub trait Sealed {}
}

pub trait Api: sealed::Sealed {
    fn call(&self) -> u32;
}

struct Impl;

impl Api for Impl {
    fn call(&self) -> u32 {
        1
    }
}

fn main() {
    let x = Impl;
    println!("{}", x.call());
}
```

```text
error[E0277]: the trait bound `Impl: Sealed` is not satisfied
  --> src/main.rs:11:14
   |
11 | impl Api for Impl {
   |              ^^^^ unsatisfied trait bound
   |
help: the trait `Sealed` is not implemented for `Impl`
  --> src/main.rs:9:1
   |
 9 | struct Impl;
   | ^^^^^^^^^^^
help: this trait has no implementations, consider adding one
  --> src/main.rs:2:5
   |
 2 |     pub trait Sealed {}
   |     ^^^^^^^^^^^^^^^^
note: required by a bound in `Api`
  --> src/main.rs:5:16
   |
 5 | pub trait Api: sealed::Sealed {
   |                ^^^^^^^^^^^^^^ required by this bound in `Api`

error: aborting due to 2 previous errors
```

**原因**：`Api` 的 supertrait 是 `sealed::Sealed`，任何类型想实现 `Api` 都必须先实现 `Sealed`。`Impl` 没有这一步，于是 trait bound 不满足，报 E0277；调用 `call` 时还会再报一次，因为方法本身也带着这个约束。如果这段代码来自外部 crate，那么 `Sealed` 所在的模块是私有的，外部连路径都写不出来，这个错误无法修复——这正是 sealed trait 想要的效果。

**修复**：

```rust,editable
mod private {
    pub trait Sealed {}
}

pub trait Api: private::Sealed {
    fn call(&self) -> u32;
}

struct Impl;

impl private::Sealed for Impl {}

impl Api for Impl {
    fn call(&self) -> u32 {
        1
    }
}

fn main() {
    println!("{}", Impl.call());
}
```

在本 crate 内部显式实现 `private::Sealed`，就能通过 supertrait 检查。要强调的边界是：能这样修，前提是代码和 `private` 模块在同一个 crate 里；跨 crate 时这条路被可见性堵死，封闭才成立。

### 案例 4：newtype 忘记包裹返回值——E0308

```rust,editable
struct Meters(f64);

fn marathon_length() -> Meters {
    42195.0
}

fn main() {
    let d = marathon_length();
    println!("{}", d.0);
}
```

```text
error[E0308]: mismatched types
 --> src/main.rs:4:5
  |
3 | fn marathon_length() -> Meters {
  |                         ------ expected `Meters` because of return type
4 |     42195.0
  |     ^^^^^^^ expected `Meters`, found floating-point number
  |
help: try wrapping the expression in `Meters`
  |
4 |     Meters(42195.0)
  |     +++++++       +
```

**原因**：函数的返回类型写的是 `Meters`，函数体最后一行却是 `f64` 字面量。newtype 不会自动把内层类型转换成外层类型，构造一个 `Meters` 必须显式调用元组结构体的构造函数。编译器知道该怎么做，甚至在 help 里给出了补全建议，但它不会替你做这个决定，因为「包哪一层」是语义问题。

**修复**：

```rust,editable
struct Meters(f64);

fn marathon_length() -> Meters {
    Meters(42195.0)
}

fn main() {
    let d = marathon_length();
    println!("{}", d.0);
}
```

把返回值包进 `Meters(...)`。如果这个转换在代码里反复出现，可以给 newtype 实现 `From<f64>`，再在返回处用 `Meters::from(42195.0)` 或 `.into()`；但要注意 `From` 会让「任何 f64 都能变成 Meters」重新合法化，可能抵消 newtype 的部分保护作用。

### 案例 5：typestate 未填必填字段就 build——E0599

```rust,editable
use std::marker::PhantomData;

struct Missing;
struct Present;

struct Builder<State> {
    name: Option<String>,
    _state: PhantomData<State>,
}

impl Builder<Missing> {
    fn new() -> Self {
        Builder {
            name: None,
            _state: PhantomData,
        }
    }

    fn name(mut self, name: &str) -> Builder<Present> {
        self.name = Some(name.to_string());
        Builder {
            name: self.name,
            _state: PhantomData,
        }
    }
}

impl Builder<Present> {
    fn build(self) -> String {
        self.name.unwrap_or_default()
    }
}

fn main() {
    let greeting = Builder::new().build();
    println!("{}", greeting);
}
```

```text
error[E0599]: no method named `build` found for struct `Builder<Missing>` in the current scope
  --> src/main.rs:35:35
   |
 6 | struct Builder<State> {
   | --------------------- method `build` not found for this struct
...
35 |     let greeting = Builder::new().build();
   |                                   ^^^^^ method not found in `Builder<Missing>`
   |
   = note: the method was found for `Builder<Present>`
```

**原因**：`build` 定义在 `impl Builder<Present>` 上，而 `Builder::new()` 返回的是 `Builder<Missing>`。类型参数不匹配，方法解析失败，报 E0599；编译器的 note 已经指出了正确类型。这不是缺陷，而是 typestate 的核心机制：没经过 `name` 这一步，`build` 在类型上就不存在，漏填字段从运行期错误变成了编译错误。

**修复**：

```rust,editable
use std::marker::PhantomData;

struct Missing;
struct Present;

struct Builder<State> {
    name: Option<String>,
    _state: PhantomData<State>,
}

impl Builder<Missing> {
    fn new() -> Self {
        Builder {
            name: None,
            _state: PhantomData,
        }
    }

    fn name(mut self, name: &str) -> Builder<Present> {
        self.name = Some(name.to_string());
        Builder {
            name: self.name,
            _state: PhantomData,
        }
    }
}

impl Builder<Present> {
    fn build(self) -> String {
        self.name.unwrap_or_default()
    }
}

fn main() {
    let greeting = Builder::new().name("ana").build();
    println!("{}", greeting);
}
```

先调用 `name`，把 `Builder<Missing>` 转成 `Builder<Present>`，`build` 才可见。这就是 37.3 里说的「把必填项编码进类型」：不是运行时检查，而是让缺字段的调用在编译期根本写不出来。代价是每个必填字段都要有一对状态类型和一个只在该状态存在的方法，字段多时样板会明显增加。

## 动手练习

1. 定义 `Celsius` 与 `Fahrenheit` 两个 newtype，只通过方法做换算。写一个只接收 `Celsius` 的函数，故意传入 `Fahrenheit`，记录编译器给出的错误码；然后把参数改成 `f64` 再试一次，说明为什么 newtype 比注释可靠。

2. 用 typestate 实现一个三态连接：`Disconnected`、`Connected`、`Authenticated`。`connect` 只在 `Disconnected` 上存在，`login` 只在 `Connected` 上存在，`send` 只在 `Authenticated` 上存在。尝试在 `Disconnected` 上调用 `send`，记录错误码，并解释为什么这比在运行期返回 `Err` 更强。

3. 给 37.3 的 `RequestBuilder` 增加第三个必填字段 `headers`，观察类型参数、impl 块和状态类型如何增长。然后写一个 `Result` 版本，比较两者的代码行数、类型签名长度，以及漏填字段时的报错位置。

4. 把 37.6 的观察者例子改成两个订阅者。由于 `mpsc` 是单消费者，你需要两条通道，或者改用广播方案。完成后回答：主题为什么不需要 `RefCell`？如果坚持用回调列表，两个订阅者同时回调主题会发生什么？

5. 在同一个 crate 里写一个扩展 trait（例如给 `String` 加 `words` 方法）和一个 sealed trait。用 `std::mem::size_of` 打印 `Box<dyn Trait>`、`&dyn Trait` 与 `PhantomData<State>` 的大小，解释 16、16、0 这三个数字分别来自哪里。

## 小结

- GoF 的多数模式是语言缺口的补丁。Rust 的枚举、闭包、trait 和所有权直接支持其中一些常见做法，剩下的模式表达的是与语言无关的设计意图。
- newtype 在运行期零成本，用来表达单位与语义，也用来绕过孤儿规则为外部类型实现外部 trait；代价是转发样板，以及把 Deref 当继承用的风险。
- typestate 用类型参数和 PhantomData 把状态编码进类型，非法状态转换在编译期不存在；转换消费 `self`，状态类型会出现在公开 API 里。
- Builder 的必填字段可以在运行期用 `Result` 检查，也可以在编译期用 typestate 检查。后者更安全，样板也更多，按漏填的代价取舍。
- RAII 把清理放进 `Drop`。提前释放用 `drop(guard)`；`guard.drop()` 是显式调用析构，报 E0040。
- 策略模式有三种写法：泛型单态化、trait 对象、闭包。函数入参优先泛型，存储与异构集合用 dyn，无状态算法用闭包。
- 观察者用通道替代回调列表，避开共享可变借用；命令用枚举加 `execute`，封闭集合用枚举，开放集合才用 trait 对象。
- 扩展 trait 给外部类型加方法，零成本但需要在使用点导入；sealed trait 用私有 supertrait 封死外部实现，代价是放弃扩展性。
- 从编译产物看：单态化为每个具体类型生成一份机器码；trait 对象是两字胖指针加虚表；PhantomData 是零大小类型，typestate 不占运行期空间。
- 反模式是把继承式设计硬套进来：遍地 `Rc<RefCell<...>>`、遍地 `Box<dyn Trait>`、为复用制造继承链。替代方案是组合、枚举和泛型。










