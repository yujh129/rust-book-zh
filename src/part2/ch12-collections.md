# 第 12 章 常用集合：Vec、String 与 HashMap

> **本章目标**
> - 会用 `Vec<T>` 保存可变长序列，理解 `len` 与 `capacity` 的区别，能用 `with_capacity` 预留空间。
> - 能区分 `String`、`&str` 与 `str`，知道 `push_str`、`+` 和 `format!` 各自在所有权上做了什么。
> - 会用 `HashMap` 的 `entry` API 完成计数与更新，能按需选择 `BTreeMap`、`HashSet` 与 `VecDeque`。
> - 能判断遍历集合时增删为什么会触发借用冲突，并写出四种替代写法。
> - 能从内存布局、哈希函数和 UTF-8 编码三个层面解释上述规则。

数组的长度是类型的一部分：`[i32; 3]` 与 `[i32; 4]` 是两个不同的类型，编译器在编译期就知道它们各占多少字节。这条规则让数组可以放在栈上、可以零成本地按索引访问，也让它无法表达「运行时才知道有多长」的数据。

真实程序里的数据几乎都是运行时才知道规模：用户输入的一行文字、日志文件里的若干行、一张表格的所有记录。Rust 在标准库里提供了三种最常用的容器：`Vec<T>` 存一串同类型的值，`String` 存一段 UTF-8 文本，`HashMap<K, V>` 按键存值。它们都在堆上分配缓冲区，栈上只留一个固定大小的描述符，因此长度可以随程序运行而改变。

容器把「数据放在哪」和「谁负责释放」绑在一起：缓冲区归容器所有，容器离开作用域时释放。这条所有权规则让动态数据不需要垃圾回收，也让容器的用法处处受借用检查器约束。你会在这一章里反复看到同一个模式：读取很容易，增长和删除要求独占。

本章先分别讲清三种容器的用法与代价，再讨论遍历时修改为什么会编译失败，最后深入到内存布局、哈希函数和 UTF-8 编码，解释这些规则背后的原因。

## 12.1 `Vec<T>`：可增长的数组

`Vec<T>` 是一个可以增长的数组，元素在内存里连续排列。连续带来两个好处：按索引访问是一次指针加法；顺序遍历对 CPU 缓存友好。代价是增长时可能要把整块数据搬到新地址。

```rust
fn main() {
    let mut v: Vec<i32> = Vec::new();
    v.push(10);
    v.push(20);
    v.push(30);

    println!("len = {}", v.len());
    println!("v[1] = {}", v[1]);
    println!("first = {:?}", v.first());
    println!("last = {:?}", v.last());

    match v.get(10) {
        Some(x) => println!("v[10] = {}", x),
        None => println!("索引 10 越界，get 返回 None"),
    }

    for x in &mut v {
        *x *= 2;
    }
    println!("{:?}", v);
}
```

两种按位置取值的写法要分清。`v[1]` 的返回类型是 `i32`，但它背后是 `Index` trait 返回的引用，越界时直接 panic；`v.get(1)` 返回 `Option<&i32>`，越界时得到 `None`，调用方必须处理。能用 `get` 的地方优先用 `get`：输入来自用户或文件时，越界是正常情况，不是程序缺陷。

`len()` 是当前元素个数，`capacity()` 是不重新分配的前提下最多能放多少个元素。二者相等并不总是成立，下面这段程序把每次增长后的数字打印出来。

```rust
fn main() {
    let mut v = Vec::new();
    for i in 0..9 {
        v.push(i);
        println!("len = {}, capacity = {}", v.len(), v.capacity());
    }
}
```

```text
len = 1, capacity = 4
len = 2, capacity = 4
len = 3, capacity = 4
len = 4, capacity = 4
len = 5, capacity = 8
len = 6, capacity = 8
len = 7, capacity = 8
len = 8, capacity = 8
len = 9, capacity = 16
```

第一次 `push` 时容量就是 4，而不是 1；第 5 个元素放不下时容量变成 8。标准库的实现策略是至少翻倍，但具体数字不是语言保证，换一个版本可能不同。重要的是摊还代价：单次 `push` 偶尔要复制全部元素，摊到每次操作上仍是 O(1)。如果你已经知道大概要放多少元素，`Vec::with_capacity(n)` 可以一次性申请够，避免多次复制。

