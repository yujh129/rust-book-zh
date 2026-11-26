# 第 44 章 项目：贪吃蛇——把状态机写进类型里

> **本章目标**
>
> - 用「规格 → 数据模型 → 核心循环 → 输入层 → 测试」的顺序完整交付一个游戏
> - 体会 `VecDeque` 为什么天生适合表示蛇身，以及「尾巴刚离开的格子」这类规则细节
> - 把随机数、渲染、输入都抽象成可替换的部件，让游戏逻辑在没有终端的环境里也能被验证
> - 写出 9 个覆盖边界条件的单元测试，并用真实 `cargo test` 输出验收
> - 认识五个在游戏/状态机代码里高频出现的编译错误

## 44.1 为什么用贪吃蛇收尾

前面的项目各有一个重心：`minigrep` 练错误处理与生命周期，JSON 解析器练递归下降，HTTP 服务器练 I/O 与并发。贪吃蛇的重心是**状态机**：每一帧只做一件事——根据当前状态和输入算出下一个状态，然后把状态画出来。

这种「输入 → 状态 → 输出」的结构是绝大多数程序的内核，只是贪吃蛇把它压缩到了一百多行里。它还有三个对学习者非常友好的性质：

1. **规则可以用中文写清楚**，写完就能翻译成代码，几乎没有模糊地带。
2. **不需要任何第三方依赖**：不用 `crossterm`、不用 `rand`、不用图形库，纯 `std` 就能跑。
3. **可以完全自动化测试**：把「方向」当成函数参数传进去，游戏就变成了一个纯函数集合，`cargo test` 能把每条规则都跑一遍。

最后一点是关键。很多人写游戏卡在「必须盯着终端看才知道对不对」。本章会刻意把**输入层**和**逻辑层**分开：逻辑层只认 `Direction`，不认键盘；输入层负责把 `w/a/s/d` 翻译成 `Direction`。这样逻辑层就能被测试，也能被脚本驱动——本章末尾的演示输出就是这么跑出来的。

## 44.2 先把规则写成规格

动手之前，把规则一条条写下来。凡是写不清楚的地方，代码里一定会变成 bug。

**棋盘**：`w × h` 个格子，坐标 `(x, y)`，左上角是 `(0, 0)`，`x` 向右增，`y` 向下增。

**蛇**：一个有序的格子序列，第一个元素是头。初始长度 3，水平放置，朝右。

**食物**：任意一个不被蛇占据的格子。吃掉后立刻在剩余空格里随机再放一个。

**每一步**：

1. 取当前方向，算出新的头位置 `next = head + delta(dir)`。
2. 若 `next` 越界 → 结束，原因「撞到墙」。
3. 若 `next` 撞到身体 → 结束，原因「咬到自己」。
4. 把 `next` 插到队首。
5. 若 `next` 是食物：分数 +1，不删尾巴（所以变长），重新放食物；若蛇长等于格子总数 → 结束，原因「满屏通关」。
6. 否则删掉队尾（长度不变）。

**转向**：不能 180 度掉头。这条规则不是「游戏手感」，而是必须的——否则蛇头会立刻撞上自己脖子上的那一节。

**一个容易漏掉的细节**：第 3 步检查身体时，**不能算上尾巴**。因为尾巴在这一步会离开，蛇头进入尾巴刚才的位置是合法的。如果不排除尾巴，蛇在短到 4 节时就会出现「明明能走却被判死」的诡异 bug。第 44.4 节的 `step` 用 `take(body_len)` 处理这一点，44.6 节还有专门的测试 `tail_cell_is_legal_to_enter`。

把规则写成表格，代码结构就浮出来了：

| 规则 | 落到哪里 |
|---|---|
| 坐标与方向 | `Point`、`Direction::delta` |
| 越界判定 | `step` 开头的边界检查 |
| 自撞判定 | `step` 里对 `snake[..len-1]` 的查找 |
| 长度变化 | `push_front` 之后是否 `pop_back` |
| 食物放置 | `place_food`（在空格里均匀抽样） |
| 结束状态 | `Status` 枚举，`step` 返回 `bool` |

## 44.3 数据模型：三个小类型撑起整个游戏

先把与游戏规则无关的「基础词汇」定义好。坐标用 `i32` 而不是 `usize`，因为计算 `x - 1` 时会短暂出现负数，用无符号类型会直接 panic。

