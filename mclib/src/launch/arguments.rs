use super::Launcher;
use crate::account::AccountType;
use crate::java::java_version::JavaVersion;
use crate::settings::GameWindowSize;
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use sysinfo::System;

/// 已展开的启动命令。参数逐项传给 Java，不经过 shell，也不需要手动添加引号。
///
/// `arguments` 包含登录凭据；记录命令时应使用脱敏后的日志。
#[derive(Debug)]
pub struct LaunchCommand {
    pub java: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    pub natives_directory: PathBuf,
    native_archives: Vec<(PathBuf, Vec<String>)>,
}

pub(super) fn invalid(message: impl Into<String>) -> crate::error::Error {
    crate::error::Error::LaunchFailed(message.into())
}

pub(crate) fn relative_path(value: &str) -> Result<PathBuf, crate::error::Error> {
    // 同时检查两种路径分隔符，避免在不同平台上解释出不同的目标路径。
    let path = PathBuf::from(value.replace('\\', "/"));
    if value.is_empty()
        || value.contains(':')
        || path
            .components()
            .any(|part| !matches!(part, Component::Normal(_) | Component::CurDir))
    {
        return Err(invalid(format!("无效的相对路径：{value}")));
    }
    Ok(path)
}

pub(crate) fn library_path(library: &Value) -> Result<Option<PathBuf>, crate::error::Error> {
    if let Some(path) = library
        .pointer("/downloads/artifact/path")
        .and_then(Value::as_str)
    {
        return relative_path(path).map(Some);
    }
    if library.get("downloads").is_some() && library.pointer("/downloads/artifact").is_none() {
        return Ok(None);
    }
    let name = library
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("支持库缺少 name 和 downloads.artifact.path"))?;
    maven_path(name).map(Some)
}

pub(crate) fn maven_path(name: &str) -> Result<PathBuf, crate::error::Error> {
    let (coordinate, extension) = name.split_once('@').unwrap_or((name, "jar"));
    let parts: Vec<_> = coordinate.split(':').collect();
    if !(3..=4).contains(&parts.len())
        || parts
            .iter()
            .chain(std::iter::once(&extension))
            .any(|part| part.is_empty() || part.contains(['/', '\\', ':', '@']) || *part == "..")
        || parts[0].split('.').any(str::is_empty)
    {
        return Err(invalid(format!("无效的 Maven 坐标：{name}")));
    }
    let classifier = parts
        .get(3)
        .map(|part| format!("-{part}"))
        .unwrap_or_default();
    relative_path(&format!(
        "{}/{}/{}/{}-{}{classifier}.{extension}",
        parts[0].replace('.', "/"),
        parts[1],
        parts[2],
        parts[1],
        parts[2]
    ))
}

pub(super) fn native_artifact(library: &Value) -> Result<Option<Value>, crate::error::Error> {
    let os = if cfg!(target_os = "macos") {
        "osx"
    } else {
        std::env::consts::OS
    };
    let Some(classifier) = library
        .get("natives")
        .and_then(|n| n.get(os))
        .and_then(Value::as_str)
    else {
        return Ok(None);
    };
    let classifier = classifier.replace(
        "${arch}",
        if cfg!(target_pointer_width = "64") {
            "64"
        } else {
            "32"
        },
    );
    library
        .pointer("/downloads/classifiers")
        .and_then(|c| c.get(&classifier))
        .cloned()
        .map(Some)
        .ok_or_else(|| invalid(format!("缺少 native 下载信息：{classifier}")))
}

pub(crate) struct RuleContext {
    pub os: String,
    pub arch: String,
    pub version: String,
    pub custom_resolution: bool,
}

impl RuleContext {
    pub fn current(custom_resolution: bool) -> Self {
        Self {
            os: if cfg!(target_os = "macos") {
                "osx"
            } else {
                std::env::consts::OS
            }
            .into(),
            arch: match std::env::consts::ARCH {
                "x86_64" => "amd64",
                other => other,
            }
            .into(),
            version: System::kernel_version().unwrap_or_default(),
            custom_resolution,
        }
    }
}

