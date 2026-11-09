//! 认证：环境变量账号密码校验、Session 管理与登录限流。

pub mod session;

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::http::{header, HeaderMap};
use subtle::ConstantTimeEq;

use crate::config::{LOGIN_FAILURE_WINDOW, LOGIN_MAX_FAILURES};
use session::{sha256, SessionManager, SESSION_COOKIE};

/// 过期认证会话的后台清理周期。
pub const SESSION_PURGE_INTERVAL: Duration = Duration::from_secs(60);

/// 从请求头解析会话 Cookie（明文 Token）。
pub fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::COOKIE)?.to_str().ok()?;
    for pair in raw.split(';') {
        let pair = pair.trim();
        if let Some((name, value)) = pair.split_once('=') {
            if name.trim() == SESSION_COOKIE {
                return Some(value.trim().to_string());
            }
        }
    }
    None
}

/// 构造登录 Set-Cookie 值。
pub fn session_cookie(token: &str, max_age: u64, secure: bool) -> String {
    let mut cookie = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}"
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

/// 构造登出清除 Cookie 的 Set-Cookie 值（空 Token + Max-Age=0）。
pub fn clear_session_cookie(secure: bool) -> String {
    session_cookie("", 0, secure)
}

/// 计算登录限流键。
///
/// 对端是环回/内网地址（典型为反向代理）时，取 X-Forwarded-For 最右侧条目
/// （由可信代理写入的真实客户端地址；左侧条目可被客户端伪造）；
/// 直连时使用对端 IP。IPv6 按 /64 前缀聚合，防止在单一前缀内轮换地址绕过限流。
pub fn rate_limit_key(peer: IpAddr, headers: &HeaderMap) -> String {
    if is_trusted_proxy_ip(peer) {
        // 拼接全部同名头后再取最右：代理可能把追加项写成第二条 X-Forwarded-For 行
        let forwarded = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .collect::<Vec<_>>()
            .join(",");
        if let Some(client) = forwarded
            .rsplit(',')
            .next()
            .and_then(|v| v.trim().parse::<IpAddr>().ok())
        {
            return ip_key(client);
        }
    }
    ip_key(peer)
}

/// 环回 / 内网（RFC1918 / ULA / 链路本地）地址视为可信代理网段。
fn is_trusted_proxy_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || {
                    let seg = v6.segments();
                    // fc00::/7 (ULA) 或 fe80::/10 (链路本地)
                    (seg[0] & 0xfe00) == 0xfc00 || (seg[0] & 0xffc0) == 0xfe80
                }
        }
    }
}

fn ip_key(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => format!("v4:{v4}"),
        IpAddr::V6(v6) => {
            let octets = v6.octets();
            let mut bytes = [0u8; 16];
            bytes[..8].copy_from_slice(&octets[..8]);
            format!("v6:{:x}", u128::from_be_bytes(bytes))
        }
    }
}

/// 登录结果。
pub enum LoginOutcome {
    /// 登录成功，返回轮换后的新 Token。
    Ok(String),
    /// 凭据错误。
    InvalidCredentials,
    /// 触发限流，返回建议的等待秒数。
    RateLimited(u64),
}

/// 认证管理器：凭据校验 + 内存 Session + 登录限流。
pub struct Auth {
    username_digest: [u8; 32],
    password_digest: [u8; 32],
    sessions: Mutex<SessionManager>,
    limiter: LoginRateLimiter,
}

impl Auth {
    pub fn new(username: &str, password: &str, session_ttl: Duration) -> Self {
        Self {
            username_digest: sha256(username.as_bytes()),
            password_digest: sha256(password.as_bytes()),
            sessions: Mutex::new(SessionManager::new(session_ttl)),
            limiter: LoginRateLimiter::new(LOGIN_MAX_FAILURES, LOGIN_FAILURE_WINDOW),
        }
    }

    /// 登录：限流检查 → 恒定时间凭据比较 → Token 轮换。
    pub fn login(
        &self,
        previous_token: Option<&str>,
        username: &str,
        password: &str,
        limit_key: String,
    ) -> LoginOutcome {
        if let Some(retry_after) = self.limiter.blocked_for(&limit_key) {
            return LoginOutcome::RateLimited(retry_after);
        }
        if !self.credentials_match(username, password) {
            self.limiter.record_failure(limit_key);
            return LoginOutcome::InvalidCredentials;
        }
        self.limiter.record_success(limit_key);
        let token = self.sessions.lock().unwrap().rotate(previous_token);
        LoginOutcome::Ok(token)
    }

    pub fn validate_session(&self, token: &str) -> bool {
        self.sessions.lock().unwrap().validate(token)
    }

    pub fn destroy_session(&self, token: &str) -> bool {
        self.sessions.lock().unwrap().destroy(token)
    }

    pub fn purge_sessions(&self) {
        self.sessions.lock().unwrap().purge_expired();
    }

    /// 恒定时间比较：对 SHA-256 摘要比较，长度固定，无时序差异。
    fn credentials_match(&self, username: &str, password: &str) -> bool {
        let username_digest = sha256(username.as_bytes());
        let password_digest = sha256(password.as_bytes());
        bool::from(username_digest.ct_eq(&self.username_digest))
            & bool::from(password_digest.ct_eq(&self.password_digest))
    }
}

/// 限流表跟踪键数上限（硬上限：达到后拒绝记录新键，内存有界）。
const MAX_TRACKED_KEYS: usize = 4096;

/// 按限流键的登录失败限流（固定窗口）。
struct LoginRateLimiter {
    max_failures: u32,
    window: Duration,
    entries: Mutex<HashMap<String, RateEntry>>,
}

#[derive(Default)]
struct RateEntry {
    failures: u32,
    window_start: Option<Instant>,
}

impl LoginRateLimiter {
    fn new(max_failures: u32, window: Duration) -> Self {
        Self {
            max_failures,
            window,
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// 若该键已被限流，返回剩余等待秒数。
    fn blocked_for(&self, key: &str) -> Option<u64> {
        let entries = self.entries.lock().unwrap();
        let entry = entries.get(key)?;
        let start = entry.window_start?;
        let elapsed = start.elapsed();
        if elapsed < self.window && entry.failures >= self.max_failures {
            Some((self.window - elapsed).as_secs().max(1))
        } else {
            None
        }
    }

    fn record_failure(&self, key: String) {
        let mut entries = self.entries.lock().unwrap();
        if entries.len() >= MAX_TRACKED_KEYS && !entries.contains_key(&key) {
            // 硬上限：窗口内海量唯一来源时放弃跟踪新键（限流对它们失效，但内存有界）
            return;
        }
        let entry = entries.entry(key).or_default();
        match entry.window_start {
            Some(start) if start.elapsed() < self.window => entry.failures += 1,
            _ => {
                entry.failures = 1;
                entry.window_start = Some(Instant::now());
            }
        }
    }

    fn record_success(&self, key: String) {
        self.entries.lock().unwrap().remove(&key);
    }
}
