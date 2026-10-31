//! coolall 协议核心。
//!
//! 这个 crate **不依赖 tauri / GUI**，可以独立编译与测试。
//! 设计目的：把协议逻辑与桌面外壳解耦，使得：
//!
//! 1. 单元测试不需要安装 WebKitGTK 等重型系统依赖
//! 2. 未来若要接入别的外壳（Tauri / 命令行 / 移动端），核心可复用
//!
//! 模块说明见 `../docs/ARCHITECTURE.md`。

pub mod device;
pub mod error;
pub mod request;
pub mod token_v3;

pub use error::Error;
pub use token_v3::TokenError;
