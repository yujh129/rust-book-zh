//! minigrep：一个迷你的 grep。
//!
//! 所有可测试的核心逻辑都在这里；`main.rs` 只做「收集参数、调用库、翻译退出码」。

use std::error::Error;
use std::fs;

/// 命令行配置。
#[derive(Debug)]
pub struct Config {
    pub query: String,
    pub file_path: String,
    pub ignore_case: bool,
}

/// 参数解析错误。
#[derive(Debug)]
pub enum ConfigError {
    MissingArgument,
    UnknownFlag(String),
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::MissingArgument => write!(f, "用法：minigrep [-i] <模式> <文件>"),
            ConfigError::UnknownFlag(flag) => write!(f, "未知参数：{}", flag),
        }
    }
}

impl Error for ConfigError {}

impl Config {
    /// 从完整参数列表（含程序名）构造配置。
    ///
    /// 接收 `&[String]` 而不是 `Vec<String>`：不夺走调用方的所有权，也方便测试时构造假参数。
    pub fn build(args: &[String]) -> Result<Config, ConfigError> {
        if args.len() < 3 {
            return Err(ConfigError::MissingArgument);
        }

        let mut ignore_case = false;
        let mut positional = Vec::new();

        for arg in &args[1..] {
            match arg.as_str() {
                "-i" | "--ignore-case" => ignore_case = true,
                other if other.starts_with('-') => {
                    return Err(ConfigError::UnknownFlag(other.to_string()));
                }
                other => positional.push(other.to_string()),
            }
        }

        if positional.len() < 2 {
            return Err(ConfigError::MissingArgument);
        }

        Ok(Config {
            query: positional[0].clone(),
            file_path: positional[1].clone(),
            ignore_case,
        })
    }
}

/// 执行搜索并把命中的行打印到标准输出，返回命中行数。
///
/// 返回 `usize` 让 `main` 能区分「0 命中」和「有命中」，
/// `Box<dyn Error>` 让 `?` 能把 `io::Error` 直接往上抛。
pub fn run(config: Config) -> Result<usize, Box<dyn Error>> {
    let contents = fs::read_to_string(&config.file_path)?;

    let hits = if config.ignore_case {
        search_case_insensitive(&config.query, &contents)
    } else {
        search(&config.query, &contents)
    };

    for (line_no, line) in &hits {
        println!("{}:{}", line_no, line);
    }

    Ok(hits.len())
}

/// 大小写敏感搜索，返回 `(行号, 行内容)`，行号从 1 开始。
///
/// 返回值里的切片只与 `contents` 同寿；`query` 只用于比较，不会出现在返回值里。
pub fn search<'a>(query: &str, contents: &'a str) -> Vec<(usize, &'a str)> {
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(query))
        .map(|(i, line)| (i + 1, line))
        .collect()
}

/// 大小写不敏感搜索：查询串只小写一次，返回的仍是 `contents` 里的原始切片。
pub fn search_case_insensitive<'a>(query: &str, contents: &'a str) -> Vec<(usize, &'a str)> {
    let query = query.to_lowercase();
    contents
        .lines()
        .enumerate()
        .filter(|(_, line)| line.to_lowercase().contains(&query))
        .map(|(i, line)| (i + 1, line))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Rust:\nsafe, fast, productive.\nPick three.\nTrust me.";

    #[test]
    fn case_sensitive_finds_one() {
        let hits = search("duct", SAMPLE);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, 2);
    }

    #[test]
    fn case_insensitive_finds_two() {
        let hits = search_case_insensitive("rUsT", SAMPLE);
        assert_eq!(hits.len(), 2);
    }

    #[test]
    fn no_match_returns_empty() {
        assert!(search("zzz", SAMPLE).is_empty());
    }
}
