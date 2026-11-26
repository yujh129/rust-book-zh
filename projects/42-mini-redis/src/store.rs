//! 存储层（第 42.3 节）：HashMap 加每次访问时的惰性过期。
//!
//! 过期时刻用 Instant（单调时钟）而不是 SystemTime：后者会被 NTP 校时、
//! 用户改钟影响，可能往回跳。代价是它不能序列化，所以不做持久化。

use std::collections::HashMap;
use std::time::{Duration, Instant};

struct Entry {
    value: String,
    expiry: Option<(Instant, Duration)>, // 设置时刻与有效时长
}

impl Entry {
    fn remaining(&self, now: Instant) -> Option<Duration> {
        self.expiry.map(|(start, ttl)| ttl.saturating_sub(now.saturating_duration_since(start)))
    }
}

pub struct Store {
    map: HashMap<String, Entry>,
}

/// INCR 可能失败的两条路径，对应 RESP 错误回复。
#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    NotAnInteger,
    Overflow,
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::NotAnInteger => write!(f, "值不是整数"),
            StoreError::Overflow => write!(f, "自增溢出"),
        }
    }
}

impl std::error::Error for StoreError {}

impl Default for Store {
    fn default() -> Self {
        Store::new()
    }
}

impl Store {
    pub fn new() -> Self {
        Store {
            map: HashMap::new(),
        }
    }

    /// 所有读写路径共用的入口：键已过期就地删掉并返回 None。
    ///
    /// 只把「是否过期」这个 bool 取出来，get 的不可变借用就在 match 结束时
    /// 释放了，后面的 remove / get_mut 才不会和它打架。
    fn live(&mut self, key: &str) -> Option<&mut Entry> {
        let expired = match self.map.get(key) {
            Some(e) => e.remaining(Instant::now()) == Some(Duration::ZERO),
            None => return None,
        };
        if expired {
            self.map.remove(key);
            return None;
        }
        self.map.get_mut(key)
    }

    pub fn set(&mut self, key: &str, value: &str, ttl: Option<Duration>) {
        let expiry = ttl.map(|duration| (Instant::now(), duration));
        match self.map.get_mut(key) {
            Some(e) => {
                e.value = value.to_string();
                e.expiry = expiry;
            }
            None => {
                self.map.insert(
                    key.to_string(),
                    Entry {
                        value: value.to_string(),
                        expiry,
                    },
                );
            }
        }
    }

    pub fn get(&mut self, key: &str) -> Option<String> {
        self.live(key).map(|e| e.value.clone())
    }

    pub fn exists(&mut self, key: &str) -> bool {
        self.live(key).is_some()
    }

    pub fn del(&mut self, key: &str) -> bool {
        if self.live(key).is_none() {
            return false;
        }
        self.map.remove(key).is_some()
    }

    /// 键不存在按 0 处理；值不是整数报错；溢出不 panic 而是报错。
    /// 就地改值而不是走 set，这样已有的过期时间不会被顺手清掉。
    pub fn incr(&mut self, key: &str) -> Result<i64, StoreError> {
        let next = match self.live(key) {
            Some(e) => e
                .value
                .parse::<i64>()
                .map_err(|_| StoreError::NotAnInteger)?
                .checked_add(1)
                .ok_or(StoreError::Overflow)?,
            None => 1,
        };
        match self.map.get_mut(key) {
            Some(e) => e.value = next.to_string(),
            None => {
                self.map.insert(
                    key.to_string(),
                    Entry {
                        value: next.to_string(),
                        expiry: None,
                    },
                );
            }
        }
        Ok(next)
    }

    /// 排序后返回键名；KEYS 会遍历全表，所以顺手把过期项剔掉，
    /// 否则用户会看到一个 GET 拿不到的键。
    pub fn keys(&mut self) -> Vec<String> {
        let now = Instant::now();
        self.map.retain(|_, e| e.remaining(now) != Some(Duration::ZERO));
        let mut keys: Vec<String> = self.map.keys().cloned().collect();
        keys.sort();
        keys
    }

    /// 只对已存在的键生效。
    pub fn expire(&mut self, key: &str, secs: u64) -> bool {
        if self.live(key).is_none() {
            return false;
        }
        match self.map.get_mut(key) {
            Some(e) => {
                e.expiry = Some((Instant::now(), Duration::from_secs(secs)));
                true
            }
            None => false,
        }
    }

    /// -2 不存在（或已过期），-1 永久，非负为剩余秒数向下取整。
    pub fn ttl(&mut self, key: &str) -> i64 {
        match self.live(key) {
            None => -2,
            Some(e) => match e.remaining(Instant::now()) {
                None => -1,
                Some(left) => i64::try_from(left.as_secs()).unwrap_or(i64::MAX),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_del_roundtrip() {
        let mut store = Store::new();
        assert_eq!(store.get("missing"), None);
        store.set("greeting", "hello", None);
        assert_eq!(store.get("greeting"), Some(String::from("hello")));
        assert!(store.exists("greeting"));
        assert!(store.del("greeting"));
        assert!(!store.del("greeting"));
        assert_eq!(store.get("greeting"), None);
        assert_eq!(store.keys(), Vec::<String>::new());
    }

    #[test]
    fn expired_key_disappears() {
        let mut store = Store::new();
        // 0 秒过期：下一次访问就不可见。
        store.set("tmp", "v", Some(Duration::from_secs(0)));
        assert_eq!(store.get("tmp"), None);
        assert!(!store.exists("tmp"));
        // KEYS 也必须把过期键剔掉。
        store.set("keep", "v", None);
        store.set("tmp2", "v", Some(Duration::from_secs(0)));
        assert_eq!(store.keys(), vec![String::from("keep")]);
    }

    #[test]
    fn incr_rejects_non_integer() {
        let mut store = Store::new();
        assert_eq!(store.incr("hits"), Ok(1));
        assert_eq!(store.incr("hits"), Ok(2));
        store.set("word", "abc", None);
        assert!(matches!(store.incr("word"), Err(StoreError::NotAnInteger)));
    }

    #[test]
    fn ttl_reports_missing_and_persistent() {
        let mut store = Store::new();
        assert_eq!(store.ttl("nope"), -2);
        store.set("forever", "v", None);
        assert_eq!(store.ttl("forever"), -1);
        store.set("soon", "v", Some(Duration::from_secs(30)));
        store.set("long", "v", Some(Duration::MAX));
        assert_eq!(store.ttl("long"), i64::MAX);
        assert!(store.expire("long", u64::MAX));
        assert_eq!(store.get("long").as_deref(), Some("v"));
        let t = store.ttl("soon");
        assert!((0..=30).contains(&t), "TTL 应在 0..=30 之间，实际 {t}");
    }
}