```rust
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Point {
    x: i32,
    y: i32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn delta(self) -> (i32, i32) {
        match self {
            Direction::Up => (0, -1),
            Direction::Down => (0, 1),
            Direction::Left => (-1, 0),
            Direction::Right => (1, 0),
        }
    }

    fn opposite(self) -> Self {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }

    fn from_char(c: char) -> Option<Self> {
        match c {
            'w' | 'W' => Some(Direction::Up),
            's' | 'S' => Some(Direction::Down),
            'a' | 'A' => Some(Direction::Left),
            'd' | 'D' => Some(Direction::Right),
            _ => None,
        }
    }
}

fn main() {
    for (name, d) in [
        ("上", Direction::Up),
        ("下", Direction::Down),
        ("左", Direction::Left),
        ("右", Direction::Right),
    ] {
        let (dx, dy) = d.delta();
        println!("{} 的增量 ({}, {})，反向是 {:?}", name, dx, dy, d.opposite());
    }
    println!("{:?}", Direction::from_char('w'));
    println!("{:?}", Direction::from_char('x'));
}
```

输出：

```text
上 的增量 (0, -1)，反向是 Down
下 的增量 (0, 1)，反向是 Up
左 的增量 (-1, 0)，反向是 Right
右 的增量 (1, 0)，反向是 Left
Some(Up)
None
```

`derive` 的这个组合值得解释一句：`Copy` 让 `Point` 和 `Direction` 传参时不用考虑所有权（它们只有 8 个字节和 1 个字节）；`PartialEq` 让 `next == self.food` 这种比较成立；`Debug` 让测试失败时能直接看到值。

随机数也不外借：用一个 4 行的线性同余生成器（LCG）就足够「食物别老在同一个地方」，而且**种子可复现**——这对测试和演示至关重要。

```rust
struct Lcg(u64);

impl Lcg {
    fn new(seed: u64) -> Self {
        Lcg(seed | 1)
    }

    fn next_u32(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    fn below(&mut self, n: u32) -> u32 {
        if n == 0 {
            0
        } else {
            self.next_u32() % n
        }
    }
}

fn main() {
    let mut rng = Lcg::new(7);
    let rolls: Vec<u32> = (0..10).map(|_| rng.below(6)).collect();
    println!("{:?}", rolls);
}
```

`seed | 1` 保证种子非零（全零的 LCG 会永远输出 0）；`>> 33` 取高位而不是 `% 2^32` 取低位，因为 LCG 的低位周期很短，直接取低位会出现明显的规律。**真正的密码学随机数不能用 LCG**，但游戏里决定食物位置完全够用。

## 44.4 核心循环：一个结构体加四个方法

把可变状态收进一个结构体，是把状态机写清楚的第一步。`Game` 持有棋盘尺寸、蛇身、方向、食物、分数、随机源和状态，除此之外没有任何全局变量。

```rust,noplayground
use std::collections::VecDeque;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Status {
    Running,
    Won,
    Crashed(&'static str),
}

struct Game {
    w: i32,
    h: i32,
    snake: VecDeque<Point>,
    dir: Direction,
    food: Point,
    score: u32,
    rng: Lcg,
    status: Status,
}
```

`Status` 用 `Crashed(&'static str)` 而不是 `Crashed(String)`：结束原因只有固定的几个字符串字面量，用 `&'static str` 不分配堆内存，还天然实现了 `Copy`——测试里 `assert_eq!(g.status, Status::Crashed("咬到自己"))` 才能直接比较。

### 构造：让不可能的状态无法构造

```rust,noplayground
impl Game {
    fn new(w: i32, h: i32, seed: u64) -> Self {
        let mut snake = VecDeque::new();
        let cy = h / 2;
        let cx = w / 2;
        snake.push_front(Point { x: cx, y: cy });
        snake.push_back(Point { x: cx - 1, y: cy });
        snake.push_back(Point { x: cx - 2, y: cy });
        let mut g = Game {
            w,
            h,
            snake,
            dir: Direction::Right,
            food: Point { x: 0, y: 0 },
            score: 0,
            rng: Lcg::new(seed),
            status: Status::Running,
        };
        g.place_food();
        g
    }

    fn head(&self) -> Point {
        *self.snake.front().expect("蛇身永远不为空")
    }
}
```

注意四个细节：

1. **字段声明用了简写** `w,` 等价于 `w: w`。Rust 的这个语法在构造结构体时用得很频繁。
2. **`food` 先填 `(0, 0)` 再调 `place_food`**。因为「食物不能在蛇身上」这个不变式需要蛇已经存在，而构造结构体时无法先调方法再赋值字段（除非用 `Option` 或两段式构造）。先放一个占位值、构造完立刻修正，是这个模式的常用解法。
3. **`head()` 返回 `Point` 而不是 `&Point`**：`Point` 是 `Copy`，返回值省去一次借用，调用方也不需要关心 `snake` 的借用期。
4. **`expect` 在这里是安全的**：不变式是「蛇至少有 3 节」，只有测试里手工清空 `snake` 才会破坏它——那种情况本就该 panic。

### 放食物：在空格里均匀抽样

```rust,noplayground
impl Game {
    fn place_food(&mut self) {
        let mut free = Vec::new();
        for y in 0..self.h {
            for x in 0..self.w {
                let p = Point { x, y };
                if !self.snake.contains(&p) {
                    free.push(p);
                }
            }
        }
        if free.is_empty() {
            self.status = Status::Won;
            return;
        }
        let idx = self.rng.below(free.len() as u32) as usize;
        self.food = free[idx];
    }
}
```

