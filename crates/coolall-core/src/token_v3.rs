//! 酷安 TokenV3 请求签名。
//!
//! 算法结构见 `docs/PROTOCOL.md` §3。本实现独立于上游，仅按协议契约重写，
//! 并与 `tools/verify_token_v3.py` 的输出逐字节交叉校验。
//!
//! 三个最容易出错的地方：
//! 1. `LOOKUP` 切片后还要再做一次 base64 解码，才得到 `segment`
//! 2. 盐末位有概率落到 `+`，必须跳过并换下一个时间戳重试
//! 3. 传入的 `device` 是编码后的设备串，不是 szlmId

use base64::{engine::general_purpose::STANDARD, Engine as _};
use md5::{Digest, Md5};

/// 查找表共 930 字节。以资源文件内嵌，避免手工抄写导致索引错位。
const LOOKUP_B64: &str = include_str!("../assets/token_lookup.b64");

/// 标准 Base64 字母表（用于盐末位前推）
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// bcrypt 专用字母表，与标准 Base64 不同：以 `.` `/` 开头
const BCRYPT_ALPHABET: &[u8; 64] =
    b"./ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";

const APP_ID: &str = "com.coolapk.market";

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("查找表损坏，无法生成签名")]
    LookupCorrupt,
    #[error("16 次尝试均失败，请检查系统时间")]
    Exhausted,
}

fn md5_hex(value: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(value.as_bytes());
    hex::encode(hasher.finalize())
}

fn b64_nopad(value: &str) -> String {
    STANDARD
        .encode(value.as_bytes())
        .trim_end_matches('=')
        .to_string()
}

fn lookup() -> Result<&'static [u8], TokenError> {
    use std::sync::OnceLock;
    static LOOKUP: OnceLock<Vec<u8>> = OnceLock::new();
    let bytes = LOOKUP.get_or_init(|| {
        let compact: String = LOOKUP_B64.split_whitespace().collect();
        STANDARD.decode(compact).unwrap_or_default()
    });
    if bytes.len() != 930 {
        return Err(TokenError::LookupCorrupt);
    }
    Ok(bytes.as_slice())
}

/// 取出第 `remainder` 段（0..100）的 segment 字符串。
///
/// 切片作用于字节数组，切片结果本身还要再做一次 base64 解码。
fn segment_for(remainder: u32) -> Result<String, TokenError> {
    let table = lookup()?;
    let start = (remainder as usize) * 4 + 0x80;
    let end = start + 0x80;
    if end > table.len() {
        return Err(TokenError::LookupCorrupt);
    }
    let decoded = STANDARD
        .decode(&table[start..end])
        .map_err(|_| TokenError::LookupCorrupt)?;
    String::from_utf8(decoded).map_err(|_| TokenError::LookupCorrupt)
}

/// 把 22 位 bcrypt 盐字符串还原成 16 字节原始盐。
///
/// bcrypt 的盐字符串使用专属字母表 `. / A-Z a-z 0-9`，与标准 Base64 不同，
/// 不能直接用 STANDARD 解码。
///
/// 关键规则：22 个字符 × 6 位 = 132 位，而 16 字节只要 128 位。
/// **末尾多出的 4 位必须为 0**，否则不是合法盐。
/// 很多实现靠捕获 bcrypt 的 "Invalid salt" 异常来跳过，但那是结果而非规则；
/// 这里显式检查，避免依赖异常控制流。
fn decode_bcrypt_salt(salt: &str) -> Option<[u8; 16]> {
    let mut bits: u64 = 0;
    let mut nbits: u32 = 0;
    let mut out = [0u8; 16];
    let mut written = 0usize;
    for ch in salt.bytes() {
        let v = BCRYPT_ALPHABET.iter().position(|&c| c == ch)? as u64;
        bits = (bits << 6) | v;
        nbits += 6;
        if nbits >= 8 {
            nbits -= 8;
            if written >= 16 {
                return None;
            }
            out[written] = ((bits >> nbits) & 0xFF) as u8;
            written += 1;
            bits &= (1u64 << nbits) - 1;
        }
    }
    // 剩余 nbits 位（应为 4）必须为 0，这是 bcrypt 盐的硬性要求
    if written == 16 && bits == 0 {
        Some(out)
    } else {
        None
    }
}

