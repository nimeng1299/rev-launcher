use serde::{Deserialize, Serialize};

use crate::error::Error;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MinecreftVersions {
    pub latest: Latest,
    pub versions: Vec<Version>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Latest{
    pub release: String,
    pub snapshot: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Version{
    pub id: String,
    #[serde(rename = "type")]
    pub _type: String,
    pub url: String,
    pub time: String,
    #[serde(rename = "releaseTime")]
    pub release_time: String,
}

impl MinecreftVersions {
    const URL: &'static str = "https://piston-meta.mojang.com/mc/game/version_manifest.json";

    /// 从 Mojang 的 piston-meta 接口获取所有 Minecraft 版本信息
    pub fn get_versions() -> Result<Self, Error>{
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .user_agent("RevLauncher/0.1")
            .build()
            .new_agent();

        let mut resp = agent
            .get(Self::URL)
            .header("accept", "application/json")
            .call()?;

        if !resp.status().is_success() {
            return Err(ureq::Error::StatusCode(resp.status().as_u16()).into());
        }

        let body = resp.body_mut().read_to_string()?;
        Ok(serde_json::from_str(&body)?)
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_get_versions() {
        let versions = MinecreftVersions::get_versions().unwrap();
        assert_eq!(versions.latest.release, "26.3".to_string());
    }
}