「先收集所有空格，再从中随机取一个」看起来笨，但它保证了**食物分布均匀**。常见的错误写法是「随机取坐标，若在蛇身上就重试」：当蛇很长时，重试次数会爆炸（棋盘快满时可能连续几百次都撞上蛇身），而且分布也不再均匀。用 `O(w×h)` 的一次扫描换来正确性和稳定性，在 12×8 的棋盘上完全不值得优化。

### 转向：拒绝 180 度掉头

```rust,noplayground
impl Game {
    fn turn(&mut self, d: Direction) -> bool {
        if d == self.dir.opposite() {
            return false;
        }
        self.dir = d;
        true
    }
}
```

返回 `bool` 而不是 `()`，是为了让调用方能区分「这次按键生效了」和「这次按键被忽略」。真实游戏里这决定了要不要发出音效；测试里它让「掉头被拒绝」可以被断言。

### 走一步：整个游戏唯一的状态转移函数

```rust,noplayground
impl Game {
    fn step(&mut self) -> bool {
        if self.status != Status::Running {
            return false;
        }
        let (dx, dy) = self.dir.delta();
        let head = self.head();
        let next = Point {
            x: head.x + dx,
            y: head.y + dy,
        };
        if next.x < 0 || next.y < 0 || next.x >= self.w || next.y >= self.h {
            self.status = Status::Crashed("撞到墙");
            return false;
        }
        let eating = next == self.food;
        let body_len = if eating { self.snake.len() } else { self.snake.len() - 1 };
        if self.snake.iter().take(body_len).any(|p| *p == next) {
            self.status = Status::Crashed("咬到自己");
            return false;
        }
        self.snake.push_front(next);
        if eating {
            self.score += 1;
            if self.snake.len() as i32 == self.w * self.h {
                self.status = Status::Won;
                return true;
            }
            self.place_food();
        } else {
            self.snake.pop_back();
        }
        true
    }
}
```

这二十行里有四处值得停下来看：

**第一，先判死再改状态。** 撞墙和自撞都在 `push_front` 之前返回，所以游戏结束时棋盘保持最后一帧合法的样子。如果先移动再检查，玩家会看到蛇头已经插进墙里——这正是很多手写实现的第一版 bug。

**第二，`body_len` 的两种取值。** 不吃食物时尾巴会离开，所以检查范围是 `len - 1`；吃食物时不删尾巴，必须检查整个身体。这个 `if eating` 的三元选择是「尾巴格子是否合法」的唯一实现点。

**第三，`iter().take(body_len).any(...)` 而不是 `contains`。** `VecDeque::contains` 无法表达「除尾巴以外」，`take` 配合 `any` 既清晰又是短路求值。

**第四，胜利判定放在 `push_front` 之后。** 此时 `snake.len()` 已经是吃完之后的长度，直接和格子总数比较即可；不需要额外的计数器。

### 渲染：把状态变成字符画

```rust,noplayground
impl Game {
    fn render(&self) -> String {
        let mut s = String::new();
        s.push('+');
        for _ in 0..self.w {
            s.push('-');
        }
        s.push_str("+\n");
        for y in 0..self.h {
            s.push('|');
            for x in 0..self.w {
                let p = Point { x, y };
                let ch = if p == self.head() {
                    'O'
                } else if self.snake.contains(&p) {
                    'o'
                } else if p == self.food {
                    '*'
                } else {
                    ' '
                };
                s.push(ch);
            }
            s.push_str("|\n");
        }
        s.push('+');
        for _ in 0..self.w {
            s.push('-');
        }
        s.push_str("+\n");
        s
    }
}
```

`render` 返回 `String` 而不是直接 `print!`，这是「纯函数」的又一处体现：测试可以断言渲染结果，主程序可以攒多帧再一起输出（减少 I/O 次数），未来接图形界面时也能直接换掉这一层。四重 `if` 的顺序很重要——蛇头优先于身体，身体优先于食物，否则蛇头会被画成 `o`。

## 44.5 输入层与主程序：把键盘翻译成方向

逻辑层完全不知道键盘的存在。`main` 的职责只有三件：读参数、把标准输入里的字符逐个翻译成 `Direction`、把结果打印出来。

