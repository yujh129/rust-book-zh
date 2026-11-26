//! RESP 回复编码（第 42.5 节）。
//!
//! 书上说「一个枚举管住五种形态」，但协议表里的第五种是数组（星号开头），
//! 服务端自己从不回数组，所以这个枚举实际是四种：状态、错误、整数、bulk。

#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Simple(String),
    Bulk(Option<String>),
    Integer(i64),
    Error(String),
}

impl Reply {
    /// 把回复编成 RESP 线格式字节。编码只有这一处实现。
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::new();
        match self {
            Reply::Simple(s) => {
                out.extend_from_slice(format!("+{}\r\n", s).as_bytes());
            }
            Reply::Bulk(Some(s)) => {
                // s.len() 是字节数，RESP 的长度单位也是字节，这里恰好对上。
                // 长度头用 format 拼出来，百分号和花括号相邻会与测试框架的
                // 模板语法冲突，所以把美元符号单独当参数传进去，输出一致。
                out.extend_from_slice(format!("{}{}\r\n{}\r\n", "$", s.len(), s).as_bytes());
            }
            Reply::Bulk(None) => out.extend_from_slice(b"$-1\r\n"),
            Reply::Integer(n) => out.extend_from_slice(format!(":{}\r\n", n).as_bytes()),
            Reply::Error(e) => out.extend_from_slice(format!("-{}\r\n", e).as_bytes()),
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replies_have_correct_wire_format() {
        assert_eq!(Reply::Simple(String::from("OK")).to_bytes(), b"+OK\r\n".to_vec());
        assert_eq!(Reply::Bulk(None).to_bytes(), b"$-1\r\n".to_vec());
        assert_eq!(
            Reply::Bulk(Some(String::from("hi"))).to_bytes(),
            b"$2\r\nhi\r\n".to_vec()
        );
        assert_eq!(Reply::Integer(42).to_bytes(), b":42\r\n".to_vec());
        assert_eq!(Reply::Error(String::from("bad")).to_bytes(), b"-bad\r\n".to_vec());
    }
}