删除元素的三个方法语义不同。`pop()` 从尾部取出一个元素，返回 `Option<T>`，O(1)；`remove(i)` 删除指定位置并把后面的元素整体前移，O(n)；`swap_remove(i)` 把最后一个元素移到 `i` 处再缩短长度，O(1)，代价是顺序被打乱。`retain` 按条件保留元素，只遍历一次，适合批量过滤。

关于所有权，`Vec<T>` 与数组一样按值传递。`for x in &v` 借用整个容器，循环结束后仍可使用 `v`；`for x in v` 把容器移动进循环，此后 `v` 不能再使用。`v[0]` 在值位置使用时会尝试把元素移出容器，对 `String` 这类非 `Copy` 类型会直接编译失败——这两点分别对应后面的案例 2 与案例 4。

## 12.2 String：带 UTF-8 不变式的字节缓冲区

Rust 的文本类型有三个名字，职责不同。

| 类型 | 是否拥有缓冲区 | 大小（64 位） | 典型用途 |
| --- | --- | --- | --- |
| `String` | 是 | 24 字节 | 需要增长或修改的文本 |
| `&str` | 否，借用他人 | 16 字节 | 只读参数、切片 |
| `str` | 不适用 | 不定长 | 只能通过引用使用 |

`str` 是未定长类型（unsized），编译器不知道它有多长，所以不能直接放在变量里；`&str` 是一个胖指针（fat pointer），包含地址和字节长度；`String` 拥有堆上的缓冲区，可以增长。三者都保证内部的字节是合法的 UTF-8，这条不变式（invariant）是安全使用文本的前提。

创建与拼接的方式不止一种，所有权代价也不同。

```rust
fn main() {
    let mut s = String::from("hello");
    s.push_str(", 世界");
    s.push('!');
    println!("{}", s);
    println!("字节数 = {}，字符数 = {}", s.len(), s.chars().count());

    let t = String::from("Rust");
    let u = s + &t;
    println!("{}", u);

    let v = format!("{}-{}", t, u);
    println!("{}", v);
}
```

`push_str` 把一段 `&str` 追加到末尾，`push` 追加单个 `char`，二者都在原缓冲区上修改，容量够时不会分配新内存。`+` 的签名等价于 `fn add(self, other: &str) -> String`：左操作数按值接收，因此 `s + &t` 会移动 `s`，之后不能再使用 `s`；右操作数只借用。`format!` 借用所有参数并返回新的 `String`，可读性更好，代价是一次新的分配。链式写法 `a + &b + &c` 合法，但第一个 `+` 已经把 `a` 移走。

文本可以切片，但切片的单位是字节，不是字符。

```rust
fn main() {
    let s = String::from("你好，世界");
    println!("{}", &s[0..6]);
    println!("{}", &s[6..9]);
    println!("{}", &s[9..]);
}
```

每个汉字在 UTF-8 里占 3 个字节，所以 `0..6` 正好是两个汉字，`6..9` 是中文逗号，`9..` 是剩下的两个字。范围的两端必须落在字符边界上；写 `&s[0..1]` 能通过编译，但运行时会 panic，信息里写明 `end byte index 1 is not a char boundary; it is inside '你' (bytes 0..3 of string)`。这也是 `s[0]` 被编译器拒绝的原因：按字符取第 n 个需要从头扫描，无法做到 O(1)，语言干脆不提供整数索引。

按字符遍历与按字节遍历是两条不同的路径。

```rust
fn main() {
    let s = String::from("héllo");
    for c in s.chars() {
        print!("[{}]", c);
    }
    println!();
    for b in s.bytes() {
        print!("{} ", b);
    }
    println!();
    println!("chars = {}, bytes = {}", s.chars().count(), s.len());
}
```

```text
[h][é][l][l][o]
104 195 169 108 108 111
chars = 5, bytes = 6
```

`é` 在 UTF-8 里占两个字节，所以字符数是 5，字节数是 6。`chars()` 逐个解码 Unicode 标量值，`bytes()` 逐个给出原始字节；`len()` 返回字节数，永远不要把它当成字符数。需要频繁按序号取字符时，先 `collect::<Vec<char>>()`，把 O(n) 的查找换成一次 O(n) 的转换加 O(1) 的索引。

函数参数用 `&str` 比用 `&String` 更通用：`&String` 能自动解引用成 `&str`，字符串字面量却无法变成 `&String`。

```rust
fn shout(text: &str) -> String {
    format!("{}!", text)
}

fn main() {
    let owned = String::from("hi");
    println!("{}", shout(&owned));
    println!("{}", shout("literal"));
}
```

## 12.3 HashMap：按键查找

