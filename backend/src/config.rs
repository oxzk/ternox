//! 配置：少量环境变量 + 编译期内置常量。

use std::time::Duration;

// ---------- 内置常量（不通过环境变量覆盖） ----------

/// 认证 Session 有效期。
pub const SESSION_TTL: Duration = Duration::from_secs(86_400);
/// 默认 Shell。
pub const TERMINAL_SHELL: &str = "/bin/bash";
/// 断开后的空闲清理时间。
pub const TERMINAL_IDLE_TIMEOUT: Duration = Duration::from_secs(1800);
/// 终端输出缓冲大小，字节。
pub const TERMINAL_BUFFER_SIZE: usize = 1_048_576;
/// WebSocket 最大消息大小，字节。
pub const TERMINAL_MAX_FRAME_SIZE: usize = 65_536;
/// 心跳超时：超过该时间未收到客户端任何消息则断开。
///
/// 前端以相同节奏（30s ping / 60s 判死）实现，见 frontend/src/services/protocol.ts。
pub const TERMINAL_PONG_TIMEOUT: Duration = Duration::from_secs(60);
/// 日志级别。
pub const LOG_LEVEL: &str = "info";

/// 登录限流：单个 IP 在窗口期内允许的最大失败次数。
pub const LOGIN_MAX_FAILURES: u32 = 8;
/// 登录限流窗口。
pub const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(60);

/// 终端尺寸上限。
pub const MAX_COLS: u16 = 500;
pub const MAX_ROWS: u16 = 300;

// ---------- 环境变量 ----------

/// 禁止使用的默认弱密码。
const WEAK_PASSWORDS: &[&str] = &[
    "password",
    "password1",
    "1234",
    "12345",
    "123456",
    "12345678",
    "123456789",
    "admin",
    "admin123",
    "root",
    "qwerty",
    "letmein",
    "000000",
    "111111",
    "change-this-password",
    "ternox",
];

#[derive(Clone)]
pub struct Config {
    /// HTTP 监听地址。
    pub host: String,
    /// HTTP 端口。
    pub port: u16,
    /// 登录用户名。
    pub auth_username: String,
    /// 登录密码。
    pub auth_password: String,
    /// Cookie Secure（默认 true；仅 HTTP 非 localhost 访问时需设为 false）。
    pub session_secure: bool,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // 密码不能进入任何 Debug 输出
        f.debug_struct("Config")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("auth_username", &self.auth_username)
            .field("auth_password", &"[REDACTED]")
            .field("session_secure", &self.session_secure)
            .finish()
    }
}

impl Config {
    /// 从环境变量加载配置；认证信息缺失或为弱密码时返回错误（拒绝启动）。
    pub fn from_env() -> Result<Self, String> {
        let auth_username = env_str("AUTH_USERNAME").ok_or("AUTH_USERNAME 未配置，拒绝启动")?;
        let auth_password = env_str("AUTH_PASSWORD").ok_or("AUTH_PASSWORD 未配置，拒绝启动")?;
        validate_credentials(&auth_username, &auth_password)?;

        let session_secure = match env_str("SESSION_SECURE") {
            None => true,
            Some(value) => match value.as_str() {
                "true" | "1" | "yes" => true,
                "false" | "0" | "no" => false,
                other => return Err(format!("SESSION_SECURE 的值 `{other}` 无效，应为 true/false")),
            },
        };

        Ok(Self {
            // 默认只监听本机：Docker 部署由镜像 ENV 显式设为 0.0.0.0
            host: env_str("HOST").unwrap_or_else(|| "127.0.0.1".into()),
            port: env_parse::<u16>("PORT", 7681)?,
            auth_username,
            auth_password,
            session_secure,
        })
    }
}

/// 校验账号密码：非空、禁止弱密码、禁止用户名与密码相同。
fn validate_credentials(username: &str, password: &str) -> Result<(), String> {
    if username.is_empty() {
        return Err("用户名不能为空".into());
    }
    if password.is_empty() {
        return Err("密码不能为空".into());
    }
    if username.eq_ignore_ascii_case(password) {
        return Err("密码不能与用户名相同".into());
    }
    if WEAK_PASSWORDS.iter().any(|weak| weak.eq_ignore_ascii_case(password)) {
        return Err("密码为默认弱密码，请通过 AUTH_PASSWORD 设置更强的密码".into());
    }
    Ok(())
}

fn env_str(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> Result<T, String> {
    match env_str(key) {
        None => Ok(default),
        Some(value) => value
            .parse()
            .map_err(|_| format!("环境变量 {key} 的值 `{value}` 无法解析")),
    }
}