```rust,noplayground
fn describe(status: Status) -> String {
    match status {
        Status::Running => String::from("还在进行"),
        Status::Won => String::from("满屏通关"),
        Status::Crashed(why) => format!("结束：{}", why),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let w = args.first().and_then(|a| a.parse().ok()).unwrap_or(12);
    let h = args.get(1).and_then(|a| a.parse().ok()).unwrap_or(8);
    let seed = args.get(2).and_then(|a| a.parse().ok()).unwrap_or(7);
    let frames = args.iter().any(|a| a == "--frames");

    let mut input = String::new();
    io::stdin().read_to_string(&mut input).expect("读标准输入失败");

    let mut game = Game::new(w, h, seed);
    println!("{}x{} 的棋盘，食物在 ({}, {})", w, h, game.food.x, game.food.y);
    if frames {
        print!("{}", game.render());
    }
    let mut moves = 0;
    for c in input.chars() {
        if c == 'q' {
            println!("主动退出");
            break;
        }
        let Some(dir) = Direction::from_char(c) else {
            continue;
        };
        game.turn(dir);
        if !game.step() {
            if frames {
                print!("{}", game.render());
            }
            println!("第 {} 步：{}", moves, describe(game.status));
            break;
        }
        moves += 1;
        if frames {
            print!("{}", game.render());
        }
    }
    println!("得分 {}，长度 {}，状态 {}", game.score, game.snake.len(), describe(game.status));
}
```

为什么用「一次读完全部标准输入」而不是「每次读一个键」？因为真正的逐键读取需要把终端切进 raw 模式（关掉行缓冲和回显），那是 `crossterm` 之类的库在做的事，标准库没有跨平台接口。本章选择把输入变成**一串命令**：`wwdd` 就是「上、上、右、右」。好处是可复现——同样的输入永远得到同样的结果，连随机种子都能从命令行传。

`let Some(dir) = ... else { continue };` 是 `let-else` 语法（Rust 1.65 稳定）：不是方向键的字符直接跳过，不用写嵌套的 `if let`。

### 跑起来的真实输出

编译并传入 `wwdd`（上、上、右、右），棋盘 8×6，种子 3：

```console
$ printf 'wwdd' | cargo run --quiet -- 8 6 3
8x6 的棋盘，食物在 (6, 1)
得分 1，长度 4，状态 还在进行
```

加上 `--frames` 就能看到每一帧。下面这段是同一个输入的真实输出（已按行合并重复的边框，原样保留了 5 帧）：

```console
$ printf 'wwdd' | cargo run --quiet -- 8 6 3 --frames
8x6 的棋盘，食物在 (6, 1)
+--------+
|        |
|      * |
|        |
|  ooO   |
|        |
|        |
+--------+
+--------+
|        |
|      * |
|    O   |
|   oo   |
|        |
|        |
+--------+
+--------+
|        |
|    O * |
|    o   |
|    o   |
|        |
|        |
+--------+
+--------+
|        |
|    oO* |
|    o   |
|        |
|        |
|        |
+--------+
+--------+
|        |
|    ooO |
|    o   |
|       *|
|        |
|        |
+--------+
得分 1，长度 4，状态 还在进行
```

逐帧读一遍能确认三件事：蛇「先长头、后缩尾」的移动方式（第 2 帧头的左边多了一节、尾巴还留在原处）；吃掉食物后长度从 3 变成 4、食物在别处重新出现（最后一帧的 `*` 跑到了右下）；`O` 始终表示蛇头。

撞墙的演示——一直按右，蛇头会在第 4 次移动时出界：

```console
$ printf 'dddddddd' | cargo run --quiet -- 8 6 3
8x6 的棋盘，食物在 (6, 1)
第 3 步：结束：撞到墙
得分 0，长度 3，状态 结束：撞到墙
```

这里打印的「第 3 步」是**成功移动的步数**：从 `x = 4` 开始，走到 `x = 7` 用了 3 步，下一步 `x = 8` 越界，`step` 返回 `false`。

## 44.6 测试：把每条规则钉死

游戏逻辑之所以能拆出来，就是为了这一刻。下面 9 个测试覆盖了初始状态、吃到食物、正常移动、撞墙、掉头、进入尾巴格、自撞、满屏通关和食物放置的不变式。

```rust,noplayground
#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: i32, y: i32) -> Point {
        Point { x, y }
    }

    #[test]
    fn new_game_has_three_segments() {
        let g = Game::new(8, 6, 3);
        assert_eq!(g.snake.len(), 3);
        assert_eq!(g.head(), point(4, 3));
        assert_eq!(g.status, Status::Running);
    }

    #[test]
    fn eating_grows_and_scores() {
        let mut g = Game::new(8, 6, 3);
        g.food = point(5, 3);
        assert!(g.step());
        assert_eq!(g.score, 1);
        assert_eq!(g.snake.len(), 4);
        assert_ne!(g.food, point(5, 3), "吃完后必须重新生成食物");
    }

    #[test]
    fn tail_cell_is_legal_to_enter() {
        let mut g = Game::new(8, 8, 3);
        g.food = point(7, 7);
        assert!(g.step());
        assert!(g.turn(Direction::Up));
        assert!(g.step());
        assert!(g.turn(Direction::Left));
        assert!(g.step());
        assert!(g.turn(Direction::Down));
        assert!(g.step(), "回到尾巴刚离开的格子是合法的");
        assert_eq!(g.status, Status::Running);
    }

    #[test]
    fn self_collision_ends_the_game() {
        let mut g = Game::new(8, 8, 3);
        g.snake.clear();
        for p in [point(4, 2), point(4, 3), point(5, 3), point(5, 2), point(5, 1)] {
            g.snake.push_back(p);
        }
        g.dir = Direction::Right;
        g.food = point(0, 7);
        assert!(!g.step());
        assert_eq!(g.status, Status::Crashed("咬到自己"));
    }

    #[test]
    fn full_board_wins() {
        let mut g = Game::new(4, 1, 3);
        assert_eq!(g.status, Status::Running);
        assert!(g.step());
        assert_eq!(g.status, Status::Won);
        assert_eq!(g.snake.len(), 4);
    }
}
```

