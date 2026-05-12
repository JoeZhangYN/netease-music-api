//! 单源 HTTP Client 构造（PR-A §1.5）。
//!
//! 收敛 SOT §3.2 漂移：pre-PR-A 解析侧 `main.rs:123` (5/10s) 与下载侧
//! `engine/mod.rs:69` (10/60s) 各自 `Client::builder()`，加新参数要改两处。
//!
//! 解析与下载的 timeout 默认值保留差异（请求-响应 vs 流式大文件性质不同），
//! 通过 `ClientProfile` 显式区分。pool 配置共享。

use std::sync::Once;
use std::time::Duration;

use reqwest::Client;

/// reqwest 0.13 用 `rustls-no-provider` feature 时必须在首个 TLS 握手前装一个
/// `CryptoProvider`；选 ring 与 0.12 默认对齐，避免 aws-lc-rs 的 cmake/clang
/// 依赖（zigbuild Linux x64 cross-compile 会缺这些）。`Once` 保证全进程仅装一次，
/// `install_default` 重复调用返 Err，被吞掉是预期行为。
fn ensure_rustls_provider() {
    static INIT: Once = Once::new();
    INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// 显式区分两条链路的 client 行为。新增 profile 时编译器穷举强制更新 builder。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientProfile {
    /// 解析侧：API 请求-响应小，期望快速返回或失败。
    Parse,
    /// 下载侧：流式大文件，read 期望长持续。
    Download,
}

impl ClientProfile {
    pub const fn connect_timeout(self) -> Duration {
        match self {
            ClientProfile::Parse => Duration::from_secs(5),
            ClientProfile::Download => Duration::from_secs(10),
        }
    }

    pub const fn read_timeout(self) -> Duration {
        match self {
            ClientProfile::Parse => Duration::from_secs(10),
            ClientProfile::Download => Duration::from_secs(60),
        }
    }
}

/// 单源工厂。所有 reqwest::Client 创建必经此函数。
///
/// pool_max_idle_per_host=10 / pool_idle_timeout=90s 在两条链路一致，
/// 直接散在 builder 内部。
pub fn make_client(profile: ClientProfile) -> Client {
    ensure_rustls_provider();
    // builder pattern + 全静态参数（无 env-driven proxy/cert）→ build 恒成功
    #[allow(clippy::expect_used)]
    Client::builder()
        .connect_timeout(profile.connect_timeout())
        .read_timeout(profile.read_timeout())
        .pool_max_idle_per_host(10)
        .pool_idle_timeout(Duration::from_secs(90))
        .build()
        .expect("static client config always valid (no env-driven proxy/cert)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_profile_has_5s_connect_10s_read() {
        assert_eq!(
            ClientProfile::Parse.connect_timeout(),
            Duration::from_secs(5)
        );
        assert_eq!(ClientProfile::Parse.read_timeout(), Duration::from_secs(10));
    }

    #[test]
    fn download_profile_has_10s_connect_60s_read() {
        assert_eq!(
            ClientProfile::Download.connect_timeout(),
            Duration::from_secs(10)
        );
        assert_eq!(
            ClientProfile::Download.read_timeout(),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn profiles_distinct_timeouts() {
        // Attacker view: 误把 Parse/Download 当同一类 client 会破坏不变量
        // (Parse 短超时给用户面快速反馈, Download 长超时给慢 CDN)
        assert_ne!(
            ClientProfile::Parse.connect_timeout(),
            ClientProfile::Download.connect_timeout()
        );
        assert_ne!(
            ClientProfile::Parse.read_timeout(),
            ClientProfile::Download.read_timeout()
        );
    }

    #[test]
    fn make_client_parse_succeeds() {
        let _c = make_client(ClientProfile::Parse);
    }

    #[test]
    fn make_client_download_succeeds() {
        let _c = make_client(ClientProfile::Download);
    }
}