pub(crate) fn rules_allow(value: &Value, context: &RuleContext) -> Result<bool, crate::error::Error> {
    let Some(rules) = value.get("rules") else {
        return Ok(true);
    };
    let rules = rules
        .as_array()
        .ok_or_else(|| invalid("rules 必须是数组"))?;
    let mut allowed = false;
    for rule in rules {
        if let Some(os) = rule.get("os") {
            if let Some(name) = os.get("name").and_then(Value::as_str)
                && name != context.os
            {
                continue;
            }
            if let Some(arch) = os.get("arch").and_then(Value::as_str) {
                let arch = if arch == "x86_64" { "amd64" } else { arch };
                if arch != context.arch {
                    continue;
                }
            }
            if let Some(version) = os.get("version").and_then(Value::as_str) {
                let pattern =
                    Regex::new(version).map_err(|e| invalid(format!("无效的系统版本规则：{e}")))?;
                if !pattern.is_match(&context.version) {
                    continue;
                }
            }
        }
        if let Some(features) = rule.get("features") {
            let features = features
                .as_object()
                .ok_or_else(|| invalid("features 必须是对象"))?;
            if features.iter().any(|(name, expected)| {
                let actual = name == "has_custom_resolution" && context.custom_resolution;
                expected.as_bool() != Some(actual)
            }) {
                continue;
            }
        }
        allowed = match rule.get("action").and_then(Value::as_str) {
            Some("allow") => true,
            Some("disallow") => false,
            _ => return Err(invalid("未知的规则 action")),
        };
    }
    Ok(allowed)
}

fn expand(template: &str, variables: &HashMap<&str, String>) -> Result<String, crate::error::Error> {
    let mut result = String::new();
    let mut remaining = template;
    while let Some(start) = remaining.find("${") {
        result.push_str(&remaining[..start]);
        let variable = &remaining[start + 2..];
        let end = variable
            .find('}')
            .ok_or_else(|| invalid(format!("未闭合的占位符：{template}")))?;
        let name = &variable[..end];
        result.push_str(
            variables
                .get(name)
                .ok_or_else(|| invalid(format!("不支持的启动参数占位符：${{{name}}}")))?,
        );
        remaining = &variable[end + 1..];
    }
    result.push_str(remaining);
    Ok(result)
}

fn expand_arguments(
    value: &Value,
    context: &RuleContext,
    variables: &HashMap<&str, String>,
) -> Result<Vec<String>, crate::error::Error> {
    let values = value
        .as_array()
        .ok_or_else(|| invalid("arguments.jvm/game 必须是数组"))?;
    let mut arguments = Vec::new();
    for value in values {
        if let Some(value) = value.as_str() {
            arguments.push(expand(value, variables)?);
        } else if rules_allow(value, context)? {
            match value.get("value") {
                Some(Value::String(value)) => arguments.push(expand(value, variables)?),
                Some(Value::Array(values)) => {
                    for value in values {
                        arguments.push(expand(
                            value
                                .as_str()
                                .ok_or_else(|| invalid("参数 value 数组只能包含字符串"))?,
                            variables,
                        )?);
                    }
                }
                _ => return Err(invalid("条件参数缺少有效的 value")),
            }
        }
    }
    Ok(arguments)
}

fn require_file(path: &Path) -> Result<(), crate::error::Error> {
    if path.is_file() {
        Ok(())
    } else {
        Err(invalid(format!("缺少启动文件：{}", path.display())))
    }
}

