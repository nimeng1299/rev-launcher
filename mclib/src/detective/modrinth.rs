pub mod modrinth_info;

use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

use serde::Serialize;
use sha1::{Digest, Sha1};

use crate::detective::modrinth::modrinth_info::{parse_version_files_response, ModrinthInfo};
use crate::error::Error;

pub fn modrinth_sha1_bytes(data: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(data);
    hex::encode(hasher.finalize())
}

pub fn modrinth_sha1<P: AsRef<Path>>(path: P) -> Result<String, Error> {
    let path = path.as_ref();
    let mut file = std::fs::File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;
    Ok(modrinth_sha1_bytes(&buf))
}

#[derive(Serialize)]
struct VersionFilesPayload {
    algorithm: &'static str,
    hashes: Vec<String>,
}

/// 批量查询文件的 Modrinth version 信息，返回 sha1 -> version 信息的映射。
/// 查询不到的 hash 不会出现在返回值中。
pub fn find_modrinth_info<P: AsRef<Path>>(
    paths: Vec<P>,
    ua: &str,
) -> Result<HashMap<String, ModrinthInfo>, Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent(ua)
        .build()
        .new_agent();

    let mut hashes = vec![];

    for path in paths {
        if let Ok(h) = modrinth_sha1(path) {
            hashes.push(h);
        }
    }

    let payload = VersionFilesPayload {
        algorithm: "sha1",
        hashes,
    };

    let mut resp = agent
        .post("https://api.modrinth.com/v2/version_files")
        .header("accept", "application/json")
        .header("Content-Type", "application/json")
        .send_json(&payload)?;

    if !resp.status().is_success() {
        return Err(ureq::Error::StatusCode(resp.status().as_u16()).into());
    }

    let body = resp.body_mut().read_to_string()?;
    parse_version_files_response(&body)
}

/// 使用启动器的ua来获取信息
/// 支持批量查询，批量查询更快更节省资源
/// 目前使用的ua: "RevLauncher/0.1"
pub fn find_modrinth_info_with_rev_ua<P: AsRef<Path>>(
    paths: Vec<P>,
) -> Result<HashMap<String, ModrinthInfo>, Error> {
    find_modrinth_info(paths, "RevLauncher/0.1")
}