`HashMap<K, V>` 存的是键值对，按键查找的平均代价是 O(1)。它要求键类型实现 `Eq` 与 `Hash`：前者判断两个键是否相等，后者把键映射成一个整数，决定放进哪个桶。`String`、`&str`、整数和元组都满足这两个约束，含 `f64` 字段的结构体不行：`f64` 没有实现 `Eq` 与 `Hash`，也无法用于要求 `Ord` 的 `BTreeMap`。

```rust
use std::collections::HashMap;

fn main() {
    let mut scores: HashMap<String, u32> = HashMap::new();
    scores.insert(String::from("蓝队"), 10);
    scores.insert(String::from("红队"), 20);
    scores.insert(String::from("蓝队"), 30);

    println!("蓝队 = {:?}", scores.get("蓝队"));
    println!("黄队 = {:?}", scores.get("黄队"));
    println!("队伍数 = {}", scores.len());

    for (team, score) in &scores {
        println!("{}: {}", team, score);
    }
}
```

`insert` 的返回值是 `Option<V>`：键原本不存在时返回 `None`，存在时返回旧值并被新值覆盖。上面对「蓝队」插入两次，最终只剩 30。`get` 接收 `&Q`，其中 `String` 可以借用为 `str`，所以 `scores.get("蓝队")` 不必先构造一个 `String`。`get` 返回 `Option<&V>`，`get_mut` 返回 `Option<&mut V>`；`map[&key]` 也能取值，但键不存在时会 panic，不适合处理外部输入。

上面的遍历顺序不是插入顺序，每次运行还可能不同。HashMap 不承诺顺序，也不应该依赖顺序。

如果逻辑是「键不存在就插入默认值，存在就更新」，先 `contains_key` 再 `insert` 会做两次查找。`entry` API 把这两步合成一次。

```rust
use std::collections::HashMap;

fn main() {
    let mut scores: HashMap<String, u32> = HashMap::new();

    scores.entry(String::from("蓝队")).or_insert(0);
    *scores.entry(String::from("蓝队")).or_insert(0) += 10;
    scores.entry(String::from("红队")).or_insert_with(|| 5);

    println!("蓝队 = {:?}", scores.get("蓝队"));
    println!("红队 = {:?}", scores.get("红队"));
}
```

`entry(key)` 返回一个 `Entry` 枚举，表示这个键「已被占用」或「空缺」。`or_insert(v)` 在空缺时插入 `v`，无论哪种情况都返回 `&mut V`，所以可以直接解引用后累加。`or_insert_with` 接收闭包，只在需要插入时才计算默认值，适合默认值构造代价高的场景。注意 `or_insert(0)` 的参数总会被求值，闭包版本则不会。

下面是一个完整的小作品：统计一段文本里每个单词出现的次数，再按频次输出前 3 名。频次相同时按单词字典序排列，保证结果可复现。

```rust
use std::collections::HashMap;

fn main() {
    let text = "the quick brown fox jumps over the lazy dog the fox";
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for word in text.split_whitespace() {
        *counts.entry(word).or_insert(0) += 1;
    }

    let mut pairs: Vec<(&str, usize)> = counts.into_iter().collect();
    pairs.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));

    for (word, n) in pairs.iter().take(3) {
        println!("{} {}", word, n);
    }
}
```

```text
the 3
fox 2
brown 1
```

这里用 `HashMap<&str, usize>` 而不是 `HashMap<String, usize>`：键直接借用原文中的片段，不复制字符串。前提是原文 `text` 的生命周期覆盖整个 map，本例中 `text` 是字面量，满足条件。如果单词来自逐行读入的文件，每一行在下一轮就被释放，就必须把键改成 `String`，用 `to_string()` 复制一份。`counts.into_iter()` 消耗 map 并产出 `(&str, usize)`，收集成 `Vec` 后排序；打印时用 `iter()` 借用 `pairs`。

排序这一步无法在 HashMap 上原地完成，因为 HashMap 没有「按值排序」的概念。通用做法是收集成 `Vec` 再排序；只需要按固定顺序输出时，也可以直接换 `BTreeMap`。

## 12.4 按需求选集合

标准库的集合各有取舍，先看一张对照表。

