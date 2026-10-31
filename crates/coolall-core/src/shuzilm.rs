//! 数盟 ID 获取流程（数盟 = Shuzilm，酷安用来识别真实设备的服务）。
//!
//! 移植自参考项目 `DuClient.kt` / `DuCodec.kt`。
//!
//! # 为什么需要这个
//!
//! 酷安的 `X-App-Device` 第一段是数盟签发的设备 ID。
//! 没有真实 ID 时服务端会返回 `status: 1004`。
//!
//! # 流程
//!
//! ```text
//! 采集本机特征（Android ID / Widevine / 开机时间）
//!   → 按 p 参数做字符混淆
//!   → JSON 序列化
//!   → deflate 压缩
//!   → XOR 加密
//!   → POST 到数盟
//!   → 响应 XOR 解密 → 取 n_a 或 cdd 字段
//! ```
//!
//! # 桌面端的限制
//!
//! 桌面没有真实的 Android ID 和 Widevine。这里只能用占位值，
//! 服务端可能因此拒绝签发。若失败，需要用户从手机导入真实 ID。

use md5::{Digest, Md5};
use serde_json::json;

/// 数盟 API Key（来自参考项目，是公开的客户端密钥）。
const API_KEY: &str = "11e7b222083a4b732b4b14811f6fc05995a01415eb";
const PKG: &str = "com.coolapk.market";

/// 数盟服务端点。
const HOSTS: &[&str] = &[
    "https://auni.telecome.cn",
    "https://global-auni.checktrustworthiness.com",
];

/// XOR 密钥表。
const KEY: &[u8] = b"xp7j@&!v5]2k#+3z{n9$q^r%[8f1*d4e:c>0l?m/6)yb-=w}";

/// 重复 XOR（加解密同一函数）。
fn xor(data: &[u8]) -> Vec<u8> {
    data.iter()
        .enumerate()
        .map(|(i, b)| b ^ KEY[i % KEY.len()])
        .collect()
}

fn md5_upper(s: &str) -> String {
    let mut h = Md5::new();
    h.update(s.as_bytes());
    h.finalize()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect()
}

fn sha1_upper(s: &str) -> String {
    use sha1::{Digest as _, Sha1};
    let mut h = Sha1::new();
    h.update(s.as_bytes());
    h.finalize().iter().map(|b| format!("{b:02X}")).collect()
}

/// 编码参数：时间字符串后 3 位去掉 '.' 后转整数。
fn param_of(time: &str) -> u32 {
    let last3: String = time
        .chars()
        .rev()
        .take(3)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    last3.replace('.', "").parse().unwrap_or(0)
}

/// 四种洗牌方式，按 (p / 100) % 4 选。
fn shuffle(s: &str, p: u32) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let m = n / 2;
    let mut out = String::with_capacity(n);
    match (p / 100) % 4 {
        0 => {
            if n % 2 == 1 {
                out.push(chars[m]);
            }
            for i in 0..m {
                out.push(chars[n - 1 - i]);
                out.push(chars[m - 1 - i]);
            }
        }
        1 => {
            for i in 0..m {
                out.push(chars[n - 1 - i]);
                out.push(chars[i]);
            }
            if n % 2 == 1 {
                out.push(chars[m]);
            }
        }
        2 => {
            let mut i = n as i64 - 1;
            while i >= 0 {
                out.push(chars[i as usize]);
                i -= 2;
            }
            let mut i = n as i64 - 2;
            while i >= 0 {
                out.push(chars[i as usize]);
                i -= 2;
            }
        }
        _ => {
            for i in (0..n).step_by(2) {
                out.push(chars[i]);
            }
            for i in (1..n).step_by(2) {
                out.push(chars[i]);
            }
        }
    }
    out
}

/// 字段混淆：每个可打印字符在 95 区间内旋转 k，再洗牌。
fn enc(s: &str, p: u32) -> String {
    let k = p % 100 % 57 + 1;
    let rotated: String = s
        .chars()
        .map(|c| {
            let code = c as u32;
            if (0x20..=0x7e).contains(&code) {
                char::from_u32((code - 0x20 + k) % 95 + 0x20).unwrap_or(c)
            } else {
                c
            }
        })
        .collect();
    shuffle(&rotated, p)
}

/// 掉字符：n > 9 时去掉 n-1-p%10 位，否则去最后一位。
fn drop_char(s: &str, p: u32) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= 1 {
        return s.to_string();
    }
    let i = if chars.len() > 9 {
        chars.len() - 1 - (p % 10) as usize
    } else {
        chars.len() - 1
    };
    chars
        .iter()
        .enumerate()
        .filter(|(idx, _)| *idx != i)
        .map(|(_, c)| *c)
        .collect()
}

/// 校验和：MD5(sum(字符码) + p%100/10) 前 7 位。
fn checksum(s: &str, p: u32) -> String {
    if s.chars().count() <= 1 {
        return "CFCD208".to_string();
    }
    let sum: u32 = s.chars().map(|c| c as u32).sum();
    md5_upper(&(sum + p % 100 / 10).to_string())[..7].to_string()
}

