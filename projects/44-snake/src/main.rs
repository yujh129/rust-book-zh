//! 第 44 章项目：贪吃蛇——把状态机写进类型里
//!
//! 完整实现见《Rust 学习指南》第 44 章。设计要点：
//!
//! - 逻辑层只认 Direction，输入层把 w/a/s/d 翻译成方向；
//! - 一次读完 stdin 的「命令串」（每个字符一步），因此完全可复现、可在 CI 里跑；
//! - 零第三方依赖：随机数用可复现的 LCG，渲染返回 String。
//!
//! 这不是「忘了做实时」：真实游戏需要非阻塞输入、定时器和帧内按键合并，
//! 标准库能做但代码会翻倍且无法自动化验证，所以本章刻意把时间轴压平成一维。

use std::collections::VecDeque;
use std::io::{self, Read};

// ---------------------------------------------------------------------------
// 1. Point / Direction（44.3）
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 2. Lcg（44.3）：4 行线性同余生成器，种子可复现
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 3. Status / Game 结构体（44.4）
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// 4. impl Game：new / head / place_food / turn / step / render（44.4）
// ---------------------------------------------------------------------------

impl Game {
    fn valid_size(w: i32, h: i32) -> bool {
        w >= 4 && h >= 1 && w.checked_mul(h).is_some_and(|cells| cells <= 65_536)
    }

    fn new(w: i32, h: i32, seed: u64) -> Self {
        assert!(Self::valid_size(w, h), "棋盘尺寸超出范围");
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

impl Game {
    fn turn(&mut self, d: Direction) -> bool {
        if d == self.dir.opposite() {
            return false;
        }
        self.dir = d;
        true
    }
}

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
        let body_len = if eating {
            self.snake.len()
        } else {
            self.snake.len() - 1
        };
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

// ---------------------------------------------------------------------------
// 5. describe / main（44.5）
// ---------------------------------------------------------------------------

fn describe(status: Status) -> String {
    match status {
        Status::Running => String::from("还在进行"),
        Status::Won => String::from("满屏通关"),
        Status::Crashed(why) => format!("结束：{}", why),
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let frames = args.iter().any(|a| a == "--frames");
    let mut values = args.iter().map(String::as_str).filter(|a| *a != "--frames");
    let w = values.next().unwrap_or("12").parse::<i32>();
    let h = values.next().unwrap_or("8").parse::<i32>();
    let seed = values.next().unwrap_or("7").parse::<u64>();
    let (Ok(w), Ok(h), Ok(seed)) = (w, h, seed) else {
        eprintln!("用法：snake [宽 高 种子] [--frames]，参数必须是整数");
        std::process::exit(2);
    };
    if values.next().is_some() || !Game::valid_size(w, h) {
        eprintln!("宽至少 4，高至少 1，总格数最多 65536；最多接受三个位置参数");
        std::process::exit(2);
    }

    let mut input = String::new();
    io::stdin()
        .read_to_string(&mut input)
        .expect("读标准输入失败");

    let mut game = Game::new(w, h, seed);
    println!(
        "{}x{} 的棋盘，食物在 ({}, {})",
        w, h, game.food.x, game.food.y
    );
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
        if !game.turn(dir) {
            continue; // 掉头被拒绝：这一步不推进
        }
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
        if game.status == Status::Won {
            break;
        }
    }
    println!(
        "得分 {}，长度 {}，状态 {}",
        game.score,
        game.snake.len(),
        describe(game.status)
    );
}

// ---------------------------------------------------------------------------
// 6. #[cfg(test)] mod tests（44.6）：9 个覆盖边界条件的单元测试
// ---------------------------------------------------------------------------

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
    fn moving_without_food_keeps_length() {
        let mut g = Game::new(8, 6, 3);
        g.food = point(0, 0);
        assert!(g.step());
        assert_eq!(g.snake.len(), 3, "不吃食物时长度不变");
        assert_eq!(g.score, 0);
        assert_eq!(g.head(), point(5, 3));
    }

    #[test]
    fn wall_crash_stops_the_game() {
        let mut g = Game::new(8, 6, 3);
        g.food = point(0, 0);
        assert!(g.step());
        assert!(g.step());
        assert!(g.step());
        assert!(!g.step(), "第 4 次移动越界");
        assert_eq!(g.status, Status::Crashed("撞到墙"));
        assert!(!g.step(), "结束之后 step 不再推进");
    }

    #[test]
    fn opposite_turn_is_rejected() {
        let mut g = Game::new(8, 6, 3);
        assert_eq!(g.dir, Direction::Right);
        assert!(!g.turn(Direction::Left));
        assert_eq!(g.dir, Direction::Right, "掉头被拒绝后方向不变");
        assert!(g.turn(Direction::Up));
        assert_eq!(g.dir, Direction::Up);
    }

    #[test]
    fn tail_cell_is_legal_to_enter() {
        let mut g = Game::new(8, 8, 3);
        g.snake = VecDeque::from([point(4, 4), point(4, 5), point(5, 5), point(5, 4)]);
        g.dir = Direction::Up;
        g.food = point(7, 7);
        let old_tail = *g.snake.back().unwrap();
        assert!(g.turn(Direction::Right));
        assert!(g.step(), "可以进入本步将移走的尾格");
        assert_eq!(g.head(), old_tail);
        assert_eq!(g.status, Status::Running);
    }

    #[test]
    fn self_collision_ends_the_game() {
        let mut g = Game::new(8, 8, 3);
        g.snake.clear();
        for p in [
            point(4, 2),
            point(4, 3),
            point(5, 3),
            point(5, 2),
            point(5, 1),
        ] {
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

    #[test]
    fn food_is_never_placed_on_the_snake() {
        for seed in 0..200u64 {
            let mut g = Game::new(12, 8, seed);
            for _ in 0..30 {
                assert!(
                    !g.snake.contains(&g.food),
                    "seed {} 把食物放到了蛇身上",
                    seed
                );
                assert!(g.food.x >= 0 && g.food.x < g.w);
                assert!(g.food.y >= 0 && g.food.y < g.h);
                g.place_food();
            }
        }
    }
}
