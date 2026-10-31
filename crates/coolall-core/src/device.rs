//! 设备身份：`X-App-Device` 编码与 `User-Agent` 构造。
//!
//! 协议见 `docs/PROTOCOL.md` §4 与 §5。
//!
//! 这两者是 WAF 判定的关键：
//! - 设备串必须是 9 段 Base64 反转，随机 hex 会被拒
//! - UA 必须包含 `(#Build; ...)` 段和 `+CoolMarket/...` 尾巴
//!
//! 实测：残缺 UA 直接 403，补全后 200。见 PROTOCOL.md §13。

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};

/// 协议常量。集中管理，便于酷安改版时快速更新。
pub mod constants {
    pub const APP_ID: &str = "com.coolapk.market";
    pub const CHANNEL: &str = "coolapk";
    pub const MODE: &str = "universal";
    pub const LOCALE: &str = "zh-CN";
    pub const REQUEST_WITH: &str = "XMLHttpRequest";

    /// 必须是 16.x。13.4.1 会被 WAF 直接 403，见 PROTOCOL.md §7
    pub const VERSION_NAME: &str = "16.7.0";
    pub const VERSION_CODE: &str = "2610291";
    pub const API_VERSION: &str = "16";

    /// TokenV3 的 appCode，等于 VERSION_CODE 的整数形式
    pub const APP_CODE: i64 = 2_610_291;

    pub const BASE_URL: &str = "https://api.coolapk.com";
    pub const BASE_URL_2: &str = "https://api2.coolapk.com";
    pub const PUBLIC_URL: &str = "https://www.coolapk.com";
}

/// 机型画像。桌面端没有真实安卓机型，使用一组稳定的模拟值。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeviceProfile {
    pub manufacturer: String,
    pub brand: String,
    pub model: String,
    pub build: String,
    pub android_version: String,
    pub sdk_int: String,
}

impl Default for DeviceProfile {
    fn default() -> Self {
        Self {
            manufacturer: "Xiaomi".into(),
            brand: "Xiaomi".into(),
            model: "2211133C".into(),
            build: "TKQ1.221114.001".into(),
            android_version: "13".into(),
            sdk_int: "33".into(),
        }
    }
}

/// 设备身份。持有 `szlmId` 与机型，负责产出两个关键请求头。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceIdentity {
    pub szlm_id: String,
    pub profile: DeviceProfile,
}

impl DeviceIdentity {
    pub fn new(szlm_id: impl Into<String>, profile: DeviceProfile) -> Self {
        Self {
            szlm_id: szlm_id.into(),
            profile,
        }
    }

    /// 生成随机的合法 szlmId（32 位小写 hex）。
    ///
    /// 桌面端无法运行数盟 SDK，首选用随机值；用户也可手动导入真实 ID。
    pub fn random_szlm_id() -> String {
        use rand::Rng;
        let mut rng = rand::thread_rng();
        (0..32)
            .map(|_| format!("{:x}", rng.gen_range(0..16u8)))
            .collect()
    }

