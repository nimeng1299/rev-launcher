use std::path::PathBuf;
use crate::error::Error;
use crate::project::game_project::GameProject;




fn detective_path(project: &GameProject) -> PathBuf{
    project.path.join("rev-launcher")
}

pub fn serialize_mods(project: &GameProject) -> Result<String, crate::error::Error> {
    let path = detective_path(&project).join("mods");
    std::fs::create_dir_all(&path)?;




    Err(Error::UnknownError)
}