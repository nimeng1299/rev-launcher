pub mod progress;

use std::path::Path;
use crate::java::java_version::JavaVersion;
use crate::project::install::progress::InstallProgress;
use crate::project::versions;

/// 从已有的项目文件夹读取项目并按加载器安装：原版安装客户端 jar
/// 和资源文件；Forge 还会下载安装器、支持库并执行 processors。
///
/// path: 项目文件夹（其中应有 `<文件夹名>.json` 版本清单或
/// `.rev_launcher/project.json`）；libraries_path: 支持库目录
/// （应使用 `settings.libraries_path`）；assets_path: 资源文件目录
/// （应使用 `settings.assets_path`）；java: 运行 Forge 安装处理器
/// 的 Java，原版安装可传 `None`。
pub fn install_form_folder<P: AsRef<Path>>(
    path: P,
    libraries_path: P,
    assets_path: P,
    java: Option<JavaVersion>,
) -> InstallProgress {
    InstallProgress::install_form_folder(path, libraries_path, assets_path, java)
}

/// 安装原版
///
/// name: 名字
/// path: 项目的父路径（项目将安装在`path.join(name)`）
/// assets_path: 资源文件目录（应使用与启动时一致的 `settings.assets_path`，
/// 即 `.minecraft/assets`），assetIndex 和 objects 都会下载到这里。
pub fn install_minecraft<P: AsRef<Path>>(
    name: &String,
    path: P,
    assets_path: P,
    version: &versions::minecreft::Version,
) -> InstallProgress{
    InstallProgress::install_minecraft(name, path, assets_path, version)
}

/// 安装 Forge。
///
/// name: 名字；path: 项目的父路径（项目将安装在 `path.join(name)`）；
/// libraries_path: 支持库目录（安装所需的库和处理器产物都放在这里，
/// 应使用与启动时一致的 `settings.libraries_path`）；
/// assets_path: 资源文件目录（应使用 `settings.assets_path`）；
/// java: 用于运行安装处理器的 Java；version: 要安装的 Forge 版本。
///
/// 在 `<name>/temp` 中下载并解压安装器，解析 `install_profile.json`，
/// 下载缺失的依赖库和资源文件后依次执行 processors，最后把合并后的
/// `version.json` 写成 `<name>.json`。
pub fn install_forge<P: AsRef<Path>>(
    name: &String,
    path: P,
    libraries_path: P,
    assets_path: P,
    java: JavaVersion,
    version: &versions::forge::Version,
) -> InstallProgress{
    InstallProgress::install_forge(name, path, libraries_path, assets_path, java, version)
}