| 集合 | 顺序 | 主要操作代价 | 适用场景 |
| --- | --- | --- | --- |
| `Vec<T>` | 插入顺序 | 索引 O(1)，中间插入 O(n) | 序列、栈、需要按位置访问 |
| `VecDeque<T>` | 插入顺序 | 两端插入删除 O(1) | 队列、滑动窗口、BFS |
| `HashMap<K, V>` | 无序 | 平均 O(1)，最坏 O(n) | 按键查找、计数、缓存 |
| `BTreeMap<K, V>` | 键升序 | O(log n) | 需要有序遍历或范围查询 |
| `HashSet<T>` | 无序 | 平均 O(1) | 去重、集合运算 |
| `BTreeSet<T>` | 升序 | O(log n) | 有序去重、取前 k 小 |

`BTreeMap` 的键必须实现 `Ord`，代价是每次操作 O(log n)，回报是遍历时键天然有序，且支持 `range` 取一段区间。需要「按时间排序的事件」「按分数排名的玩家」时，它比 HashMap 加排序更直接。

```rust
use std::collections::{BTreeMap, VecDeque};

fn main() {
    let mut steps = BTreeMap::new();
    steps.insert(3, "启动服务");
    steps.insert(1, "读取配置");
    steps.insert(2, "连接数据库");

    for (n, text) in &steps {
        println!("{}: {}", n, text);
    }

    let mut window: VecDeque<i32> = VecDeque::new();
    for x in [1, 2, 3, 4] {
        window.push_back(x);
        if window.len() > 3 {
            window.pop_front();
        }
    }
    println!("{:?}", window);
}
```

输出顺序是 1、2、3，与插入顺序无关。`VecDeque` 是双端队列，底层是一段环形缓冲区（ring buffer），`push_front`、`push_back`、`pop_front`、`pop_back` 都是 O(1)；用它实现「只保留最近 3 条」比用 `Vec::remove(0)` 更合适，后者每次都要搬移整个数组。需要把 `VecDeque` 交给只接受切片的函数时，先调用 `make_contiguous`。

`HashSet<T>` 是只有键没有值的 HashMap，`insert` 返回 `bool` 表示是否为新元素，因此一行代码就能去重。它也支持并集、交集、差集等集合运算。

```rust
use std::collections::HashSet;

fn main() {
    let a: HashSet<i32> = [1, 2, 3, 4].into_iter().collect();
    let b: HashSet<i32> = [3, 4, 5].into_iter().collect();

    let union: HashSet<i32> = a.union(&b).copied().collect();
    let inter: HashSet<i32> = a.intersection(&b).copied().collect();

    println!("并集元素数 = {}", union.len());
    println!("交集元素数 = {}", inter.len());
    println!("b 含 5 = {}", b.contains(&5));
}
```

选择顺序可以记成三句话：默认用 `Vec`；需要按键快速查找时用 `HashMap`；需要顺序、范围或稳定输出时用 `BTreeMap` 或 `BTreeSet`。队列和滑动窗口用 `VecDeque`，去重用 `HashSet`。如果数据量很小（几十个元素），线性扫描的 `Vec` 往往比哈希更快，因为哈希本身也有计算和缓存不友好的问题。

## 12.5 遍历时增删：借用检查器在拦什么

初学者最常撞上的一堵墙是这段代码。

```rust,noplayground
fn main() {
    let mut v = vec![1, 2, 3];
    for x in &v {
        if *x == 2 {
            v.push(4);
        }
    }
    println!("{:?}", v);
}
```

`for x in &v` 创建了一个迭代器，它在整个循环期间借用 `v`；`v.push(4)` 需要可变借用。同一时刻既有共享借用又有可变借用，违反借用规则，编译器报 E0502。这条规则不是形式主义：`push` 可能触发扩容，把元素搬到新地址，迭代器手里的指针会立刻失效。编译器在类型层面阻止了悬垂指针。

有四种常用写法可以表达「边看边改」。

第一种是索引循环。范围 `0..v.len()` 在循环开始时求值一次，之后不再借用 `v`，循环体里可以自由修改。

```rust
fn main() {
    let mut v = vec![1, 2, 3, 4, 5, 6];

    v.retain(|x| x % 2 == 0);
    println!("retain 之后: {:?}", v);

    let mut v = vec![1, 2, 3];
    for i in 0..v.len() {
        if v[i] == 2 {
            v.push(20);
        }
    }
    println!("索引循环之后: {:?}", v);

    let mut v = vec![1, 2, 3];
    let extra: Vec<i32> = v.iter().filter(|x| **x > 1).map(|x| x * 10).collect();
    v.extend(extra);
    println!("extend 之后: {:?}", v);
}
```

