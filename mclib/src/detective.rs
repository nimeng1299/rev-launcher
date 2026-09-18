//! 用于从curseforge和modrinth中获取信息和下载
//! 支持第三方api
//! 所有操作均为同步操作（包括网络请求）

pub mod curseforge;
pub mod modrinth;
pub mod base;