/// 生成 `X-App-Token`。
///
/// * `device` —— `X-App-Device` 的编码值（不是 szlmId）
/// * `app_code` —— 等于 `X-App-Code`，即 VERSION_CODE 的整数形式
/// * `timestamp` —— Unix 秒
pub fn create(device: &str, app_code: i64, timestamp: i64) -> Result<String, TokenError> {
    let device_md5 = md5_hex(device);

    for offset in 0..16i64 {
        let t = timestamp + offset;
        let remainder = ((t + app_code).rem_euclid(100)) as u32;
        let segment = segment_for(remainder)?;

        let plain = format!("{APP_ID}&{segment}&{device_md5}&{t}&{app_code}");
        let password = md5_hex(&b64_nopad(&plain));

        let source_full = b64_nopad(&format!("{t:x}/{}", md5_hex(&plain)));
        if source_full.len() < 22 {
            continue;
        }
        let source = &source_full[..22];

        // 末位前推 5 位得到盐的最后一位
        let last = source.as_bytes()[21];
        let Some(pos) = ALPHABET.iter().position(|&c| c == last) else {
            continue;
        };
        let mut salt = String::with_capacity(22);
        salt.push_str(&source[..21]);
        salt.push(ALPHABET[(pos + 64 - 5) % 64] as char);

        // '+' 在 ALPHABET 内但不是合法 bcrypt 盐字符，必须跳过重试
        if salt
            .bytes()
            .any(|c| !(c.is_ascii_alphanumeric() || c == b'.' || c == b'/'))
        {
            continue;
        }

        let Some(raw_salt) = decode_bcrypt_salt(&salt) else {
            continue;
        };

        let Ok(hashed) = bcrypt::hash_with_salt(password.as_bytes(), 4, raw_salt) else {
            continue;
        };

        // 输出前缀从 $2a 改为 $2y
        let encoded = hashed.format_for_version(bcrypt::Version::TwoY);
        return Ok(format!("v3{}", STANDARD.encode(encoded.as_bytes())));
    }

    Err(TokenError::Exhausted)
}

#[cfg(test)]
mod tests {
    use super::*;

    const DEVICE: &str = "00000000000000000000000000000000";
    const APP_CODE: i64 = 2610291;
    const FIXED_TS: i64 = 1735689600;

    /// 由 Python 参考实现 `tools/verify_token_v3.py` 算出，两边逐字节一致。
    const EXPECTED: &str =
        "v3JDJ5JDA0JE5qYzNORGcxT0RJdk1tUmpNVGM0TWVEUnRoSU1QZDdSbUk1MFBmWmg2TzlCeC9TMVc0WlhD";

    #[test]
    fn lookup_is_intact() {
        assert_eq!(lookup().unwrap().len(), 930);
    }

    #[test]
    fn all_100_segments_decode() {
        for r in 0..100 {
            let seg = segment_for(r).unwrap_or_else(|_| panic!("余数 {r} 解码失败"));
            assert_eq!(seg.chars().count(), 96, "余数 {r} segment 长度异常");
        }
    }

    #[test]
    fn deterministic_for_same_input() {
        let a = create(DEVICE, APP_CODE, FIXED_TS).unwrap();
        let b = create(DEVICE, APP_CODE, FIXED_TS).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn matches_python_reference() {
        let token = create(DEVICE, APP_CODE, FIXED_TS).unwrap();
        assert_eq!(token, EXPECTED, "与 Python 参考实现不一致");
    }

    #[test]
    fn structure_is_valid() {
        let token = create(DEVICE, APP_CODE, FIXED_TS).unwrap();
        assert!(token.starts_with("v3"));
        let raw = STANDARD.decode(&token[2..]).unwrap();
        assert_eq!(raw.len(), 60);
        assert!(raw.starts_with(b"$2y$"));
        let text = String::from_utf8(raw).unwrap();
        assert_eq!(&text[4..6], "04");
    }

    #[test]
    fn changes_with_timestamp() {
        // 注意：相邻时间戳可能因跳过非法盐而落到同一个可用槽位，
        // 因此这里用间隔足够远的时间戳来验证时间敏感性。
        let a = create(DEVICE, APP_CODE, FIXED_TS).unwrap();
        let b = create(DEVICE, APP_CODE, FIXED_TS + 100).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn adjacent_timestamps_may_converge() {
        // 记录一个真实特性：160 与 161 会跳过非法盐后落到同一槽位，
        // 这不是 bug，而是算法容错机制的必然结果。
        let a = create(DEVICE, APP_CODE, FIXED_TS).unwrap();
        let b = create(DEVICE, APP_CODE, FIXED_TS + 1).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn salt_with_trailing_nonzero_bits_is_rejected() {
        // 22 字符 × 6 位 = 132 位，末尾 4 位必须为 0 才是合法 bcrypt 盐。
        // "Njc3NDg1ODEvN2RiMjg4OR" 正则合法但末尾残余非 0，必须被拒。
        assert!(decode_bcrypt_salt("Njc3NDg1ODEvN2RiMjg4OR").is_none());
        // 合法对照
        assert!(decode_bcrypt_salt("Njc3NDg1ODIvMmRjMTc4Me").is_some());
    }

    #[test]
    fn changes_with_device() {
        let a = create(DEVICE, APP_CODE, FIXED_TS).unwrap();
        let b = create("ffffffffffffffffffffffffffffffff", APP_CODE, FIXED_TS).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn bcrypt_salt_roundtrip() {
        // 该盐来自 Python 实测，应能还原成 16 字节
        let salt = "Njc3NDg1ODAvY2E4Zjg0Nu";
        let raw = decode_bcrypt_salt(salt).expect("盐应可解码");
        assert_eq!(hex::encode(raw), "3e57b93c58b74050b16b81ba6e58b63f");
    }
}

