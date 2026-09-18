pub mod curse_finger_print;

use std::io::Read;
use std::path::Path;
use serde::Serialize;
use crate::detective::curseforge::curse_finger_print::{
    parse_file_info_response, parse_fingerprint_response, FileInfo, FingerprintMatches,
};

#[derive(Serialize)]
struct Payload {
    fingerprints: Vec<u32>,
}

/// 标准 MurmurHash2 64 位实现
fn murmurhash2(data: &[u8], seed: u32) -> u32 {
    const M: u32 = 0x5bd1e995;
    const R: u32 = 24;

    let mut h = seed ^ (data.len() as u32);
    let mut chunks = data.chunks_exact(4);

    for chunk in &mut chunks {
        let mut k = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
        k = k.wrapping_mul(M);
        k ^= k >> R;
        k = k.wrapping_mul(M);

        h = h.wrapping_mul(M);
        h ^= k;
    }

    let rem = chunks.remainder();
    match rem.len() {
        3 => {
            h ^= (rem[2] as u32) << 16;
            h ^= (rem[1] as u32) << 8;
            h ^= rem[0] as u32;
            h = h.wrapping_mul(M);
        }
        2 => {
            h ^= (rem[1] as u32) << 8;
            h ^= rem[0] as u32;
            h = h.wrapping_mul(M);
        }
        1 => {
            h ^= rem[0] as u32;
            h = h.wrapping_mul(M);
        }
        _ => {}
    }

    h ^= h >> 13;
    h = h.wrapping_mul(M);
    h ^= h >> 15;

    h
}

/// CurseForge 指纹：过滤掉空格、制表符、换行、回车后，用 MurmurHash2（seed=1）
pub fn fingerprint_bytes(data: &[u8]) -> u32 {
    let filtered: Vec<u8> = data
        .iter()
        .copied()
        .filter(|b| !matches!(*b, b' ' | b'\t' | b'\n' | b'\r'))
        .collect();

    murmurhash2(&filtered, 1)
}

pub fn fingerprint<P: AsRef<Path>>(path: P) -> Result<u32, std::io::Error> {
    let path = path.as_ref();
    let mut file = std::fs::File::open(path)?;

    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    Ok(fingerprint_bytes(&buf))
}

/// 批量查询文件的 CurseForge 指纹匹配信息。
///
/// 每次请求最多携带 [`super::QUERY_BATCH_SIZE`] 个指纹，超出时自动分批，
/// 最后把各批结果合并返回。
pub fn find_curse_info<P: AsRef<Path>>(
    paths: Vec<P>,
    ua: &str,
) -> Result<FingerprintMatches, crate::error::Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent(ua)
        .build()
        .new_agent();

    let mut fingers = vec![];

    for path in paths {
        if let Ok(f) = fingerprint(path) {
            fingers.push(f);
        }
    }

    let mut merged = FingerprintMatches::default();

    for batch in fingers.chunks(super::QUERY_BATCH_SIZE) {
        let payload = Payload {
            fingerprints: batch.to_vec(),
        };

        let mut resp = agent
            .post("https://mod.mcimirror.top/curseforge/v1/fingerprints")
            .header("accept", "application/json")
            .header("Content-Type", "application/json")
            .send_json(&payload)?;

        if !resp.status().is_success() {
            return Err(ureq::Error::StatusCode(resp.status().as_u16()).into());
        }

        let body = resp.body_mut().read_to_string()?;
        let matches = parse_fingerprint_response(&body)?;

        merged.exact_fingerprints.extend(matches.exact_fingerprints);
        merged.exact_matches.extend(matches.exact_matches);
        merged.installed_fingerprints.extend(matches.installed_fingerprints);
        merged.is_cache_built |= matches.is_cache_built;
        merged
            .partial_match_fingerprints
            .extend(matches.partial_match_fingerprints);
        merged.partial_matches.extend(matches.partial_matches);
        merged
            .unmatched_fingerprints
            .extend(matches.unmatched_fingerprints);
    }

    Ok(merged)
}

/// 使用启动器的ua来获取信息
/// 支持批量查询，批量查询更快更节省资源
/// 目前使用的ua: "RevLauncher/0.1"
pub fn find_curse_info_with_rev_ua<P: AsRef<Path>>(
    paths: Vec<P>,
) -> Result<FingerprintMatches, crate::error::Error> {
    find_curse_info(paths, "RevLauncher/0.1")
}

/// 通过 mod id（project id）和 file id 获取文件信息
/// 接口：GET /curseforge/v1/mods/{mod_id}/files/{file_id}
pub fn find_curse_file_info(
    mod_id: i64,
    file_id: i64,
    ua: &str,
) -> Result<FileInfo, crate::error::Error> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent(ua)
        .build()
        .new_agent();

    let mut resp = agent
        .get(format!(
            "https://mod.mcimirror.top/curseforge/v1/mods/{mod_id}/files/{file_id}"
        ))
        .header("accept", "application/json")
        .call()?;

    if !resp.status().is_success() {
        return Err(ureq::Error::StatusCode(resp.status().as_u16()).into());
    }

    let body = resp.body_mut().read_to_string()?;
    parse_file_info_response(&body)
}

/// 使用启动器的ua通过 id 获取文件信息
/// 目前使用的ua: "RevLauncher/0.1"
pub fn find_curse_file_info_with_rev_ua(
    mod_id: i64,
    file_id: i64,
) -> Result<FileInfo, crate::error::Error> {
    find_curse_file_info(mod_id, file_id, "RevLauncher/0.1")
}
