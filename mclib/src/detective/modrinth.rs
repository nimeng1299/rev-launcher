pub mod modrinth_info;

use std::io::Read;
use std::path::Path;
use sha1::{Digest, Sha1};
use crate::detective::modrinth::modrinth_info::{parse_fingerprint_response, ModrinthInfo};
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


pub fn find_modrinth_info<P: AsRef<Path>>(
    path: P,
    ua: &str,
) -> Result<ModrinthInfo, crate::error::Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent(ua)
        .build()
        .new_agent();


    let sha1 = modrinth_sha1(path)?;


    let mut resp = agent
        .get("https://api.modrinth.com/modrinth/v2/version_file")
        .query("algorithm", "sha1")
        .query("hash", sha1)
        .call()?;

    if !resp.status().is_success() {
        return Err(ureq::Error::StatusCode(resp.status().as_u16()).into());
    }

    let body = resp.body_mut().read_to_string()?;
    parse_fingerprint_response(&body)

}

/// 使用启动器的ua来获取信息
/// 支持批量查询，批量查询更快更节省资源
/// 目前使用的ua: "RevLauncher/0.1"
pub fn find_modrinth_info_with_rev_ua<P: AsRef<Path>>(
    path: P,
) -> Result<ModrinthInfo, crate::error::Error> {
    find_modrinth_info(path, "RevLauncher/0.1")
}
