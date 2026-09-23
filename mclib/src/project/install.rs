pub mod progress;

use std::path::{Path, PathBuf};
use crate::error::Error;
use crate::java::java_version::JavaVersion;
use crate::project::game_project::GameProject;
use crate::project::install::progress::InstallProgress;
use crate::project::versions;

pub fn install_git_project(_path_buf: PathBuf) -> Result<GameProject, Error>{
    todo!("install_git_project");


    //Err(Error::UnknownError)
}

/// 安装原版
///
/// name: 名字
/// path: 项目的父路径（项目将安装在`path.join(name)`）
pub fn install_minecraft<P: AsRef<Path>>(
    name: &String,
    path: P,
    version: &versions::minecreft::Version,
) -> InstallProgress{
    InstallProgress::install_minecraft(name, path, version)
}

/// 安装 Forge。
///
/// name: 名字；path: 项目的父路径（项目将安装在 `path.join(name)`）；
/// libraries_path: 支持库目录（安装所需的库和处理器产物都放在这里，
/// 应使用与启动时一致的 `settings.libraries_path`）；
/// java: 用于运行安装处理器的 Java；version: 要安装的 Forge 版本。
///
/// 在 `<name>/temp` 中下载并解压安装器，解析 `install_profile.json`，
/// 下载缺失的依赖库后依次执行 processors，最后把合并后的
/// `version.json` 写成 `<name>.json`。
pub fn install_forge<P: AsRef<Path>>(
    name: &String,
    path: P,
    libraries_path: P,
    java: JavaVersion,
    version: &versions::forge::Version,
) -> InstallProgress{
    InstallProgress::install_forge(name, path, libraries_path, java, version)
}