/// 伪 MAC：SHA1 大写后按 [1:7]:[13:19]:[7:13] 切片。
fn fake_mac(sha1_upper: &str) -> String {
    if sha1_upper.len() < 19 {
        return "00:00:00:00:00:00".to_string();
    }
    format!(
        "{}:{}:{}",
        &sha1_upper[1..7],
        &sha1_upper[13..19],
        &sha1_upper[7..13]
    )
}

/// deflate 压缩（zlib 格式）。
fn deflate(data: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut e = ZlibEncoder::new(Vec::new(), Compression::default());
    let _ = e.write_all(data);
    e.finish().unwrap_or_default()
}

/// 构造上报 payload。
///
/// 桌面端无法拿到真实 Android ID / Widevine，
/// 这里用稳定的占位值，保证同一台机器多次调用一致。
pub fn build_payload(android_id: &str, previous_id: Option<&str>) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let nsec = (now_ms % 1000) * 1_000_000;
    let time = format!("{}.{}", now_ms / 1000, nsec.max(1));
    let p = param_of(&time);

    let sha1 = sha1_upper(android_id);
    let fake = fake_mac(&sha1);

    let mut obj = json!({
        "foO": enc(&drop_char(android_id, p), p),
        "sfOo": checksum(android_id, p),
        "P1J": fake,
        "AYk": fake,
        "dLH": enc("00000000-0000-4000-8000-000000000000", p),
        "75c": API_KEY,
        "DUv": PKG,
        "3mS": "v9.0.1",
        "AAA": "v1.0",
        "BBB": "v1.0",
        "UOv": enc("0.0", p),
        "2cO": time,
        "R37": "Xiaomi",
        "R3d": "2211133C",
        "R41": "13",
        "SAN": 33,
    });

    if let Some(prev) = previous_id {
        if let Some(m) = obj.as_object_mut() {
            m.insert("ubF".into(), json!(prev));
        }
    }

    obj.to_string()
}

/// 请求数盟，获取设备 ID。
pub async fn fetch_device_id(
    client: &reqwest::Client,
    android_id: &str,
    previous_id: Option<&str>,
) -> Result<String, String> {
    let payload = build_payload(android_id, previous_id);
    let body = xor(&deflate(payload.as_bytes()));

    let rnd = uuid_like();
    let path = format!(
        "/a/mdna/report?v=8.7&t=m&p={}&r={}&n=0&l=2&h=0",
        md5_upper(&format!("{PKG}.37be8a1ee106979a")),
        rnd
    );

    let mut last_err = String::from("无可用端点");

    for host in HOSTS {
        let url = format!("{host}{path}");
        match client
            .post(&url)
            .header("Content-Length", body.len().to_string())
            .body(body.clone())
            .send()
            .await
        {
            Ok(resp) => {
                if resp.status() != 200 {
                    last_err = format!("{host} 返回 {}", resp.status());
                    continue;
                }
                let raw = resp.bytes().await.map_err(|e| e.to_string())?;
                let decoded = xor(&raw);
                let text = String::from_utf8_lossy(&decoded);
                let v: serde_json::Value =
                    serde_json::from_str(&text).map_err(|e| format!("解析失败: {e}"))?;

                if v.get("err").and_then(|x| x.as_str()) != Some("0") {
                    last_err = format!("{host} 返回 err={}", v.get("err").unwrap_or(&json!(null)));
                    continue;
                }
                for key in ["n_a", "cdd"] {
                    if let Some(id) = v.get(key).and_then(|x| x.as_str()) {
                        if !id.is_empty() && !id.chars().all(|c| c == '0') {
                            return Ok(id.to_string());
                        }
                    }
                }
                last_err = format!("{host} 未返回有效 id");
            }
            Err(e) => {
                last_err = format!("{host} 请求失败: {e}");
            }
        }
    }

    Err(last_err)
}

fn uuid_like() -> String {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    (0..32)
        .map(|_| format!("{:x}", rng.gen_range(0..16u8)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codec_matches_reference_shape() {
        // 参考项目的编码是纯函数，这里校验关键性质而非具体值
        let p = 123;
        let s = "abcdefghij";
        assert_eq!(enc(s, p).len(), s.len(), "编码不改变长度");
        assert_eq!(drop_char(s, p).len(), s.len() - 1);
        assert_ne!(enc(s, p), s, "编码应改变内容");
    }

    #[test]
    fn checksum_is_seven_chars() {
        assert_eq!(checksum("abcdef123456", 42).len(), 7);
        assert_eq!(checksum("x", 42), "CFCD208");
    }

    #[test]
    fn xor_is_reversible() {
        let data = b"hello world";
        assert_eq!(xor(&xor(data)), data.to_vec());
    }

    #[test]
    fn payload_is_valid_json() {
        let p = build_payload("abcdef1234567890", None);
        let v: serde_json::Value = serde_json::from_str(&p).expect("应为合法 JSON");
        assert!(v.get("75c").is_some(), "应含 API key");
        assert!(v.get("DUv").is_some(), "应含包名");
        assert!(v.get("2cO").is_some(), "应含时间");
    }

    #[test]
    fn payload_includes_previous_id() {
        let p = build_payload("abcdef1234567890", Some("prev-id-123"));
        assert!(p.contains("prev-id-123"));
    }
}