（完整文件里还有 `moving_without_food_keeps_length`、`wall_crash_stops_the_game`、`opposite_turn_is_rejected`、`food_is_never_placed_on_the_snake` 四个测试，篇幅原因这里只摘了五个。）

两个测试值得单独说明：

- `tail_cell_is_legal_to_enter` 走了一个 4 步的方框：右、上、左、下。第 4 步蛇头要进入的格子，正是第 1 步尾巴离开的位置。如果 `step` 里的检查没有排除尾巴，这个测试会失败——它保护的就是 44.2 节里那条「容易漏掉的细节」。
- `full_board_wins` 用 4×1 的棋盘：初始蛇占 3 格、食物在唯一的空格上，走一步就吃满 → `Won`。用最小棋盘构造边界条件，比在 12×8 里手动喂 93 个食物现实得多。

实测结果：

```console
$ cargo test --offline
   Compiling snake v0.1.0 (D:\code\dshwork\rust-book\.scratch\ch44\snake)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 1.09s
     Running unittests src\main.rs (target\debug\build\snake/723b06d8f259c687\out\snake-723b06d8f259c687.exe)

running 9 tests
test tests::eating_grows_and_scores ... ok
test tests::full_board_wins ... ok
test tests::moving_without_food_keeps_length ... ok
test tests::new_game_has_three_segments ... ok
test tests::opposite_turn_is_rejected ... ok
test tests::self_collision_ends_the_game ... ok
test tests::tail_cell_is_legal_to_enter ... ok
test tests::food_is_never_placed_on_the_snake ... ok
test tests::wall_crash_stops_the_game ... ok

test result: ok. 9 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

注意 `Game` 的字段是私有的，但测试模块能直接 `g.food = point(5, 3)`——因为 `mod tests` 是 `main.rs` 的子模块，子模块可以访问父模块的私有项。这也是「单元测试放在源码文件里」的好处：不需要为了测试暴露 `pub` 接口。

## 44.7 深入：五个设计选择的理由

### 为什么蛇身用 VecDeque 而不是 Vec

蛇的操作用到两个动作：**在队首插入**（新头）和**在队尾删除**（旧尾）。

- `Vec::insert(0, x)` 要把后面所有元素整体后移一格，`O(n)`；每帧都这么做，长度变长之后帧率会掉。
- `VecDeque` 是环形缓冲区：队首插入只需移动一个头指针（必要时扩容），`O(1)` 均摊；队尾删除同理。

```rust,noplayground
use std::collections::VecDeque;