impl LaunchCommand {
    pub(super) fn build(
        data: &Launcher,
        json: &Value,
        java: &JavaVersion,
    ) -> Result<Self, crate::error::Error> {
        if json.get("inheritsFrom").is_some() {
            return Err(invalid(
                "请先合并 inheritsFrom 指定的父版本 JSON，再启动此版本",
            ));
        }
        let working_directory =
            std::path::absolute(&data.project.path).map_err(|e| invalid(e.to_string()))?;
        let libraries = std::path::absolute(&data.setting.libraries_path)
            .map_err(|e| invalid(e.to_string()))?;
        super::libraries::validate_runtime(json, &libraries, &data.project.loader)?;
        let java_path = std::path::absolute(&java.path_buf).map_err(|e| invalid(e.to_string()))?;
        require_file(&java_path)?;
        let version = json
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or(&data.project.name);
        let jar_name = json.get("jar").and_then(Value::as_str).unwrap_or(version);
        let jar_file = relative_path(&format!("{jar_name}.jar"))?;
        if jar_file.components().count() != 1 {
            return Err(invalid("jar 必须是版本名称"));
        }
        let mut client = working_directory.join(&jar_file);
        if !client.is_file()
            && let Some(versions) = working_directory.parent()
        {
            let inherited = versions.join(jar_name).join(&jar_file);
            if inherited.is_file() {
                client = inherited;
            }
        }
        require_file(&client)?;
        let natives_directory = working_directory
            .join(".rev_launcher/natives")
            .join(format!(
                "{}-{}",
                std::env::consts::OS,
                std::env::consts::ARCH
            ));
        let custom_resolution = matches!(data.setting.window_size, GameWindowSize::Windowed(_, _));
        let context = RuleContext::current(custom_resolution);
        let library_context = RuleContext::current(false);
        let mut classpath = Vec::new();
        let mut native_archives = Vec::new();
        let library_list = json
            .get("libraries")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("版本 JSON 缺少 libraries 数组"))?;
        for library in library_list {
            if !rules_allow(library, &library_context)? {
                continue;
            }
            if let Some(path) = library_path(library)? {
                let path = libraries.join(path);
                require_file(&path)?;
                if !classpath.contains(&path) {
                    classpath.push(path);
                }
            }
            if let Some(artifact) = native_artifact(library)? {
                let path = artifact
                    .get("path")
                    .and_then(Value::as_str)
                    .ok_or_else(|| invalid("native 下载信息缺少 path"))?;
                let path = libraries.join(relative_path(path)?);
                require_file(&path)?;
                let excludes = library
                    .pointer("/extract/exclude")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                native_archives.push((path, excludes));
            }
        }
        classpath.push(client.clone());
        let classpath = std::env::join_paths(&classpath)
            .map_err(|e| invalid(format!("无效的 classpath：{e}")))?
            .into_string()
            .map_err(|_| invalid("classpath 不是有效的 Unicode 路径"))?;

