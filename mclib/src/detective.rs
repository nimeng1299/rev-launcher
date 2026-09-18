//! 用于从curseforge和modrinth中获取信息和下载
//! 支持第三方api
//! 所有操作均为同步操作（包括网络请求）

pub mod curseforge;
pub mod modrinth;
pub mod base;
pub mod mod_info;

/// 单次批量查询请求携带的最大文件数，防止一次请求过大导致超时
pub(crate) const QUERY_BATCH_SIZE: usize = 10;