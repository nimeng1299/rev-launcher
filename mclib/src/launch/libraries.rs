//! 支持库准备：清单下载、已安装实例的缓存复用，以及各加载器的隐式运行依赖。
use super::arguments::{
    RuleContext, library_path, maven_path, native_artifact, relative_path, rules_allow,
};
use super::loaders::LoaderProfile;
use super::logging::LaunchLog;
use super::{LaunchError, LaunchLogSource, Launcher};
use crate::project::game_project::ModLoader;
use serde::Deserialize;
use serde_json::Value;
use sha1::{Digest, Sha1};
use sharingan::downloader::{DownloadBuilder, Downloader};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

fn failed(message: impl Into<String>) -> LaunchError {
    LaunchError::DownloadFailed(message.into())
}

#[derive(Clone, Default, Deserialize)]
struct Metadata {
    url: Option<String>,
    sha1: Option<String>,
    size: Option<u64>,
}

#[derive(Clone)]
pub(super) struct Artifact {
    path: PathBuf,
    metadata: Metadata,
    generated: bool,
}

impl Artifact {
    fn from_json(path: PathBuf, json: &Value) -> Result<Self, LaunchError> {
        let mut metadata: Metadata =
            serde_json::from_value(json.clone()).map_err(LaunchError::Json)?;
        metadata.url = metadata.url.filter(|url| !url.is_empty());
        metadata.sha1 = metadata.sha1.filter(|sha1| !sha1.is_empty());
        if let Some(sha1) = &metadata.sha1
            && (sha1.len() != 40 || !sha1.bytes().all(|c| c.is_ascii_hexdigit()))
        {
            return Err(failed(format!("支持库 SHA-1 无效：{}", path.display())));
        }
        Ok(Self {
            path,
            metadata,
            generated: false,
        })
    }

    pub(super) fn maven(name: &str, repository: Option<&str>) -> Result<Self, LaunchError> {
        let path = maven_path(name)?;
        let url = repository.map(|base| {
            format!(
                "{}/{}",
                base.trim_end_matches('/'),
                path.to_string_lossy().replace('\\', "/")
            )
        });
        Ok(Self {
            path,
            metadata: Metadata {
                url,
                ..Metadata::default()
            },
            generated: repository.is_none(),
        })
    }

    fn valid(&self, path: &Path) -> bool {
        let Ok(file) = File::open(path) else {
            return false;
        };
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        if !metadata.is_file()
            || metadata.len() == 0
            || self
                .metadata
                .size
                .is_some_and(|size| size != metadata.len())
        {
            return false;
        }
        if let Some(expected) = &self.metadata.sha1 {
            let mut reader = BufReader::new(file);
            let mut hasher = Sha1::new();
            let mut buffer = [0; 64 * 1024];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(count) => hasher.update(&buffer[..count]),
                    Err(_) => return false,
                }
            }
            return expected.eq_ignore_ascii_case(&hex::encode(hasher.finalize()));
        }
        // 生成文件和旧式 Maven 清单没有哈希，至少拒绝空文件、截断 JAR 和 HTML 错误页。
        // 使用清单中的扩展名，因为下载器传入的文件名带有临时后缀。
        if self
            .path
            .extension()
            .is_some_and(|ext| ext == "jar" || ext == "zip")
        {
            return zip::ZipArchive::new(file).is_ok_and(|archive| !archive.is_empty());
        }
        true
    }
}

