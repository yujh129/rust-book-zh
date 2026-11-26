//! 集成测试：通过真实进程验证退出码与 stderr，`main` 里的行为只能这样测。

use std::fs;
use std::process::Command;

const POEM: &str = "tests/fixtures/poem.txt";

#[test]
fn exits_with_1_when_nothing_matches() {
    let output = Command::new(env!("CARGO_BIN_EXE_minigrep"))
        .args(["zzz", POEM])
        .output()
        .expect("启动失败");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("没有找到匹配行"));
}

#[test]
fn exits_with_2_on_bad_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_minigrep"))
        .args(["--bad", "poem.txt"])
        .output()
        .expect("启动失败");

    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn exits_with_0_when_matching() {
    let output = Command::new(env!("CARGO_BIN_EXE_minigrep"))
        .args(["duct", POEM])
        .output()
        .expect("启动失败");

    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(stdout.trim_end(), "2:safe, fast, productive.");
}

#[test]
fn exits_with_3_on_non_utf8_file() {
    // env!("CARGO_TARGET_TMPDIR") 是 cargo 为集成测试提供的可写临时目录。
    let mut path = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR"));
    path.push(format!("minigrep-bad-{}.bin", std::process::id()));
    fs::write(&path, [0xffu8, 0xfeu8]).expect("写临时文件失败");

    let output = Command::new(env!("CARGO_BIN_EXE_minigrep"))
        .args(["x", path.to_str().expect("临时路径不是 UTF-8")])
        .output()
        .expect("启动失败");

    let _ = fs::remove_file(&path);
    assert_eq!(output.status.code(), Some(3));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("读取错误"));
}
