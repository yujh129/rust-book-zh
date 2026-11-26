#!/usr/bin/env python3
"""逐个构建并测试 projects/ 下的项目工程。

用法：
    python tools/check_projects.py              # 需要联网（或用本地 cargo 缓存）
    python tools/check_projects.py --offline    # 只用本地缓存，本机推荐
    python tools/check_projects.py --only 42    # 只跑目录名里含该子串的工程
"""

import argparse
import os
import subprocess
import sys
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
PROJECTS = os.path.join(ROOT, "projects")


def find_projects(only):
    found = []
    for name in sorted(os.listdir(PROJECTS)):
        path = os.path.join(PROJECTS, name)
        if not os.path.isfile(os.path.join(path, "Cargo.toml")):
            continue
        if only and only not in name:
            continue
        found.append((name, path))
    return found


def run_cargo(cwd, offline):
    cmd = ["cargo", "test", "--color", "never"]
    if offline:
        cmd.append("--offline")
    started = time.monotonic()
    proc = subprocess.run(cmd, cwd=cwd, capture_output=True, text=True,
                          encoding="utf-8", errors="replace")
    return proc.returncode, (proc.stdout or "") + (proc.stderr or ""), time.monotonic() - started


def count_passed(output):
    total = 0
    for line in output.splitlines():
        if line.startswith("test result: ok."):
            try:
                total += int(line.split("ok.")[1].split("passed")[0].strip())
            except (IndexError, ValueError):
                pass
    return total


def main():
    parser = argparse.ArgumentParser(description="构建并测试 projects/ 下的每个工程")
    parser.add_argument("--offline", action="store_true", help="给 cargo 加 --offline，只用本地缓存")
    parser.add_argument("--only", help="只跑名字包含该子串的工程，例如 42")
    args = parser.parse_args()

    projects = find_projects(args.only)
    if not projects:
        print("projects/ 下没有找到工程")
        return 1

    print("{} 个工程，cargo test{}\n".format(len(projects), " --offline" if args.offline else ""))
    failed = []
    total_passed = 0
    for name, path in projects:
        code, output, elapsed = run_cargo(path, args.offline)
        passed = count_passed(output)
        if code == 0:
            total_passed += passed
            print("  ok   {:<18} {:>3} 个测试  {:.1f}s".format(name, passed, elapsed))
        else:
            failed.append(name)
            print("  FAIL {:<18} {:.1f}s".format(name, elapsed))
            errors = [line for line in output.splitlines() if line.startswith("error")]
            for line in (errors or output.splitlines()[-6:])[:6]:
                print("       " + line)

    if failed:
        print("\n{} 个工程失败：{}".format(len(failed), ", ".join(failed)))
        return 1
    print("\n全部通过：{} 个工程，{} 个测试".format(len(projects), total_passed))
    return 0


if __name__ == "__main__":
    sys.exit(main())
