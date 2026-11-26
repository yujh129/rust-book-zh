//! RESP 请求解析（第 42.4 节）：纯函数，只吃字节，不碰 socket。
//!
//! 三态返回分别对应三种网络状况：
//! - Ok(Some((args, used)))：拿到一条完整命令，used 是本次消耗的字节数；
//! - Ok(None)：数据不够，合法但没读完，继续读；
//! - Err(_)：字节流不符合协议，已经无法定位下一条命令的边界，只能回错误再断开。

/// 在 buf 的 from.. 范围里找第一个 CRLF 的 \r 下标。
///
/// 搜索范围是 from..len-1：\r 后面必须还有字节才可能凑成一对，
/// 否则取 i + 1 会下标越界，这是手写解析器最常见的 panic 来源。
fn find_crlf(buf: &[u8], from: usize) -> Option<usize> {
    (from..buf.len().saturating_sub(1)).find(|i| buf[*i] == b'\r' && buf[*i + 1] == b'\n')
}

/// 从缓冲区头部切出一条命令。
///
/// 成功时返回 (参数列表, 消耗字节数)，调用方负责从缓冲区里切掉这 used 个字节；
/// 数据不足返回 Ok(None)，绝不报错。
pub fn parse_command(buf: &[u8]) -> Result<Option<(Vec<String>, usize)>, String> {
    if buf.is_empty() {
        return Ok(None);
    }
    if buf[0] != b'*' {
        // inline 形式：一行，空格分隔，方便用 telnet / nc 手测。
        let Some(end) = find_crlf(buf, 0) else {
            return Ok(None);
        };
        let line = std::str::from_utf8(&buf[..end]).map_err(|_| String::from("非法 UTF-8"))?;
        let parts: Vec<String> = line.split_whitespace().map(String::from).collect();
        if parts.is_empty() {
            return Err(String::from("空命令"));
        }
        return Ok(Some((parts, end + 2)));
    }
    // RESP 数组形式：*N 换行，后面跟 N 个 $len 换行 + 正文 + CRLF。
    let Some(end) = find_crlf(buf, 0) else {
        return Ok(None);
    };
    let count: usize = std::str::from_utf8(&buf[1..end])
        .map_err(|_| String::from("非法 UTF-8"))?
        .parse()
        .map_err(|_| String::from("数组长度非法"))?;
    let mut pos = end + 2;
    if count > 1024 {
        return Err(String::from("参数过多"));
    }
    let mut args = Vec::new();
    for _ in 0..count {
        if pos >= buf.len() {
            return Ok(None);
        }
        if buf[pos] != b'$' {
            return Err(String::from("期望 bulk string"));
        }
        let Some(hdr_end) = find_crlf(buf, pos) else {
            return Ok(None);
        };
        let len: usize = std::str::from_utf8(&buf[pos + 1..hdr_end])
            .map_err(|_| String::from("非法 UTF-8"))?
            .parse()
            .map_err(|_| String::from("bulk 长度非法"))?;
        if len > 8 * 1024 * 1024 {
            return Err(String::from("单个参数过长"));
        }
        let body_start = hdr_end + 2;
        // 长度以及尾部分隔符都要计入本帧。
        let Some(body_end) = body_start.checked_add(len) else {
            return Err(String::from("bulk 长度非法"));
        };
        // 正文后面还有两个字节的 CRLF，少一个字节都算没读完。
        let frame_end = body_end.checked_add(2).ok_or("bulk 长度非法")?;
        if buf.len() < frame_end {
            return Ok(None);
        }
        if &buf[body_end..frame_end] != b"\r\n" {
            return Err(String::from("bulk 缺少 CRLF"));
        }
        let s = std::str::from_utf8(&buf[body_start..body_end])
            .map_err(|_| String::from("非法 UTF-8"))?;
        args.push(s.to_string());
        pos = frame_end;
    }
    Ok(Some((args, pos)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_inline_command() {
        let (cmd, used) = parse_command(b"PING\r\n").unwrap().unwrap();
        assert_eq!(cmd, vec![String::from("PING")]);
        assert_eq!(used, 6);
    }

    #[test]
    fn parses_resp_array() {
        let raw = b"*3\r\n$3\r\nSET\r\n$3\r\nfoo\r\n$3\r\nbar\r\n";
        let (cmd, used) = parse_command(raw).unwrap().unwrap();
        assert_eq!(
            cmd,
            vec![String::from("SET"), String::from("foo"), String::from("bar")]
        );
        assert_eq!(used, raw.len());
    }

    #[test]
    fn rejects_invalid_bulk_terminator_and_oversized_frames() {
        assert!(parse_command(b"*1\r\n$4\r\nPING!!").is_err());
        assert!(parse_command(b"*1025\r\n").is_err());
        assert!(parse_command(b"*1\r\n$8388609\r\n").is_err());
        assert!(parse_command(b"*1\r\n$4\r\nPING\r").unwrap().is_none());
    }

    #[test]
    fn incomplete_input_returns_none() {
        assert!(parse_command(b"*2\r\n$3\r\nSE").unwrap().is_none());
        assert!(parse_command(b"*2\r\n$3\r\nSET\r\n$3\r\nfo").unwrap().is_none());
    }

    #[test]
    fn two_commands_in_one_buffer() {
        let raw = b"*1\r\n$4\r\nPING\r\n*1\r\n$4\r\nPING\r\n";
        let (first, used) = parse_command(raw).unwrap().unwrap();
        assert_eq!(first, vec![String::from("PING")]);
        let (second, _) = parse_command(&raw[used..]).unwrap().unwrap();
        assert_eq!(second, vec![String::from("PING")]);
    }
}
