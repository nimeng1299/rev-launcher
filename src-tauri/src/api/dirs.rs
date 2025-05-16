use anyhow::{bail, Result};
use std::path::PathBuf;

pub fn get_config_dirs() -> Result<PathBuf> {
    if let Some(mut config_dirs) = dirs_next::config_dir() {
        config_dirs.push("rev-launcher");
        return get_and_create_dir(config_dirs);
    } else {
        bail!("Config directory not found");
    }
}

fn get_and_create_dir(path: PathBuf) -> Result<PathBuf> {
    if !path.exists() {
        std::fs::create_dir_all(&path)?;
    }

    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_config_dirs() {
        let config_dir = get_config_dirs();
        assert!(config_dir.is_ok());
        assert!(config_dir.unwrap().exists());
    }
}