索引循环的上界只算一次，所以新压入的元素不会被本轮遍历；如果你希望处理新元素，写成 `while i < v.len()`，并确保循环条件最终会变为假。`retain` 用于删除：它只遍历一遍，内部通过移动元素保持连续，比「倒序 remove」清晰。`extend` 接收一个迭代器，把新元素批量追加；先收集再追加的思路适用于任何「根据旧元素算出新元素」的场景。

第二种是 `drain`。它按范围移除元素，并把它们交给一个迭代器，原容器立刻缩短。

```rust
fn main() {
    let mut v = vec![1, 2, 3, 4, 5];
    let removed: Vec<i32> = v.drain(1..4).collect();
    println!("移除: {:?}", removed);
    println!("剩余: {:?}", v);
}
```

`drain` 适合「清空并处理」：缓冲区换一批数据、队列消费前 N 个。它拿的是 `&mut self`，所以调用期间不能再从 `v` 读取，但被移除的元素已经归迭代器所有。

第三种是拆分借用。需要同时修改两个不同位置时，`split_at_mut` 把切片拆成两个互不重叠的可变借用，借用检查器能验证它们不重叠；`iter_mut` 则在遍历时给每个元素一个 `&mut T`，适合就地修改而不是增删。

第四种针对映射：先查再插的写法在 HashMap 上既慢又容易和借用冲突，改用 `entry` 一次完成。下面这种写法无法通过编译，因为循环持有 `&map`，插入需要 `&mut map`。

```rust,noplayground
use std::collections::HashMap;

fn main() {
    let mut map: HashMap<&str, i32> = HashMap::new();
    map.insert("a", 1);
    for (key, value) in &map {
        if *value == 1 {
            map.insert(*key, 2);
        }
    }
    println!("{:?}", map);
}
```

正确的做法是先收集要改的键，循环结束后再改；或者直接对每个键调用 `entry`。判断标准很简单：只要循环体需要增长容器，就不要在循环里持有它的借用。

## 12.6 深入：三元组、哈希与 UTF-8 不变式

### Vec 的三元组与 realloc

`Vec<T>` 在栈上只有三个字段：指向堆缓冲区的指针、当前长度、容量。

```text
Vec<T>（栈上，64 位下 24 字节）
┌──────────┬──────────┬──────────┐
│ ptr      │ len = 3  │ cap = 4  │
└────┬─────┴──────────┴──────────┘
     │
     ▼
   堆：[ T ][ T ][ T ][ 未使用 ]
```

`len` 是已经初始化的元素个数，`cap` 是缓冲区能容纳的元素个数。访问 `v[i]` 只需 `ptr + i * size_of::<T>()`，再用 `len` 做一次边界检查。正因为栈上只有这三个机器字，`Vec<u8>`、`String` 的大小都是 24 字节，`&str` 是 16 字节（指针加长度），`char` 是 4 字节。移动一个 `Vec` 只复制这 24 字节，堆上的元素不动。

当 `len == cap` 时再 `push`，Vec 会向分配器申请一块更大的缓冲区（实现上通常翻倍），把已有元素逐个移过去，再释放旧缓冲区。这一步叫 realloc，单次代价 O(n)，但每扩容一次容量至少翻倍，所以连续 n 次 `push` 的总复制量是 O(n)，摊还到每次是 O(1)。扩容后旧地址失效，任何指向旧缓冲区的引用都会悬垂——这就是 `push` 要求 `&mut self`、而迭代器持有 `&self` 时不能插入的根本原因。`with_capacity` 把「多次扩容」换成「一次分配」，代价是可能浪费空间；元素数量可预估时值得用。

### HashMap 的 SipHash 与哈希洪水

HashMap 先对键计算哈希值，再对容量取模得到桶下标。如果攻击者能控制键，并且知道哈希函数，就可以构造大量落到同一个桶的键：每次查找都要扫描整条链表，n 次查找退化成 O(n²)，CPU 被占满。这种攻击叫哈希洪水（hash flooding）。

Rust 的默认哈希器是 SipHash-1-3，配合 `RandomState`：种子来自操作系统提供的随机数，每个 HashMap 得到的哈希键不同，攻击者无法在程序运行前预测桶的分布，也就无法稳定构造碰撞。代价是 SipHash 比那些为速度设计的哈希函数慢。标准库选择了抗攻击性，因为 HashMap 常常直接处理网络输入。

如果键完全可信，且性能敏感，可以用 `HashMap::with_hasher` 换一个更快的 `BuildHasher`，例如整数键常用的 FxHash 或 ahash。这是一个明确的权衡：放弃抗碰撞攻击的能力，换取吞吐量。默认实现下最坏情况仍是 O(n)，只是攻击者难以制造。

