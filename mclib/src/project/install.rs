pub mod progress;

use std::path::PathBuf;
use crate::error::Error;
use crate::project::game_project::GameProject;

pub fn install_git_project(path_buf: PathBuf) -> Result<GameProject, Error>{
    todo!("install_git_project");


    //Err(Error::UnknownError)
}