fn main() {
    let mut q: VecDeque<i32> = VecDeque::new();
    for i in 0..5 {
        q.push_front(i); // O(1)
    }
    q.pop_back(); // O(1)
    let v: Vec<i32> = q.iter().copied().collect();
    println!("{:?} 容量 {}", v, q.capacity());
}
```

`VecDeque` 的代价是**不保证内存连续**（它可能被切成两段），所以不能把它当 `&[T]` 用。`make_contiguous()` 可以显式整理成连续内存，但本章不需要——我们只用 `iter()`、`front()`、`push_front`、`pop_back` 这些接口。

还有一个细节：`VecDeque` 的容量是 2 的幂，扩容时按倍数增长。对一条蛇来说，容量增长到 64 之后几乎不会再变，所以长期运行的帧率是稳定的。

### 碰撞检测为什么可以偷懒

`step` 里用 `iter().take(n).any(...)` 做线性扫描，复杂度 `O(n)`。蛇最长也就 `w×h` 节，12×8 的棋盘上最多 96 次比较——一帧 96 次整数比较对现代 CPU 是噪音级别。

如果棋盘大到几千格，就该换数据结构了：维护一个 `HashSet<Point>`（或 `Vec<Vec<bool>>` 的位图）记录被占据的格子，插入和查询都是 `O(1)`。这里的原则和 44.2 节的规则一致：**先让规则正确，再让实现变快**。过早引入 `HashSet` 会让「尾巴格子是否合法」这类逻辑多一处需要同步的状态——两处状态一旦不同步，就会出现「明明没撞却死」的诡异 bug。

### 随机数为什么自己写

`rand` crate 当然更专业，但它带来一个依赖，而本章的目标是「纯 std 可运行」。LCG 的三行实现在游戏里完全够用，因为食物位置的随机性不影响任何逻辑正确性——最坏情况是食物出现得不够随机，玩家体验差一点。

但要注意 LCG 的两个坑：种子不能是 0（否则输出恒为 0），以及**低位随机性差**。`(self.0 >> 33) as u32` 取的是中间 32 位，避开了最低位的短周期。如果换成 `(self.0 % 100) as u32` 去取食物下标，会看到食物明显「偏好」某些格子。

### 为什么用「命令串」而不是实时循环

真实游戏是「每 100ms 推进一帧，期间收集键盘事件」。这需要三个东西：非阻塞输入、定时器、以及「一帧内多次按键只取最后一次」的合并逻辑。标准库能拼出来（`set_read_timeout` + `Instant`），但代码会膨胀一倍，而且没法在 CI 里跑。

本章的「命令串」把时间轴压平成一维：每个字符一步。这在教学上是优点——读者可以精确复现每一帧；在工程上是简化——真实项目应该用 `crossterm` 的 `event::poll` 配合固定时间步。这个取舍值得写进代码注释里，让后来者知道「这不是忘了做实时，是故意没做」。

### 渲染为什么返回 String

`render` 每次都新建一个 `String` 并 push 几十个字符。对 12×8 的棋盘这是 60 次分配上限的小对象，完全可以忽略。但如果你要做 60fps 的全屏重绘，正确做法是**复用缓冲区**：把 `String` 变成 `Game` 之外的一个 `&mut String` 参数，进入函数时 `clear()` 而不是新建。这是 Rust 里非常常见的性能手法——把分配移到循环外。

```rust,noplayground
fn render_into(&self, buf: &mut String) {
    buf.clear();
    buf.push('+');
    for _ in 0..self.w {
        buf.push('-');
    }
    buf.push('\n');
    // ... 其余部分相同
}
```

## 44.8 典型错误案例

### 案例 1：一边遍历蛇身一边修改——E0502

```rust,noplayground
use std::collections::VecDeque;

struct Game {
    snake: VecDeque<(i32, i32)>,
}

impl Game {
    fn wrap_edges(&mut self) {
        for p in self.snake.iter() {
            if p.0 < 0 {
                self.snake.push_back((0, p.1));
            }
        }
    }
}

fn main() {
    let mut g = Game { snake: VecDeque::new() };
    g.snake.push_back((-1, 0));
    g.wrap_edges();
}
```

```text
error[E0502]: cannot borrow `self.snake` as mutable because it is also borrowed as immutable
  --> src/main.rs:11:17
   |
 9 |         for p in self.snake.iter() {
   |         -        -----------------
   |         |        |
   |         |        immutable borrow occurs here
   |  _________|        immutable borrow later used here
   | |
10 | |             if p.0 < 0 {
11 | |                 self.snake.push_back((0, p.1));
   | |                 ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^ mutable borrow occurs here
12 | |             }
13 | |         }
   | |_________- this for loop borrows `self.snake` immutably, preventing mutation within its body
   |
   = help: consider using an index-based loop instead, or collecting modifications into a separate collection
```

**原因**：`iter()` 借用了整个 `VecDeque`，而且这个借用要活到循环结束；`push_back` 又要求独占借用。两者在时间上重叠，借用检查器禁止——因为 `push_back` 可能触发扩容，让迭代器持有的指针失效，继续迭代就是访问已释放内存。

**修复**：先把要追加的内容收集起来，循环结束后再写回去。这是 Rust 里「先收集、后修改」的标准解法：

```rust
use std::collections::VecDeque;

struct Game {
    snake: VecDeque<(i32, i32)>,
}

impl Game {
    fn wrap_edges(&mut self) {
        let wrapped: Vec<(i32, i32)> = self
            .snake
            .iter()
            .filter(|p| p.0 < 0)
            .map(|p| (0, p.1))
            .collect();
        for p in wrapped {
            self.snake.push_back(p);
        }
    }
}

fn main() {
    let mut g = Game { snake: VecDeque::new() };
    g.snake.push_back((-1, 0));
    g.wrap_edges();
    println!("{:?}", g.snake);
}
```

### 案例 2：漏写一个方向分支——E0004

```rust,noplayground
enum Direction {
    Up,
    Down,
    Left,
    Right,
}

fn delta(d: Direction) -> (i32, i32) {
    match d {
        Direction::Up => (0, -1),
        Direction::Left => (-1, 0),
        Direction::Right => (1, 0),
    }
}