fn artifacts(json: &Value, loader: &LoaderProfile) -> Result<Vec<Artifact>, LaunchError> {
    let libraries = json
        .get("libraries")
        .and_then(Value::as_array)
        .ok_or_else(|| failed("版本 JSON 缺少 libraries 数组"))?;
    let context = RuleContext::current(false);
    let mut result: Vec<Artifact> = Vec::new();
    let mut indices = HashMap::new();
    for library in libraries {
        if !rules_allow(library, &context)? {
            continue;
        }
        let mut entries = Vec::new();
        if let Some(path) = library_path(library)? {
            let mut artifact = Artifact::from_json(
                path,
                library.pointer("/downloads/artifact").unwrap_or(library),
            )?;
            // 旧式清单的 url 是 Maven 仓库根地址；显式空 URL 表示本地安装产物。
            if library.get("downloads").is_none() {
                let base = library
                    .get("url")
                    .and_then(Value::as_str)
                    .unwrap_or("https://libraries.minecraft.net");
                artifact.metadata.url = (!base.is_empty()).then(|| {
                    format!(
                        "{}/{}",
                        base.trim_end_matches('/'),
                        artifact.path.to_string_lossy().replace('\\', "/")
                    )
                });
            }
            entries.push(artifact);
        }
        if let Some(native) = native_artifact(library)? {
            let path = native
                .get("path")
                .and_then(Value::as_str)
                .ok_or_else(|| failed("native 下载信息缺少 path"))?;
            entries.push(Artifact::from_json(relative_path(path)?, &native)?);
        }
        for artifact in entries {
            if !indices.contains_key(&artifact.path) {
                indices.insert(artifact.path.clone(), result.len());
                result.push(artifact);
            }
        }
    }
    for artifact in loader.runtime_libraries()? {
        if let Some(&index) = indices.get(&artifact.path) {
            // 保留清单给出的哈希和大小；安装生成的 JAR 不猜测下载地址。
            let existing = &mut result[index];
            existing.generated = artifact.generated;
            if artifact.generated || existing.metadata.url.is_none() {
                existing.metadata.url = artifact.metadata.url;
            }
        } else {
            indices.insert(artifact.path.clone(), result.len());
            result.push(artifact);
        }
    }
    Ok(result)
}

fn logging_artifact(json: &Value) -> Result<Option<Artifact>, LaunchError> {
    if json.pointer("/logging/client/argument").is_none() {
        return Ok(None);
    }
    let file = json
        .pointer("/logging/client/file")
        .ok_or_else(|| failed("logging.client 缺少配置文件"))?;
    let name = file
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| failed("logging.client 缺少配置文件 id"))?;
    Artifact::from_json(logging_cache_path(name)?, file).map(Some)
}

pub(super) fn logging_cache_path(name: &str) -> Result<PathBuf, LaunchError> {
    let name = relative_path(name)?;
    if name.components().count() != 1 || name.file_name().is_none() {
        return Err(failed("logging.client.file.id 必须是文件名"));
    }
    Ok(PathBuf::from(".rev_launcher/log_configs").join(name))
}

fn source_roots(instance: &Path) -> Vec<PathBuf> {
    let mut roots = vec![instance.join("libraries")];
    if let Some(versions) = instance.parent()
        && versions.file_name().is_some_and(|name| name == "versions")
        && let Some(root) = versions.parent()
    {
        roots.push(root.join("libraries"));
    }
    roots
}

