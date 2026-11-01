//! 打印 `X-App-Token`，供 tls-probe 的 Python 抓包脚本调用。
//!
//! 签名算法完全复用 `tools/rs-probe` 里已与 Python 参考实现逐字节交叉验证过的
//! `token_probe::create`，本 crate 只做命令行封装。
//!
//! 用法：
//!     tokencli <设备串编码值> [appCode] [unix秒]

use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    let Some(device) = args.get(1) else {
        eprintln!("用法: tokencli <设备串编码值> [appCode] [unix秒]");
        std::process::exit(2);
    };
    let app_code: i64 = args
        .get(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2_610_291);
    let ts: i64 = args
        .get(3)
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| chrono::Utc::now().timestamp());

    match token_probe::create(device, app_code, ts) {
        Ok(token) => println!("{token}"),
        Err(e) => {
            eprintln!("token 生成失败: {e}");
            std::process::exit(1);
        }
    }
}