    /// 校验 szlmId 是否合法，对齐参考实现的 `OfficialDeviceIdentity.valid`。
    pub fn valid_szlm_id(id: &str) -> bool {
        if id.eq_ignore_ascii_case("null") || id.eq_ignore_ascii_case("undefined") {
            return false;
        }
        let len = id.len();
        (8..=4096).contains(&len)
            && id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '/' | '.' | '=' | '-'))
    }

    /// 从官方 App「复制 OAID」得到的文本中提取 ShuzlmID。
    ///
    /// 匹配规则：`^\s*ShuzlmID\s*[:：]\s*([^\r\n]*)$`（多行、忽略大小写），必须唯一命中。
    pub fn parse_shuzlm_id(text: &str) -> Result<String, String> {
        if text.len() > 32768 {
            return Err("复制内容过长，请重新从官方酷安复制".into());
        }
        let cleaned = text.replace('\u{feff}', "");
        let mut found: Vec<String> = Vec::new();
        for line in cleaned.lines() {
            let trimmed = line.trim_start();
            if !trimmed.to_lowercase().starts_with("shuzlmid") {
                continue;
            }
            let after = trimmed[8..].trim_start();
            let after = after.strip_prefix([':', '：']).unwrap_or(after);
            found.push(after.trim().to_string());
        }
        if found.len() != 1 {
            return Err(
                "未识别到唯一的 ShuzlmID，请在官方同一页面点击“复制 OAID”5 次后粘贴完整内容"
                    .into(),
            );
        }
        let id = found.remove(0);
        if !Self::valid_szlm_id(&id) {
            return Err("数盟 ID 为空或格式不正确，请稍等后重新复制".into());
        }
        Ok(id)
    }

    /// 设备串明文：9 段分号分隔。
    fn device_plaintext(&self) -> String {
        let p = &self.profile;
        format!(
            "{}; ; ; ; {}; {}; {}; {}; null",
            self.szlm_id, p.manufacturer, p.brand, p.model, p.build
        )
    }

    /// `X-App-Device` 的值：Base64 无填充 → 整体反转 → 去除换行与 `=`。
    pub fn device_header(&self) -> String {
        let b64 = STANDARD.encode(self.device_plaintext().as_bytes());
        let trimmed = b64.trim_end_matches('=');
        trimmed
            .chars()
            .rev()
            .collect::<String>()
            .replace(['\r', '\n', '='], "")
    }

    /// 还原设备串，用于自检。
    pub fn decode_device_header(header: &str) -> Option<String> {
        let reversed: String = header.chars().rev().collect();
        let rem = reversed.len() % 4;
        let padded = if rem == 0 {
            reversed
        } else {
            format!("{}{}", reversed, "=".repeat(4 - rem))
        };
        STANDARD
            .decode(padded)
            .ok()
            .and_then(|b| String::from_utf8(b).ok())
    }

    /// `User-Agent`。三段式，缺一不可，见 PROTOCOL.md §5。
    pub fn user_agent(&self) -> String {
        let p = &self.profile;
        format!(
            "Dalvik/2.1.0 (Linux; U; Android {rel}; {model} {build}) (#Build; {brand}; {model}; {build}; {rel}) +CoolMarket/{vn}-{vc}-{mode}",
            rel = p.android_version,
            model = p.model,
            build = p.build,
            brand = p.brand,
            vn = constants::VERSION_NAME,
            vc = constants::VERSION_CODE,
            mode = constants::MODE,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> DeviceIdentity {
        DeviceIdentity::new(
            "63a31cc4b95ae992233b215af7cb1f0d",
            DeviceProfile::default(),
        )
    }

    #[test]
    fn device_header_roundtrip() {
        let id = identity();
        let header = id.device_header();
        let decoded = DeviceIdentity::decode_device_header(&header).expect("应可解码");
        assert_eq!(decoded, id.device_plaintext());
        assert_eq!(decoded.split(';').count(), 9, "必须是 9 段");
    }

    #[test]
    fn device_header_has_no_padding_or_newline() {
        let header = identity().device_header();
        assert!(!header.contains('='));
        assert!(!header.contains('\n'));
        assert!(!header.contains('\r'));
    }

    #[test]
    fn user_agent_shape() {
        let ua = identity().user_agent();
        assert!(ua.starts_with("Dalvik/2.1.0 (Linux; U; Android 13;"));
        assert!(ua.contains("(#Build; Xiaomi; 2211133C; TKQ1.221114.001; 13)"));
        assert!(ua.ends_with("+CoolMarket/16.7.0-2610291-universal"));
    }

    #[test]
    fn random_id_is_valid() {
        for _ in 0..20 {
            let id = DeviceIdentity::random_szlm_id();
            assert_eq!(id.len(), 32);
            assert!(DeviceIdentity::valid_szlm_id(&id));
        }
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(!DeviceIdentity::valid_szlm_id("null"));
        assert!(!DeviceIdentity::valid_szlm_id("undefined"));
        assert!(!DeviceIdentity::valid_szlm_id("short"));
        assert!(!DeviceIdentity::valid_szlm_id("has space here"));
    }

    #[test]
    fn parses_shuzlm_id_from_official_text() {
        let text = "OAID: 1234\nShuzlmID: abcdef12345678\nfoo";
        assert_eq!(
            DeviceIdentity::parse_shuzlm_id(text).unwrap(),
            "abcdef12345678"
        );
    }

    #[test]
    fn parses_fullwidth_colon() {
        let text = "ShuzlmID：abcdef12345678";
        assert_eq!(
            DeviceIdentity::parse_shuzlm_id(text).unwrap(),
            "abcdef12345678"
        );
    }

    #[test]
    fn rejects_ambiguous_or_missing() {
        assert!(DeviceIdentity::parse_shuzlm_id("nothing here").is_err());
        assert!(DeviceIdentity::parse_shuzlm_id("ShuzlmID: aaaa1111\nShuzlmID: bbbb2222").is_err());
    }
}