fn copy_library(source: &Path, target: &Path, artifact: &Artifact) -> Result<(), LaunchError> {
    let parent = target
        .parent()
        .ok_or_else(|| failed("支持库路径缺少父目录"))?;
    let copy = || -> Result<(), Box<dyn std::error::Error>> {
        std::fs::create_dir_all(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        std::io::copy(&mut File::open(source)?, temporary.as_file_mut())?;
        temporary.as_file().sync_all()?;
        if !artifact.valid(temporary.path()) {
            return Err("复制后的支持库校验失败".into());
        }
        temporary.persist(target)?;
        Ok(())
    };
    copy().map_err(|e| {
        failed(format!(
            "复用支持库失败：{} -> {}：{e}",
            source.display(),
            target.display()
        ))
    })
}

pub(super) fn download(
    data: &Launcher,
    log: Option<&LaunchLog>,
) -> Result<Downloader, LaunchError> {
    let json = data
        .project
        .read_json()
        .map_err(|e| failed(e.to_string()))?;
    let loader = LoaderProfile::from_json(&json, &data.project.loader)?;
    if let Some(log) = log {
        log.write(
            LaunchLogSource::Launcher,
            &format!("加载器：{}；按对应加载器准备支持库", loader.name()),
        )?;
    }
    let mut artifacts = artifacts(&json, &loader)?;
    let logging = logging_artifact(&json)?;
    artifacts.extend(logging.iter().cloned());
    let roots = source_roots(&data.project.path);
    let mut pending = Vec::new();
    let mut missing = Vec::new();
    let mut reused = 0;
    for artifact in artifacts {
        let target = data.setting.libraries_path.join(&artifact.path);
        if artifact.valid(&target) {
            continue;
        }
        let mut sources: Vec<_> = roots.iter().map(|root| root.join(&artifact.path)).collect();
        if logging
            .as_ref()
            .is_some_and(|logging| logging.path == artifact.path)
        {
            // 导入实例已有的日志配置也可复用，公共配置缺失时存入启动器缓存。
            for root in &roots {
                if let Some(parent) = root.parent() {
                    sources.push(
                        parent
                            .join("assets/log_configs")
                            .join(artifact.path.file_name().unwrap()),
                    );
                }
            }
            if let Some(parent) = data.setting.libraries_path.parent() {
                sources.push(
                    parent
                        .join("assets/log_configs")
                        .join(artifact.path.file_name().unwrap()),
                );
            }
        }
        if let Some(source) = sources.into_iter().find(|path| artifact.valid(path)) {
            copy_library(&source, &target, &artifact)?;
            reused += 1;
            if let Some(log) = log {
                log.write(
                    LaunchLogSource::Launcher,
                    &format!("复用支持库：{} -> {}", source.display(), target.display()),
                )?;
            }
        } else if artifact.metadata.url.is_some() {
            pending.push(artifact);
        } else {
            missing.push(format!(
                "{}{}",
                target.display(),
                if artifact.generated {
                    format!("（{} 安装生成文件）", loader.name())
                } else {
                    "（无下载地址）".to_owned()
                }
            ));
        }
    }
    if !missing.is_empty() {
        return Err(failed(format!(
            "缺少无法直接下载的 {} 支持库：\n{}\n已检查实例 libraries 和原 .minecraft/libraries。{}",
            loader.name(),
            missing.join("\n"),
            loader.installation_hint()
        )));
    }
    if let Some(log) = log {
        log.write(
            LaunchLogSource::Launcher,
            &format!(
                "支持库检查完成：复用 {reused} 个文件，需要下载 {} 个文件",
                pending.len()
            ),
        )?;
    }
    let mut downloader = DownloadBuilder::new().thread_num(8).build();
    for artifact in pending {
        let target = data.setting.libraries_path.join(&artifact.path);
        let filename = target
            .file_name()
            .ok_or_else(|| failed("支持库路径缺少文件名"))?
            .to_string_lossy()
            .into_owned();
        let parent = target
            .parent()
            .ok_or_else(|| failed("支持库路径缺少父目录"))?
            .to_path_buf();
        downloader.download(move |builder| {
            let validator = artifact.clone();
            builder
                .filename(filename.clone())
                .path(parent.clone())
                .url(artifact.metadata.url.clone().expect("已检查下载地址"))
                .overwrite(true)
                .validator(move |path| validator.valid(&path))
                .build()
        });
    }
    Ok(downloader)
}

pub(super) fn validate_runtime(
    json: &Value,
    root: &Path,
    fallback: &ModLoader,
) -> Result<(), LaunchError> {
    let loader = LoaderProfile::from_json(json, fallback)?;
    let missing: Vec<_> = loader
        .runtime_libraries()?
        .into_iter()
        .filter_map(|artifact| {
            let path = root.join(&artifact.path);
            (!artifact.valid(&path)).then(|| path.display().to_string())
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(LaunchError::LaunchFailed(format!(
        "{} 运行库缺失或损坏，请先准备支持库：\n{}",
        loader.name(),
        missing.join("\n")
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::tests::{TestProject, write_jar};
    use serde_json::json;
    use std::io::Write;
    use std::time::{Duration, Instant};

    fn artifacts(json: &Value) -> Result<Vec<Artifact>, LaunchError> {
        super::artifacts(
            json,
            &LoaderProfile::from_json(json, &ModLoader::Minecraft)?,
        )
    }

    fn runtime(json: &Value) -> Result<Vec<Artifact>, LaunchError> {
        LoaderProfile::from_json(json, &ModLoader::Minecraft)?.runtime_libraries()
    }

    fn forge_json() -> Value {
        json!({"id":"instance", "mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher",
            "javaVersion":{"majorVersion":21}, "libraries":[],
            "arguments":{"jvm":["-DlibraryDirectory=${library_directory}", "-cp", "${classpath}"],
                "game":["--launchTarget","forgeclient", "--fml.mcVersion","1.20.1",
                    "--fml.forgeVersion","47.3.7", "--fml.mcpVersion","20230612.114412",
                    "--fml.forgeGroup","net.minecraftforge"]}})
    }

    // 来自 Creative Drawers Producer 2 的关键字段；NeoForge 仍使用 forgeclient 入口。
    fn neoforge_json() -> Value {
        json!({"id":"instance", "mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher",
            "javaVersion":{"majorVersion":21}, "libraries":[],
            "patches":[{"id":"game","version":"1.21.1"},{"id":"neoforge","version":"21.1.241"}],
            "arguments":{"jvm":["-DlibraryDirectory=${library_directory}","-cp","${classpath}"],
                "game":["--fml.neoForgeVersion","21.1.241","--fml.fmlVersion","4.0.43",
                    "--fml.mcVersion","1.21.1","--fml.neoFormVersion","20240808.144430",
                    "--launchTarget","forgeclient"]}})
    }

    fn write_manifest(data: &Launcher, json: &Value) {
        std::fs::create_dir_all(&data.project.path).unwrap();
        std::fs::write(data.project.path.join("instance.json"), json.to_string()).unwrap();
    }

    #[test]
    fn forge_implicit_dependencies_include_generated_jars_and_language_loaders() {
        let artifacts = artifacts(&forge_json()).unwrap();
        assert_eq!(artifacts.len(), 8);
        let paths: Vec<_> = artifacts
            .iter()
            .map(|artifact| artifact.path.clone())
            .collect();
        for path in [
            "net/minecraft/client/1.20.1-20230612.114412/client-1.20.1-20230612.114412-srg.jar",
            "net/minecraft/client/1.20.1-20230612.114412/client-1.20.1-20230612.114412-extra.jar",
            "net/minecraftforge/forge/1.20.1-47.3.7/forge-1.20.1-47.3.7-client.jar",
            "net/minecraftforge/forge/1.20.1-47.3.7/forge-1.20.1-47.3.7-universal.jar",
            "net/minecraftforge/fmlcore/1.20.1-47.3.7/fmlcore-1.20.1-47.3.7.jar",
            "net/minecraftforge/javafmllanguage/1.20.1-47.3.7/javafmllanguage-1.20.1-47.3.7.jar",
            "net/minecraftforge/lowcodelanguage/1.20.1-47.3.7/lowcodelanguage-1.20.1-47.3.7.jar",
            "net/minecraftforge/mclanguage/1.20.1-47.3.7/mclanguage-1.20.1-47.3.7.jar",
        ] {
            assert!(paths.contains(&PathBuf::from(path)), "{path}");
        }
        assert_eq!(
            artifacts
                .iter()
                .filter(|artifact| artifact.generated)
                .count(),
            3
        );
        for artifact in artifacts {
            if artifact.generated {
                assert!(artifact.metadata.url.is_none());
            } else {
                assert!(
                    artifact
                        .metadata
                        .url
                        .unwrap()
                        .starts_with("https://maven.minecraftforge.net/")
                );
            }
        }
    }

    #[test]
    fn neoforge_uses_neoform_and_neoforge_coordinates_with_forgeclient_target() {
        let result = artifacts(&neoforge_json()).unwrap();
        assert_eq!(result.len(), 4);
        for (path, generated) in [
            (
                "net/minecraft/client/1.21.1-20240808.144430/client-1.21.1-20240808.144430-srg.jar",
                true,
            ),
            (
                "net/minecraft/client/1.21.1-20240808.144430/client-1.21.1-20240808.144430-extra.jar",
                true,
            ),
            (
                "net/neoforged/neoforge/21.1.241/neoforge-21.1.241-client.jar",
                true,
            ),
            (
                "net/neoforged/neoforge/21.1.241/neoforge-21.1.241-universal.jar",
                false,
            ),
        ] {
            let artifact = result
                .iter()
                .find(|artifact| artifact.path == PathBuf::from(path))
                .expect(path);
            assert_eq!(artifact.generated, generated);
            if generated {
                assert!(artifact.metadata.url.is_none());
            } else {
                assert_eq!(
                    artifact.metadata.url.as_deref(),
                    Some(
                        "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.241/neoforge-21.1.241-universal.jar"
                    )
                );
            }
        }
        // 早期 NeoForge 的 MCP 布局也必须使用自己的 group 和仓库。
        let mut legacy = forge_json();
        let args = legacy["arguments"]["game"].as_array_mut().unwrap();
        *args.last_mut().unwrap() = json!("net.neoforged");
        let legacy = artifacts(&legacy).unwrap();
        assert!(legacy.iter().any(|artifact| artifact.path
            == PathBuf::from("net/neoforged/forge/1.20.1-47.3.7/forge-1.20.1-47.3.7-client.jar")));
        assert!(
            legacy
                .iter()
                .filter_map(|artifact| artifact.metadata.url.as_deref())
                .all(|url| url.starts_with("https://maven.neoforged.net/releases/"))
        );
    }

    #[test]
    fn imported_neoforge_is_prepared_and_launch_arguments_are_preserved() {
        let project = TestProject::new();
        let mut data = project.launcher();
        data.project.path = project.path.join("hmcl/.minecraft/versions/instance");
        data.project.loader = ModLoader::Forge; // 陈旧的缓存元数据不能覆盖 JSON 的 NeoForge 声明。
        data.setting.java.as_mut().unwrap().path_buf = std::env::current_exe().unwrap();
        data.setting.memory = Some(512);
        let mut json = neoforge_json();
        let config = b"<Configuration/>";
        json["logging"] = json!({"client":{"argument":"-Dlog4j.configurationFile=${path}",
            "file":{"id":"client-1.12.xml","url":"https://example.invalid/client-1.12.xml",
                "size":config.len(),"sha1":hex::encode(Sha1::digest(config))}}});
        json["assetIndex"] = json!({"id":"17"});
        json["arguments"]["game"]
            .as_array_mut()
            .unwrap()
            .extend([json!("--assetsDir"), json!("${assets_root}")]);
        write_manifest(&data, &json);
        write_jar(&data.project.path.join("instance.jar"));
        let source = project.path.join("hmcl/.minecraft/libraries");
        let assets = project.path.join("hmcl/.minecraft/assets");
        std::fs::create_dir_all(assets.join("indexes")).unwrap();
        std::fs::create_dir_all(assets.join("log_configs")).unwrap();
        std::fs::write(assets.join("indexes/17.json"), b"{}").unwrap();
        std::fs::write(assets.join("log_configs/client-1.12.xml"), config).unwrap();
        for artifact in artifacts(&json).unwrap() {
            write_jar(&source.join(&artifact.path));
        }
        let error = format!("{:?}", data.launch_command().unwrap_err());
        assert!(error.contains("NeoForge 运行库缺失或损坏"));
        let log = LaunchLog::new(&project.path.join("test-log"), "secret").unwrap();
        let downloader = download(&data, Some(&log)).unwrap();
        assert!(downloader.tasks().is_empty());
        assert!(
            log.entries()
                .iter()
                .any(|entry| entry.message.contains("加载器：NeoForge"))
        );
        assert!(
            !data
                .setting
                .libraries_path
                .join("net/minecraftforge")
                .exists()
        );
        let command = data.launch_command().unwrap();
        assert!(
            command.arguments.contains(&format!(
                "-Dlog4j.configurationFile={}",
                data.setting
                    .libraries_path
                    .join(logging_cache_path("client-1.12.xml").unwrap())
                    .display()
            ))
        );
        assert!(
            command
                .arguments
                .windows(2)
                .any(|pair| pair[0] == "--assetsDir" && Path::new(&pair[1]) == assets)
        );
        for (key, value) in [
            ("--fml.neoForgeVersion", "21.1.241"),
            ("--fml.fmlVersion", "4.0.43"),
            ("--fml.mcVersion", "1.21.1"),
            ("--fml.neoFormVersion", "20240808.144430"),
            ("--launchTarget", "forgeclient"),
        ] {
            assert!(
                command
                    .arguments
                    .windows(2)
                    .any(|pair| pair == [key, value])
            );
        }
        assert!(
            !command
                .arguments
                .iter()
                .any(|arg| arg == "--fml.forgeVersion" || arg == "--fml.mcpVersion")
        );
    }

    #[test]
    fn missing_neoforge_files_report_neoforge_installer_and_correct_parameters() {
        let project = TestProject::new();
        let mut data = project.launcher();
        data.project.loader = ModLoader::Neoforge;
        let mut json = neoforge_json();
        write_manifest(&data, &json);
        let error = match data.download_libraries() {
            Err(error) => format!("{error:?}"),
            Ok(_) => panic!("应当报告缺少 NeoForge 安装产物"),
        };
        assert!(error.contains("对应版本 NeoForge 安装器"));
        assert!(error.contains("neoforge-21.1.241-client.jar"));
        assert!(!error.contains("对应版本 Forge 安装器"));
        json["arguments"]["game"] =
            json!(["--launchTarget", "forgeclient", "--fml.mcVersion", "1.21.1"]);
        let error = match runtime(&json) {
            Err(error) => format!("{error:?}"),
            Ok(_) => panic!("应当检查 NeoForge 参数"),
        };
        assert!(error.contains("NeoForge 启动参数缺少 --fml.neoForgeVersion"));
    }

    #[test]
    fn vanilla_and_fabric_only_use_manifest_libraries() {
        for (main, coordinate) in [
            ("net.minecraft.client.main.Main", "org.example:vanilla:1"),
            (
                "net.fabricmc.loader.impl.launch.knot.KnotClient",
                "net.fabricmc:fabric-loader:0.16.10",
            ),
        ] {
            let json = json!({"mainClass":main,"libraries":[{"name":coordinate}]});
            assert!(runtime(&json).unwrap().is_empty());
            assert_eq!(artifacts(&json).unwrap().len(), 1);
        }
    }

    #[test]
    fn imported_forge_cache_is_repaired_before_building_command() {
        let project = TestProject::new();
        let mut data = project.launcher();
        data.project.path = project.path.join("原安装目录/.minecraft/versions/instance");
        data.setting.java.as_mut().unwrap().path_buf = std::env::current_exe().unwrap();
        data.setting.memory = Some(512);
        let source = project.path.join("原安装目录/.minecraft/libraries");
        let mut json = forge_json();
        json["libraries"] =
            json!([{"name":"example:declared:1", "url":"https://invalid.example/"}]);
        write_manifest(&data, &json);
        write_jar(&data.project.path.join("instance.jar"));
        let all = artifacts(&json).unwrap();
        for artifact in &all {
            write_jar(&source.join(&artifact.path));
        }
        write_jar(&source.join("unrelated/old-version.jar"));
        // 中断复制留下的截断 JAR 也必须修复。
        let corrupt = data.setting.libraries_path.join(&all[1].path);
        std::fs::create_dir_all(corrupt.parent().unwrap()).unwrap();
        std::fs::write(&corrupt, b"PK truncated").unwrap();
        let error = data.launch_command().unwrap_err();
        assert!(format!("{error:?}").contains("Forge 运行库缺失或损坏"));
        let log = LaunchLog::new(&project.path.join("test-log"), "secret").unwrap();
        let downloader = download(&data, Some(&log)).unwrap();
        assert!(downloader.tasks().is_empty());
        assert!(downloader.is_all_success());
        for artifact in all {
            assert_eq!(
                std::fs::read(source.join(&artifact.path)).unwrap(),
                std::fs::read(data.setting.libraries_path.join(&artifact.path)).unwrap()
            );
        }
        assert!(
            !data
                .setting
                .libraries_path
                .join("unrelated/old-version.jar")
                .exists()
        );
        let command = data.launch_command().unwrap();
        assert!(command.arguments.contains(&format!(
            "-DlibraryDirectory={}",
            data.setting.libraries_path.display()
        )));
        assert!(
            log.entries()
                .iter()
                .any(|entry| entry.message.contains("复用 9 个文件"))
        );
        // 不依赖原缓存仍可重复准备，不提交重复任务。
        data.project.path = project.path.join("instance");
        write_manifest(&data, &json);
        assert!(data.download_libraries().unwrap().tasks().is_empty());
    }

    #[test]
    fn missing_forge_outputs_fail_at_library_stage_without_starting_java() {
        let project = TestProject::new();
        let data = project.launcher();
        write_manifest(&data, &forge_json());
        let launch = crate::launch::LaunchInfo::launch(data);
        let deadline = Instant::now() + Duration::from_secs(5);
        while launch.state() != crate::launch::LaunchState::Failed {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            launch.failed_state(),
            Some(crate::launch::LaunchState::DownloadLibrary)
        );
        assert!(launch.process_id().is_none());
        let error = format!("{:?}", launch.error().unwrap());
        for name in ["-srg.jar", "-extra.jar", "-client.jar", "安装处理器生成"] {
            assert!(error.contains(name), "{error}");
        }
        assert!(!project.path.join("libraries").exists());
    }

    #[test]
    fn same_size_corruption_is_repaired_using_manifest_hash() {
        let project = TestProject::new();
        let data = project.launcher();
        let contents = b"good";
        let path = "example/demo/1/demo-1.jar";
        let json = json!({"libraries":[{"downloads":{"artifact":{
            "path":path,"url":"", "size":contents.len(),"sha1":hex::encode(Sha1::digest(contents))}}}]});
        write_manifest(&data, &json);
        let source = data.project.path.join("libraries").join(path);
        let target = data.setting.libraries_path.join(path);
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&source, contents).unwrap();
        std::fs::write(&target, b"evil").unwrap();
        assert!(data.download_libraries().unwrap().tasks().is_empty());
        assert_eq!(std::fs::read(target).unwrap(), contents);
    }

    #[test]
    fn name_only_maven_library_downloads_from_repository_and_deduplicates() {
        let project = TestProject::new();
        let data = project.launcher();
        let jar = project.path.join("download.jar");
        write_jar(&jar);
        let bytes = std::fs::read(jar).unwrap();
        let expected = bytes.clone();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(5);
            let (mut stream, _) = loop {
                if let Ok(connection) = listener.accept() {
                    break connection;
                }
                assert!(Instant::now() < deadline, "没有收到下载请求");
                std::thread::sleep(Duration::from_millis(10));
            };
            stream
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut request = String::new();
            std::io::BufRead::read_line(
                &mut BufReader::new(stream.try_clone().unwrap()),
                &mut request,
            )
            .unwrap();
            assert!(
                request.starts_with("GET /repo/org/example/demo/2/demo-2-client.jar "),
                "{request}"
            );
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .unwrap();
            stream.write_all(&bytes).unwrap();
        });
        let lib =
            json!({"name":"org.example:demo:2:client", "url":format!("http://{address}/repo/")});
        write_manifest(&data, &json!({"libraries":[lib.clone(),lib]}));
        let downloader = data.download_libraries().unwrap();
        assert_eq!(downloader.tasks().len(), 1);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !downloader.is_finished() {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(10));
        }
        server.join().unwrap();
        assert!(downloader.is_all_success());
        assert_eq!(
            std::fs::read(
                data.setting
                    .libraries_path
                    .join("org/example/demo/2/demo-2-client.jar")
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    fn library_metadata_handles_optional_fields_and_platform_rules() {
        let os = if cfg!(target_os = "macos") {
            "osx"
        } else {
            std::env::consts::OS
        };
        let json = json!({"libraries":[
            {"name":"org.example:legacy:1"},
            {"name":"org.example:data:2@zip", "downloads":{"artifact":{"url":"https://example.invalid/data.zip"}}},
            {"name":"org.example:local:1", "url":""},
            {"name":"org.example:disabled:1", "rules":[{"action":"allow","os":{"name":"other"}}]},
            {"natives":{os:"native"},"downloads":{"classifiers":{"native":{"path":"native.jar","url":"https://example.invalid/native.jar"}}}}
        ]});
        let result = artifacts(&json).unwrap();
        assert_eq!(result.len(), 4);
        assert_eq!(
            result[0].metadata.url.as_deref(),
            Some("https://libraries.minecraft.net/org/example/legacy/1/legacy-1.jar")
        );
        assert_eq!(
            result[1].path,
            PathBuf::from("org/example/data/2/data-2.zip")
        );
        assert!(result[2].metadata.url.is_none());
        assert_eq!(result[3].path, PathBuf::from("native.jar"));
    }

    #[test]
    fn generated_entries_keep_hashes_but_never_use_guessed_download_urls() {
        let mut json = forge_json();
        json["libraries"] = json!([{"name":"net.minecraft:client:1.20.1-20230612.114412:srg", "sha1":"0123456789012345678901234567890123456789"}]);
        let result = artifacts(&json).unwrap();
        assert_eq!(result.len(), 8);
        assert!(result[0].generated);
        assert!(result[0].metadata.url.is_none());
        assert!(result[0].metadata.sha1.is_some());
    }

    #[test]
    fn forge_options_support_equals_and_do_not_apply_to_other_loaders() {
        let mut json = forge_json();
        json["arguments"]["game"] = json!([
            "--launchTarget=forgeclient",
            "--fml.mcVersion=1.19.2",
            "--fml.forgeVersion=43.4.0",
            "--fml.mcpVersion=20220805.130853"
        ]);
        let result = runtime(&json).unwrap();
        assert!(result.iter().any(|artifact| artifact.path
            == PathBuf::from(
                "net/minecraftforge/forge/1.19.2-43.4.0/forge-1.19.2-43.4.0-client.jar"
            )));
        json["arguments"]["game"][0] = json!("--launchTarget=neoforgeclient");
        assert!(runtime(&json).unwrap().is_empty());
        json["mainClass"] = json!("net.minecraft.client.main.Main");
        assert!(runtime(&json).unwrap().is_empty());
    }

    #[test]
    fn unsafe_library_coordinates_are_rejected() {
        assert!(logging_cache_path("../config.xml").is_err());
        assert!(logging_cache_path(".").is_err());
        for name in [
            "org.example:../escape:1",
            "org..example:demo:1",
            "org.example:demo:../../1",
            "org.example:demo:1@../jar",
        ] {
            assert!(maven_path(name).is_err(), "{name}");
        }
        assert!(
            artifacts(&json!({"libraries":[{"downloads":{"artifact":{"path":"../outside.jar"}}}]}))
                .is_err()
        );
    }
}
