//! minigrep 的命令行外壳：只做三件事——收集参数、调用库、把结果翻译成退出码。

use std::env;
use std::process::ExitCode;

use minigrep::{run, Config};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().collect();
    let config = match Config::build(&args) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("参数错误：{}", e);
            return ExitCode::from(2);
        }
    };

    match run(config) {
        Ok(0) => {
            eprintln!("没有找到匹配行");
            ExitCode::from(1)
        }
        Ok(_) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("读取错误：{}", e);
            ExitCode::from(3)
        }
    }
}