HashMap 无序也是同一套机制的结果。桶下标由哈希值决定，与插入顺序无关；插入触发扩容时所有元素会重新分布；不同运行、不同 HashMap 的哈希键都可能不同，遍历顺序还会变化。所以「按插入顺序输出」不能靠 HashMap，要么改用 `BTreeMap`，要么在输出前收集排序。要求稳定输出时，排序键里要包含全部并列条件，否则相同频次的元素顺序仍然随机。

### String 的 UTF-8 不变式

`String` 的内部就是一个 `Vec<u8>`，额外承诺所有字节构成合法的 UTF-8。这条不变式让 `&str` 可以安全地按字符解码，也让所有安全 API 都必须维护它：`push_str` 接收的已经是 `&str`，追加后仍然合法；从字节构造时用 `String::from_utf8` 做校验。

```rust
fn main() {
    let bytes = vec![0xE4, 0xBD, 0xA0, 0xE5, 0xA5, 0xBD];
    match String::from_utf8(bytes) {
        Ok(s) => println!("{}", s),
        Err(e) => println!("不是合法 UTF-8: {}", e),
    }

    let bad = vec![0xFF, 0xFE];
    println!("bad 是否失败 = {}", String::from_utf8(bad).is_err());
}
```

`from_utf8` 返回 `Result`，失败时给出出错的位置。对应的 `unsafe` 版本 `from_utf8_unchecked` 跳过校验：一旦传入非法字节，后续任何 `&str` 操作都建立在错误假设上，属于未定义行为。需要处理非 UTF-8 的旧编码时，用专门的编码库解码成 `String`，不要绕过校验。

UTF-8 是变长编码：一个 Unicode 标量值占 1 到 4 个字节，`char` 固定占 4 字节，但存进字符串后按实际编码缩短。因此 `len()` 返回字节数，`chars().count()` 返回字符数，后者需要从头扫描，O(n)。按字节切片是 O(1)，但必须落在字符边界；按字符取第 n 个没有 O(1) 的实现，`chars().nth(n)` 是 O(n)。这些数字解释了 12.2 的规则：不是 Rust 故意为难你，而是 UTF-8 的存储方式决定了「第几个字符」无法直接算出地址。

## 典型错误案例

下面五个错误都来自集合的所有权与索引规则。报错文本由 rustc 1.100.0 实际编译得到，行号与代码块一致。

### 案例 1：遍历时修改——E0502

```rust,editable
fn main() {
    let mut v = vec![1, 2, 3];
    for x in &v {
        if *x == 2 {
            v.push(4);
        }
    }
    println!("{:?}", v);
}
```

