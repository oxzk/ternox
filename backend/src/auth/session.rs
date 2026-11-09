//! 内存认证 Session 管理。
//!
//! 服务端只存储 Token 的 SHA-256 摘要；应用重启后所有 Session 失效。

use std::collections::HashMap;
use std::time::{Duration, Instant};

use getrandom::getrandom;
use sha2::{Digest, Sha256};

/// 认证 Cookie 名称。
pub const SESSION_COOKIE: &str = "web_terminal_session";

/// Token 长度：256bit 随机数的十六进制表示。
const TOKEN_LEN: usize = 64;

#[derive(Debug)]
pub struct SessionManager {
    /// key: Token 摘要，value: 过期时间。
    sessions: HashMap<[u8; 32], Instant>,
    ttl: Duration,
}

impl SessionManager {
    pub fn new(ttl: Duration) -> Self {
        Self {
            sessions: HashMap::new(),
            ttl,
        }
    }

    /// 创建新 Session：密码学安全随机 256bit Token，返回明文 Token 用于 Cookie。
    pub fn create(&mut self) -> String {
        let mut bytes = [0u8; 32];
        getrandom(&mut bytes).expect("OS 随机数不可用");
        let token = hex_encode(&bytes);
        self.sessions.insert(sha256(token.as_bytes()), self.expiry());
        token
    }

    /// 登录成功时轮换 Token：销毁旧 Session 并创建新 Session。
    pub fn rotate(&mut self, previous_token: Option<&str>) -> String {
        if let Some(previous) = previous_token {
            self.destroy(previous);
        }
        self.create()
    }

    pub fn validate(&mut self, token: &str) -> bool {
        self.purge_expired();
        match digest_of(token) {
            Some(digest) => matches!(self.sessions.get(&digest), Some(expiry) if *expiry > Instant::now()),
            None => false,
        }
    }

    pub fn destroy(&mut self, token: &str) -> bool {
        match digest_of(token) {
            Some(digest) => self.sessions.remove(&digest).is_some(),
            None => false,
        }
    }

    /// 清理过期 Session（由后台任务定期调用，验证时也会顺带清理）。
    pub fn purge_expired(&mut self) {
        let now = Instant::now();
        self.sessions.retain(|_, expiry| *expiry > now);
    }

    fn expiry(&self) -> Instant {
        Instant::now() + self.ttl
    }
}

fn digest_of(token: &str) -> Option<[u8; 32]> {
    if token.len() != TOKEN_LEN || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(sha256(token.as_bytes()))
}

pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(data);
    digest
        .as_slice()
        .try_into()
        .expect("sha256 digest is 32 bytes")
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