        let default_assets = libraries
            .parent()
            .unwrap_or(&working_directory)
            .join("assets");
        let instance_assets = working_directory.join("assets");
        let shared_assets = working_directory
            .parent()
            .and_then(Path::parent)
            .map(|p| p.join("assets"));
        let assets = [
            Some(default_assets.clone()),
            Some(instance_assets),
            shared_assets,
        ]
        .into_iter()
        .flatten()
        .find(|path| path.is_dir())
        .unwrap_or(default_assets);
        let asset_index = json
            .pointer("/assetIndex/id")
            .or_else(|| json.get("assets"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if !asset_index.is_empty() {
            require_file(
                &assets
                    .join("indexes")
                    .join(relative_path(&format!("{asset_index}.json"))?),
            )?;
        }
        let virtual_assets = assets.join("virtual").join(asset_index);
        let game_assets = if virtual_assets.is_dir() {
            &virtual_assets
        } else {
            &assets
        };
        let (width, height) = match data.setting.window_size {
            GameWindowSize::Windowed(width, height) if width > 0 && height > 0 => (width, height),
            GameWindowSize::Windowed(_, _) => return Err(invalid("窗口宽高必须大于 0")),
            _ => (860, 640),
        };
        let uuid = data.account.uuid.replace('-', "");
        let mut variables = HashMap::from([
            ("auth_player_name", data.account.name.clone()),
            ("auth_uuid", uuid.clone()),
            ("auth_access_token", data.account.token.clone()),
            (
                "auth_session",
                format!("token:{}:{uuid}", data.account.token),
            ),
            (
                "user_type",
                if data.account.account_type == AccountType::Online {
                    "msa"
                } else {
                    "legacy"
                }
                .into(),
            ),
            ("user_properties", "{}".into()),
            ("profile_properties", "{}".into()),
            // 当前 Account 尚未保存这两个微软登录字段，离线启动传入空值。
            ("clientid", String::new()),
            ("auth_xuid", String::new()),
            ("version_name", version.into()),
            (
                "version_type",
                json.get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("release")
                    .into(),
            ),
            (
                "game_directory",
                working_directory.to_string_lossy().into_owned(),
            ),
            ("assets_root", assets.to_string_lossy().into_owned()),
            ("assets_index_name", asset_index.into()),
            ("game_assets", game_assets.to_string_lossy().into_owned()),
            (
                "natives_directory",
                natives_directory.to_string_lossy().into_owned(),
            ),
            (
                "library_directory",
                libraries.to_string_lossy().into_owned(),
            ),
            ("classpath", classpath),
            (
                "classpath_separator",
                if cfg!(windows) { ";" } else { ":" }.into(),
            ),
            (
                "primary_jar_name",
                client.file_name().unwrap().to_string_lossy().into_owned(),
            ),
            ("launcher_name", "rev-launcher".into()),
            ("launcher_version", env!("CARGO_PKG_VERSION").into()),
            ("resolution_width", width.to_string()),
            ("resolution_height", height.to_string()),
        ]);
        let mut arguments = if let Some(jvm) = json.pointer("/arguments/jvm") {
            expand_arguments(jvm, &context, &variables)?
        } else {
            let mut args = vec![
                expand("-Djava.library.path=${natives_directory}", &variables)?,
                "-cp".into(),
                variables["classpath"].clone(),
            ];
            if cfg!(target_os = "macos") {
                args.insert(0, "-XstartOnFirstThread".into());
            }
            args
        };
        if !arguments.iter().any(|arg| {
            matches!(arg.as_str(), "-cp" | "-classpath" | "--class-path")
                || arg.starts_with("--class-path=")
        }) {
            arguments.extend(["-cp".into(), variables["classpath"].clone()]);
        }
        let memory = data.setting.memory.unwrap_or_else(|| {
            let mut system = System::new();
            system.refresh_memory();
            let available = system.available_memory() / 1024 / 1024;
            if available == 0 {
                2048
            } else {
                (available / 2).clamp(512, 8192) as usize
            }
        });
        if memory == 0 {
            return Err(invalid("分配的内存必须大于 0 MB"));
        }
        arguments.retain(|arg| !arg.starts_with("-Xmx"));
        arguments.push(format!("-Xmx{memory}M"));
        if let Some(logging) = json
            .pointer("/logging/client/argument")
            .and_then(Value::as_str)
        {
            let name = json
                .pointer("/logging/client/file/id")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("logging.client 缺少配置文件 id"))?;
            let cached = libraries.join(super::libraries::logging_cache_path(name)?);
            let path = if cached.is_file() {
                cached
            } else {
                assets.join("log_configs").join(relative_path(name)?)
            };
            require_file(&path)?;
            variables.insert("path", path.to_string_lossy().into_owned());
            arguments.push(expand(logging, &variables)?);
        }
        let main_class = json
            .get("mainClass")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| invalid("版本 JSON 缺少 mainClass"))?;
        arguments.push(main_class.into());
        if let Some(game) = json.pointer("/arguments/game") {
            arguments.extend(expand_arguments(game, &context, &variables)?);
        } else if let Some(legacy) = json.get("minecraftArguments").and_then(Value::as_str) {
            let legacy =
                shlex::split(legacy).ok_or_else(|| invalid("minecraftArguments 引号不匹配"))?;
            for argument in legacy {
                arguments.push(expand(&argument, &variables)?);
            }
            if custom_resolution {
                arguments.extend([
                    "--width".into(),
                    width.to_string(),
                    "--height".into(),
                    height.to_string(),
                ]);
            }
        } else {
            return Err(invalid(
                "版本 JSON 缺少 arguments.game 或 minecraftArguments",
            ));
        }
        if matches!(data.setting.window_size, GameWindowSize::Fullscreen)
            && !arguments.iter().any(|arg| arg == "--fullscreen")
        {
            arguments.push("--fullscreen".into());
        }
        Ok(Self {
            java: java_path,
            arguments,
            working_directory,
            natives_directory,
            native_archives,
        })
    }

    pub(super) fn prepare_natives(&self) -> Result<(), crate::error::Error> {
        std::fs::create_dir_all(&self.natives_directory)
            .map_err(|e| invalid(format!("创建 natives 目录失败：{e}")))?;
        for (path, excludes) in &self.native_archives {
            let file = File::open(path).map_err(|e| invalid(format!("读取 native 库失败：{e}")))?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| invalid(format!("读取 native 压缩包失败：{e}")))?;
            for index in 0..archive.len() {
                let mut entry = archive
                    .by_index(index)
                    .map_err(|e| invalid(e.to_string()))?;
                if entry.is_dir()
                    || entry.name().starts_with("META-INF/")
                    || excludes
                        .iter()
                        .any(|prefix| entry.name().starts_with(prefix))
                {
                    continue;
                }
                if entry
                    .unix_mode()
                    .is_some_and(|mode| mode & 0o170000 == 0o120000)
                {
                    return Err(invalid("native 压缩包包含符号链接"));
                }
                let name = entry
                    .enclosed_name()
                    .ok_or_else(|| invalid("native 压缩包包含越界路径"))?;
                let target = self
                    .natives_directory
                    .join(relative_path(&name.to_string_lossy())?);
                std::fs::create_dir_all(target.parent().unwrap())
                    .map_err(|e| invalid(e.to_string()))?;
                let mut output = File::create(&target)
                    .map_err(|e| invalid(format!("解压 {} 失败：{e}", target.display())))?;
                std::io::copy(&mut entry, &mut output).map_err(|e| invalid(e.to_string()))?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::tests::TestProject;
    use serde_json::json;
    use std::io::Write;

    fn fixture() -> (TestProject, Launcher, Value) {
        let project = TestProject::new();
        std::fs::write(project.path.join("instance/instance.jar"), []).unwrap();
        let mut data = project.launcher();
        data.setting.java.as_mut().unwrap().path_buf = std::env::current_exe().unwrap();
        data.setting.memory = Some(2048);
        data.setting.window_size = GameWindowSize::Windowed(860, 640);
        let json = json!({"id":"instance", "mainClass":"example.Main", "libraries":[],
            "arguments":{"jvm":["-cp", "${classpath}"], "game":[]}});
        (project, data, json)
    }

    fn build(data: &Launcher, json: &Value) -> Result<LaunchCommand, crate::error::Error> {
        LaunchCommand::build(data, json, data.setting.java.as_ref().unwrap())
    }

    #[test]
    fn forge_arguments_preserve_paths_and_filter_features() {
        let (project, mut data, mut json) = fixture();
        data.account.name = "玩家 Test".into();
        let library = "org/example/demo/1/demo-1.jar";
        std::fs::create_dir_all(data.setting.libraries_path.join("org/example/demo/1")).unwrap();
        std::fs::write(data.setting.libraries_path.join(library), []).unwrap();
        std::fs::create_dir_all(project.path.join("assets/indexes")).unwrap();
        std::fs::write(project.path.join("assets/indexes/5.json"), "{}").unwrap();
        json["assetIndex"] = json!({"id":"5"});
        json["libraries"] = json!([{"downloads":{"artifact":{"path":library}}},
            {"name":"other:platform:1", "rules":[{"action":"allow", "os":{"name":"unknown"}}]}]);
        json["mainClass"] = json!("cpw.mods.bootstraplauncher.BootstrapLauncher");
        json["arguments"]["jvm"] = json!([
            "-cp",
            "${classpath}",
            "-Djava.library.path=${natives_directory}",
            "-p",
            "${library_directory}/first.jar${classpath_separator}${library_directory}/second.jar",
            "--add-modules",
            "ALL-MODULE-PATH",
            "-DignoreList=${version_name}.jar,${primary_jar_name}"
        ]);
        json["arguments"]["game"] = json!(["--username", "${auth_player_name}", "--gameDir", "${game_directory}",
            "--assetsDir", "${assets_root}", "--uuid", "${auth_uuid}", "--accessToken", "${auth_access_token}",
            {"rules":[{"action":"allow", "features":{"is_demo_user":true}}],"value":"--demo"},
            {"rules":[{"action":"allow", "features":{"has_quick_plays_support":true}}],"value":["${quickPlayPath}"]},
            {"rules":[{"action":"allow", "features":{"has_custom_resolution":true}}],"value":["--width","${resolution_width}","--height","${resolution_height}"]},
            "--launchTarget", "forgeclient", "--fml.forgeVersion", "47.3.7",
            "--fml.mcVersion", "1.20.1", "--fml.mcpVersion", "20230612.114412"]);
        for coordinate in [
            "net.minecraft:client:1.20.1-20230612.114412:srg",
            "net.minecraft:client:1.20.1-20230612.114412:extra",
            "net.minecraftforge:forge:1.20.1-47.3.7:client",
            "net.minecraftforge:forge:1.20.1-47.3.7:universal",
            "net.minecraftforge:fmlcore:1.20.1-47.3.7",
            "net.minecraftforge:javafmllanguage:1.20.1-47.3.7",
            "net.minecraftforge:lowcodelanguage:1.20.1-47.3.7",
            "net.minecraftforge:mclanguage:1.20.1-47.3.7",
        ] {
            crate::launch::tests::write_jar(
                &data
                    .setting
                    .libraries_path
                    .join(maven_path(coordinate).unwrap()),
            );
        }
        let command = build(&data, &json).unwrap();
        let args = &command.arguments;
        assert!(!args.iter().any(|arg| arg.contains("${") || arg == "--demo"));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--username", "玩家 Test"])
        );
        assert!(args.windows(2).any(|pair| pair == ["--width", "860"]));
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--launchTarget", "forgeclient"])
        );
        assert!(args.iter().any(|arg| arg == "-Xmx2048M"));
        let cp = &args[args.iter().position(|arg| arg == "-cp").unwrap() + 1];
        let paths: Vec<_> = std::env::split_paths(cp).collect();
        assert_eq!(
            paths,
            [
                data.setting.libraries_path.join(library),
                project.path.join("instance/instance.jar")
            ]
        );
        let game = &args[args.iter().position(|arg| arg == "--gameDir").unwrap() + 1];
        assert_eq!(Path::new(game), project.path.join("instance"));
    }

    #[test]
    fn rules_match_os_arch_version_and_last_matching_action() {
        let context = RuleContext {
            os: "windows".into(),
            arch: "amd64".into(),
            version: "10.0.22631".into(),
            custom_resolution: true,
        };
        assert!(rules_allow(&json!({"rules":[{"action":"allow","os":{"name":"windows","arch":"x86_64","version":"^10\\."}}]}), &context).unwrap());
        assert!(
            !rules_allow(
                &json!({"rules":[{"action":"allow","os":{"arch":"x86"}}]}),
                &context
            )
            .unwrap()
        );
        assert!(
            !rules_allow(
                &json!({"rules":[{"action":"allow","os":{"version":"^6\\."}}]}),
                &context
            )
            .unwrap()
        );
        assert!(
            !rules_allow(
                &json!({"rules":[{"action":"allow"},{"action":"disallow"}]}),
                &context
            )
            .unwrap()
        );
        assert!(
            rules_allow(
                &json!({"rules":[{"action":"allow","features":{"unknown_feature":false}}]}),
                &context
            )
            .unwrap()
        );
        assert!(
            rules_allow(
                &json!({"rules":[{"action":"allow","os":{"version":"["}}]}),
                &context
            )
            .is_err()
        );
    }

    #[test]
    fn legacy_arguments_are_split_before_expanding_paths() {
        let (_project, mut data, mut json) = fixture();
        data.account.name = "Player With Spaces".into();
        data.setting.window_size = GameWindowSize::Fullscreen;
        json.as_object_mut().unwrap().remove("arguments");
        json["minecraftArguments"] = json!(
            "--username ${auth_player_name} --gameDir \"${game_directory}\" --userProperties ${user_properties}"
        );
        let command = build(&data, &json).unwrap();
        assert!(
            command
                .arguments
                .windows(2)
                .any(|pair| pair == ["--username", "Player With Spaces"])
        );
        assert!(command.arguments.iter().any(|arg| arg == "--fullscreen"));
        assert!(!command.arguments.iter().any(|arg| arg == "--width"));
        assert!(
            command
                .arguments
                .iter()
                .any(|arg| arg.starts_with("-Djava.library.path="))
        );
    }

    #[test]
    fn invalid_manifest_and_missing_files_return_errors() {
        let (project, data, mut json) = fixture();
        json["arguments"]["game"] = json!(["${unsupported}"]);
        assert!(
            matches!(build(&data, &json), Err(crate::error::Error::LaunchFailed(message)) if message.contains("unsupported"))
        );
        json["arguments"]["game"] = json!([]);
        json["assetIndex"] = json!({"id":"missing"});
        assert!(
            matches!(build(&data, &json), Err(crate::error::Error::LaunchFailed(message)) if message.contains("missing.json"))
        );
        json.as_object_mut().unwrap().remove("assetIndex");
        std::fs::remove_file(project.path.join("instance/instance.jar")).unwrap();
        assert!(
            matches!(build(&data, &json), Err(crate::error::Error::LaunchFailed(message)) if message.contains("instance.jar"))
        );
        for path in ["../outside", "C:\\outside", "/absolute", "..\\outside"] {
            assert!(relative_path(path).is_err());
        }
    }

    #[test]
    fn maven_classifiers_and_native_selection() {
        assert_eq!(
            library_path(&json!({"name":"org.example:demo:1.2:client@zip"}))
                .unwrap()
                .unwrap(),
            PathBuf::from("org/example/demo/1.2/demo-1.2-client.zip")
        );
        let os = if cfg!(target_os = "macos") {
            "osx"
        } else {
            std::env::consts::OS
        };
        let classifier = format!("native-{}", usize::BITS);
        let library = json!({"natives":{os:"native-${arch}"},"downloads":{"classifiers":{classifier:{"path":"native.jar"}}}});
        assert_eq!(
            native_artifact(&library).unwrap().unwrap()["path"],
            "native.jar"
        );
    }

    #[test]
    fn natives_extract_excludes_metadata_and_rejects_traversal() {
        let (project, data, json) = fixture();
        let archive = project.path.join("native.jar");
        let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
        for name in ["lib/native.dll", "META-INF/signature", "excluded.txt"] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            writer.write_all(b"native").unwrap();
        }
        writer.finish().unwrap();
        let mut command = build(&data, &json).unwrap();
        command
            .native_archives
            .push((archive, vec!["excluded".into()]));
        command.prepare_natives().unwrap();
        assert_eq!(
            std::fs::read(command.natives_directory.join("lib/native.dll")).unwrap(),
            b"native"
        );
        assert!(!command.natives_directory.join("META-INF").exists());
        assert!(!command.natives_directory.join("excluded.txt").exists());
        let malicious = project.path.join("malicious.jar");
        let mut writer = zip::ZipWriter::new(File::create(&malicious).unwrap());
        writer
            .start_file("../escaped.dll", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"invalid").unwrap();
        writer.finish().unwrap();
        command.native_archives = vec![(malicious, vec![])];
        assert!(command.prepare_natives().is_err());
        assert!(
            !command
                .natives_directory
                .parent()
                .unwrap()
                .join("escaped.dll")
                .exists()
        );
    }
}