```text
error[E0502]: cannot borrow `v` as mutable because it is also borrowed as immutable
 --> src/main.rs:5:13
  |
3 |       for x in &v {
  |       -        --
  |       |        |
  |       |        immutable borrow occurs here
  |  _____|        immutable borrow later used here
  | |
4 | |         if *x == 2 {
5 | |             v.push(4);
  | |             ^^^^^^^^^ mutable borrow occurs here
6 | |         }
7 | |     }
  | |_____- this for loop borrows `v` immutably, preventing mutation within its body
  |
  = help: consider using an index-based loop instead, or collecting modifications into a separate collection
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`for x in &v` 展开成 `IntoIterator::into_iter(&v)`，迭代器在整个循环期间持有 `v` 的共享借用。`v.push(4)` 需要可变借用，两者不能共存。更深一层的原因是 `push` 可能扩容并搬移元素，迭代器内部的指针会失效，编译器必须在类型层面拒绝这种写法。

**修复**：改成索引循环，让循环变量是一个独立的 `usize`，不再借用容器；或者把要追加的元素先收集到另一个 `Vec`，循环结束后再 `extend`。

```rust,editable
fn main() {
    let mut v = vec![1, 2, 3];
    for i in 0..v.len() {
        if v[i] == 2 {
            v.push(4);
        }
    }
    println!("{:?}", v);

    let mut v = vec![1, 2, 3];
    let extra: Vec<i32> = v.iter().filter(|x| **x > 1).copied().collect();
    v.extend(extra);
    println!("{:?}", v);
}
```

</details>

### 案例 2：for 循环消费了 Vec——E0382

```rust,editable
fn main() {
    let v = vec![String::from("a"), String::from("b")];
    for s in v {
        println!("{}", s);
    }
    println!("len = {}", v.len());
}
```

```text
error[E0382]: borrow of moved value: `v`
   --> src/main.rs:6:26
    |
  2 |     let v = vec![String::from("a"), String::from("b")];
    |         - move occurs because `v` has type `Vec<String>`, which does not implement the `Copy` trait
  3 |     for s in v {
    |              - `v` moved due to this implicit call to `.into_iter()`
...
  6 |     println!("len = {}", v.len());
    |                          ^ value borrowed here after move
    |
note: the `for` loop is desugared into a call to `std::iter::IntoIterator::into_iter`, which takes ownership of the receiver `self`, which moves `v`
help: consider iterating over a slice of the `Vec<String>`'s content to avoid moving into the `for` loop
    |
  3 |     for s in &v {
    |              +
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`for s in v` 调用 `v.into_iter()`，这个方法接收 `self`，把整个 `Vec<String>` 按值移进迭代器，每个 `String` 也随所有权一起交给循环变量。循环结束后 `v` 已经被消耗，再调用 `len()` 就是使用已移动的值。`Vec<String>` 没有实现 `Copy`，编译器不会自动复制。

**修复**：只需要读取时用 `&v`，循环变量类型变成 `&String`；确实需要拥有每个元素时，允许 `v` 被消费，循环后不要再使用它。

```rust,editable
fn main() {
    let v = vec![String::from("a"), String::from("b")];
    for s in &v {
        println!("{}", s);
    }
    println!("len = {}", v.len());

    let total: usize = v.iter().map(|s| s.len()).sum();
    println!("总字节数 = {}", total);
}
```

</details>

### 案例 3：在 HashMap 上调用 push——E0599

```rust,editable
use std::collections::HashMap;

fn main() {
    let mut map: HashMap<&str, i32> = HashMap::new();
    map.push("a", 1);
    println!("{:?}", map);
}
```

```text
error[E0599]: no method named `push` found for struct `HashMap<K, V, S, A>` in the current scope
 --> src/main.rs:5:9
  |
5 |     map.push("a", 1);
  |         ^^^^
  |
help: you might have meant to use `insert`
  |
5 -     map.push("a", 1);
5 +     map.insert("a", 1);
  |
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`push` 是序列类型的方法，含义是「在尾部追加一个元素」；HashMap 没有尾部，也没有元素顺序，它的写入方法叫 `insert`，语义是「键存在则覆盖，不存在则插入」。E0599 表示当前类型上找不到这个方法，编译器据此给出了改名建议。

**修复**：改用 `insert`。如果只想在键不存在时写入，用 `entry(key).or_insert(value)`，避免覆盖已有值。

```rust,editable
use std::collections::HashMap;

fn main() {
    let mut map: HashMap<&str, i32> = HashMap::new();
    map.insert("a", 1);
    map.insert("b", 2);
    map.entry("a").or_insert(100);

    println!("{:?}", map.get("a"));
    println!("{:?}", map.get("b"));
}
```

</details>

### 案例 4：从 Vec 索引里移出值——E0507

```rust,editable
fn main() {
    let v = vec![String::from("a")];
    let first = v[0];
    println!("{}", first);
}
```

```text
error[E0507]: cannot move out of index of `Vec<String>`
 --> src/main.rs:3:17
  |
3 |     let first = v[0];
  |                 ^^^^ move occurs because value has type `String`, which does not implement the `Copy` trait
  |
help: consider borrowing here
  |
3 |     let first = &v[0];
  |                 +
help: consider cloning the value if the performance cost is acceptable
  |
3 |     let first = v[0].clone();
  |                     ++++++++
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`Vec<T>` 的 `Index<usize>` 返回 `&T`，`v[0]` 只是 `*v.index(0)` 的语法糖。把解引用得到的 `String` 直接赋给新变量，等于从一个借用后面把值搬走；`String` 管理堆内存，不允许按位复制，编译器因此拒绝。如果元素是 `i32` 这类 `Copy` 类型，这里会自动复制，不会报错，所以这个错误常常在把 `Vec<i32>` 改成 `Vec<String>` 之后突然出现。

**修复**：只读时加 `&`；需要独立的字符串时 `clone`；确实要取出所有权时用 `remove(i)` 或 `into_iter()`，两者都会真正移动元素。

```rust,editable
fn main() {
    let v = vec![String::from("a"), String::from("b")];

    let first = &v[0];
    println!("{}", first);

    let owned = v[0].clone();
    println!("{}", owned);

    let mut v2 = v;
    let moved = v2.remove(0);
    println!("{}", moved);
    println!("剩余 {:?}", v2);
}
```

</details>

### 案例 5：用整数索引 String——E0277

```rust,editable
fn main() {
    let s = String::from("hi");
    let c = s[0];
    println!("{}", c);
}
```

```text
error[E0277]: the type `str` cannot be indexed by `{integer}`
   --> src/main.rs:3:15
    |
  3 |     let c = s[0];
    |               ^ string indices are ranges of `usize`
    |
    = help: the trait `SliceIndex<str>` is not implemented for `{integer}`
    = note: you can use `.chars().nth()` or `.bytes().nth()`
```

<details class="case-answer">
<summary>查看答案</summary>

**原因**：`String` 通过 `Deref` 把索引操作转发给 `str`，而 `str` 只实现了 `Index<Range<usize>>` 这类范围索引，没有实现 `Index<usize>`。UTF-8 是变长编码，第 n 个字符的起始字节位置无法用一次乘法算出；如果允许 `s[0]` 返回第 0 个字符，这次索引就必须是 O(n)，与「索引是 O(1)」的直觉冲突。语言选择不提供整数索引，只提供按字节范围切片，并在运行时检查边界。

**修复**：按字符取用 `chars().nth(n)`（O(n)），需要重复按序号访问时先收集成 `Vec<char>`；按字节取用 `as_bytes()`。如果目标只是首字符，`chars().next()` 更直接。

```rust,editable
fn main() {
    let s = String::from("hi");

    let c = s.chars().next().unwrap();
    println!("{}", c);

    let bytes = s.as_bytes();
    println!("第一个字节 = {}", bytes[0]);

    let chars: Vec<char> = s.chars().collect();
    println!("第三个字符 = {:?}", chars.get(2));
}
```

</details>

## 动手练习

1. 从标准输入读一行以空格分隔的整数，存入 `Vec<i64>`，输出总和、最大值与平均值。要求处理空输入与无法解析的片段，不能 panic；平均值用 `f64`，保留两位小数。
2. 实现 `fn reverse(s: &str) -> String`，按 Unicode 字符反转，而不是按字节反转。用 `"你好, Rust"` 验证，说明为什么 `s.bytes().rev()` 不能得到正确结果。
3. 给定 `Vec<i32>`，用 `HashSet` 去重并保持首次出现的顺序，输出结果；再用 `BTreeSet` 输出去重后的升序结果。比较两段代码在顺序与复杂度上的差异。
4. 用 `VecDeque` 实现一个固定容量的最近日志缓冲区：容量为 N，新日志从尾部进入，超过容量时从头部丢弃。写一个 `push` 方法并打印最终内容，说明为什么不用 `Vec::remove(0)`。
5. 扩展 12.3 的词频统计器：从标准输入读取多行文本，忽略大小写与标点，输出出现次数最多的前 10 个单词。频次相同时按字典序排列；同时输出总词数与不同词数。提示：`to_lowercase`、`split`、`trim_matches` 与 `entry`。

## 小结

- `Vec<T>` 在堆上维护连续缓冲区，栈上只有指针、长度、容量三个字段；`len()` 是元素个数，`capacity()` 是无需重新分配的容量，末尾 `push` 的摊还代价是 O(1)，已知规模时用 `with_capacity`。
- `v[i]` 越界会 panic，`v.get(i)` 返回 `Option<&T>`；外部输入优先用 `get`。
- `for x in &v` 借用容器，`for x in v` 移动容器；移动后原变量不能再使用。
- `String` 拥有 UTF-8 字节缓冲区，`&str` 是借用，`str` 是动态大小类型，可用 `&str` 借用，也可用 `Box<str>` 拥有。
- `s.len()` 是字节数，`s.chars().count()` 是字符数；`String` 不支持整数索引，按字节切片必须落在字符边界。
- `+` 会移动左操作数，`format!` 借用所有参数；函数参数优先写 `&str`。
- `HashMap` 平均 O(1)、无序；`entry(...).or_insert(...)` 用一次查找完成插入或更新。
- `BTreeMap` 按键有序、O(log n)；`HashSet` 用于去重与集合运算；`VecDeque` 两端操作 O(1)。
- 遍历时增删会触发 E0502，改用索引循环、先收集再 `extend`、`retain`/`drain`，或在映射上用 `entry`。
- 扩容 realloc 会让旧引用失效，这是借用规则背后的内存原因；默认 SipHash 用速度换抗哈希洪水能力。