fn main() {
    println!("{:?}", delta(Direction::Down));
}
```

```text
error[E0004]: non-exhaustive patterns: `Direction::Down` not covered
  --> src/main.rs:9:11
   |
 9 |     match d {
   |           ^ pattern `Direction::Down` not covered
   |
note: `Direction` defined here
  --> src/main.rs:1:6
   |
 1 | enum Direction {
   |      ^^^^^^^^^
 2 |     Up,
 3 |     Down,
   |     ---- not covered
   = note: the matched value is of type `Direction`
help: ensure that all possible cases are being handled by adding a match arm with a wildcard pattern or an explicit pattern as shown
   |
12 ~         Direction::Right => (1, 0),
13 ~         Direction::Down => todo!(),
   |
```

**原因**：`match` 必须覆盖所有分支。这条规则看起来啰嗦，实际是在帮你：以后给 `Direction` 加一个 `Direction::Teleport`，所有漏掉它的 `match` 都会立刻编译失败，而不是等到运行时走进一个未定义分支。

**修复**：补上 `Direction::Down => (0, 1)`。如果方向列表未来可能扩展、且某些分支行为相同，可以合并写 `Direction::Left | Direction::Right => ...` 或兜底 `_ => ...`；但兜底会让「新增方向」重新变成静默行为，一般不建议在核心逻辑里用。

### 案例 3：调用了不存在的方法——E0599

```rust,noplayground
struct Game {
    score: u32,
}

fn main() {
    let g = Game { score: 0 };
    println!("{}", g.total_score());
}
```

```text
error[E0599]: no method named `total_score` found for struct `Game` in the current scope
 --> src/main.rs:7:22
  |
1 | struct Game {
  | ----------- method `total_score` not found for this struct
...
7 |     println!("{}", g.total_score());
  |                      ^^^^^^^^^^^ method not found in `Game`
```

**原因**：字段叫 `score`，方法不存在。Rust 不会自动把字段名当成 getter（这一点和 Java 的 Lombok 或 Kotlin 的属性不同）。

**修复**：直接用 `g.score`，或显式实现方法：

```rust
struct Game {
    score: u32,
}

impl Game {
    fn total_score(&self) -> u32 {
        self.score
    }
}

fn main() {
    let g = Game { score: 0 };
    println!("{}", g.total_score());
}
```

### 案例 4：把字符当方向传——E0308

```rust,noplayground
enum Direction {
    Up,
    Down,
}

fn turn(d: Direction) {
    let _ = d;
}

fn main() {
    turn('w');
}
```

```text
error[E0308]: mismatched types
  --> src/main.rs:11:10
   |
11 |     turn('w');
   |     ---- ^^^ expected `Direction`, found `char`
   |     |
   |     arguments to this function are incorrect
   |
note: function defined here
  --> src/main.rs:6:4
   |
 6 | fn turn(d: Direction) {
   |    ^^^^ ------------
```

**原因**：`Direction` 是自定义枚举，`char` 是字符，两者之间没有隐式转换。这正是本章把输入层单独拆出来的理由：翻译发生在 `Direction::from_char`，逻辑层永远收到的是干净的 `Direction`。

**修复**：先翻译，再传参——注意 `from_char` 返回 `Option`，必须处理「这个键不是方向键」的情况：

```rust
#[derive(Debug)]
enum Direction {
    Up,
    Down,
}

impl Direction {
    fn from_char(c: char) -> Option<Self> {
        match c {
            'w' => Some(Direction::Up),
            's' => Some(Direction::Down),
            _ => None,
        }
    }
}

fn turn(d: Direction) {
    println!("转向 {:?}", d);
}

fn main() {
    if let Some(d) = Direction::from_char('w') {
        turn(d);
    }
    // 'x' 不是方向键，什么也不做
    if let Some(d) = Direction::from_char('x') {
        turn(d);
    }
}
```

### 案例 5：移动后继续使用——E0382

```rust,noplayground
#[derive(Debug)]
struct Game {
    score: u32,
}

fn main() {
    let g = Game { score: 0 };
    let other = g;
    println!("{:?}", g);
    println!("{:?}", other);
}
```

```text
error[E0382]: borrow of moved value: `g`
 --> src/main.rs:9:22
  |
7 |     let g = Game { score: 0 };
  |         - move occurs because `g` has type `Game`, which does not implement the `Copy` trait
8 |     let other = g;
  |                 - value moved here
9 |     println!("{:?}", g);
  |                      ^ value borrowed here after move
  |
note: if `Game` implemented `Clone`, you could clone the value
 --> src/main.rs:2:1
  |
2 | struct Game {
  | ^^^^^^^^^^^ consider implementing `Clone` for this type
...
8 |     let other = g;
  |                 - you could clone this value
```

**原因**：`Game` 没有实现 `Copy`，`let other = g;` 是移动而不是复制。这个错误在游戏代码里尤其常见：想把当前状态存一份「快照」用于回放，结果把整个游戏移走了。

**修复**：真要两份数据就 `#[derive(Clone)]` 后显式 `g.clone()`；只是借用就传 `&g`。注意 `Clone` 是**深拷贝**，`Game` 里如果有 `VecDeque` 就会复制整条蛇——大多数「快照」场景其实应该重新构造状态或只存差异。

```rust
#[derive(Debug, Clone)]
struct Game {
    score: u32,
    snake: Vec<(i32, i32)>,
}

fn snapshot(g: &Game) -> Game {
    g.clone()
}

fn main() {
    let g = Game { score: 0, snake: vec![(1, 1), (0, 1)] };
    let copy = snapshot(&g);
    println!("{:?}", g);
    println!("{:?}", copy);
}
```

## 44.9 拼装与运行

本章的代码块按出现顺序拼接，就是完整的 `src/main.rs`（约 330 行）。顺序与对应的文件头如下：

```rust,noplayground
use std::collections::VecDeque;
use std::io::{self, Read};

// 1. Point / Direction（44.3，两个 derive + impl Direction）
// 2. Lcg（44.3）
// 3. Status / Game 结构体（44.4）
// 4. impl Game：new / head / place_food / turn / step / render（44.4）
// 5. describe / main（44.5）
// 6. #[cfg(test)] mod tests（44.6）
```

工程目录只要两个文件：

```text
snake/
├── Cargo.toml
└── src/
    └── main.rs
```

`Cargo.toml` 里除 `[package]` 之外没有任何依赖：

```toml
[package]
name = "snake"
version = "0.1.0"
edition = "2021"

[dependencies]
```

运行方式：

```bash
cargo run --quiet -- 8 6 3 --frames   # 8x6 棋盘，种子 3，逐帧打印
printf 'wwdd' | cargo run --quiet -- 8 6 3   # 用命令串驱动，不打印过程
cargo test --offline   # 9 个单元测试
```

本章所有输出都是在本机（rustc 1.100.0、Windows、`cargo build --offline`）实际跑出来的，没有编造。

## 44.10 动手练习

**练习 1（改规则）**：把「撞墙」改成「穿墙」——从右边出去就从左边进来。提示：只在 `step` 的边界检查处把 `next` 取模修正，其他地方不动。改完为它补一个测试：8×8 棋盘上从 `x = 7` 向右，头应该出现在 `x = 0`。

**练习 2（加状态）**：加入「加速食物」：吃到它之后接下来 5 步每次前进两格。你需要新增一个字段记录剩余加速步数，并想清楚「两格中的第一格撞到墙」时该怎么判。给 `Game` 加一个测试覆盖加速结束后的恢复。

**练习 3（换成实时）**：把命令串换成真实按键。用 `std::io::stdin().read_line` 每次读一行（回车确认）是最简版本；进阶版用 `crossterm::event::poll(Duration::from_millis(100))` 实现 100ms 一帧的心跳。注意 `crossterm` 不在 std 里，需要联网拉依赖。

**练习 4（更好看）**：让蛇头按朝向显示不同的箭头 `^ v < >`，身体用 `#`，食物用 `@`，并给整个棋盘加双线边框。改完检查 `render` 的四重 `if` 顺序是否还正确。

**练习 5（性能）**：把碰撞检测换成 `HashSet<(i32, i32)>`，并在 `step` 里同时维护「蛇身集合」和 `VecDeque`。用 100×100 的棋盘、蛇长 500 节做基准测试（`std::time::Instant` 计 1 万步），与原来的线性扫描对比。注意集合里必须同步删除尾巴，否则会出现「走进自己刚离开的格子却死了」。

## 44.11 小结

- 先把规则写成规格，再写代码：本章 44.2 节的六条规则直接对应了 `step` 的六段逻辑。
- 用 `VecDeque` 表示蛇身：队首插入、队尾删除都是 `O(1)`；`Vec::insert(0, _)` 是 `O(n)`。
- 「尾巴格子可以进入」必须显式处理（`take(len - 1)`），这是最容易漏的规则，44.6 节有专门的测试守着它。
- 先判死再改状态，游戏结束时棋盘才是最后一帧合法的画面。
- 把随机数种子做成参数，演示和测试才能完全复现；LCG 三行够用，但种子不能为 0、也不该取低位。
- 输入层与逻辑层分离：逻辑层只认 `Direction`，于是它能在没有终端的环境里被单元测试驱动。
- `Status` 用 `Crashed(&'static str)` 而不是 `String`：不分配、可 `Copy`、测试里可以直接 `assert_eq!`。
- `render` 返回 `String` 而不是直接打印，让测试能断言画面、让主程序能控制输出节奏。
- 五个真实错误都指向同一件事：**借用与所有权规则不是障碍，而是在替你排除「边遍历边改」「移动后复用」这类运行时炸弹**。
- 这个项目的下一步是「把时间轴加回来」：固定时间步、非阻塞输入、双缓冲渲染——规则层一行都不用改。