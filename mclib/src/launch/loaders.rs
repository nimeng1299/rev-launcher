//! 按加载器选择运行依赖；launchTarget 是入口名称，不能作为加载器类型。
use super::arguments::{RuleContext, rules_allow};
use super::libraries::Artifact;
use crate::project::game_project::ModLoader;
use serde_json::Value;

pub(super) struct LoaderProfile {
    pub kind: ModLoader,
    arguments: Vec<String>,
    bootstrap: bool,
}

impl LoaderProfile {
    pub fn from_json(json: &Value, fallback: &ModLoader) -> Result<Self, crate::error::Error> {
        let arguments = game_arguments(json)?;
        let main = json.get("mainClass").and_then(Value::as_str).unwrap_or("");
        let patch = |id: &str| {
            json.get("patches")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|patch| {
                    patch
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|value| value == id || value.starts_with(&format!("{id}:")))
                })
        };
        // 只识别加载器自身的坐标。NeoForge 也会引用 Fabric 的 Mixin 和 Forge 的 srgutils。
        let library = |coordinate: &str| {
            json.get("libraries")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|lib| {
                    lib.get("name").and_then(Value::as_str).is_some_and(|name| {
                        name.strip_prefix(coordinate)
                            .is_some_and(|suffix| suffix.starts_with(':'))
                    })
                })
        };
        let has = |name: &str| {
            arguments.iter().any(|arg| {
                arg == name
                    || arg
                        .strip_prefix(name)
                        .is_some_and(|suffix| suffix.starts_with('='))
            })
        };
        // 当前版本 JSON 的明确证据优先于可能过期的 project.json。
        let kind = if has("--fml.neoForgeVersion")
            || has("--fml.neoFormVersion")
            || option(&arguments, "--fml.forgeGroup") == Some("net.neoforged")
            || patch("neoforge")
            || library("net.neoforged:neoforge")
            || library("net.neoforged:forge")
            || library("net.neoforged:fmlloader")
            || library("net.neoforged.fancymodloader:loader")
        {
            ModLoader::Neoforge
        } else if main.starts_with("net.fabricmc.loader.")
            || patch("fabric")
            || library("net.fabricmc:fabric-loader")
        {
            ModLoader::Fabric
        } else if has("--fml.forgeVersion")
            || patch("forge")
            || library("net.minecraftforge:forge")
            || library("net.minecraftforge:fmlloader")
        {
            ModLoader::Forge
        } else if main == "net.minecraft.client.main.Main" {
            ModLoader::Minecraft
        } else {
            fallback.clone()
        };
        Ok(Self {
            kind,
            arguments,
            bootstrap: main == "cpw.mods.bootstraplauncher.BootstrapLauncher",
        })
    }

    pub fn name(&self) -> &'static str {
        match self.kind {
            ModLoader::Minecraft => "原版 Minecraft",
            ModLoader::Forge => "Forge",
            ModLoader::Neoforge => "NeoForge",
            ModLoader::Fabric => "Fabric",
        }
    }

    pub fn runtime_libraries(&self) -> Result<Vec<Artifact>, crate::error::Error> {
        match self.kind {
            // 原版和 Fabric 的运行库由版本清单列出，不添加 FML 安装产物。
            ModLoader::Minecraft | ModLoader::Fabric => Ok(Vec::new()),
            ModLoader::Forge => self.forge_runtime(),
            ModLoader::Neoforge => self.neoforge_runtime(),
        }
    }

    pub fn installation_hint(&self) -> String {
        match self.kind {
            ModLoader::Forge | ModLoader::Neoforge => format!(
                "请恢复完整的原安装目录，或使用对应版本 {} 安装器完成客户端安装；缺失的生成文件需要安装处理器生成。",
                self.name()
            ),
            ModLoader::Minecraft | ModLoader::Fabric => format!(
                "请恢复完整的原安装目录，或重新安装此 {} 版本并检查支持库下载地址。",
                self.name()
            ),
        }
    }

    fn required(&self, name: &str) -> Result<&str, crate::error::Error> {
        option(&self.arguments, name)
            .filter(|value| !value.is_empty() && !value.starts_with("--"))
            .ok_or_else(|| {
                crate::error::Error::DownloadFailed(format!("{} 启动参数缺少 {name}", self.name()))
            })
    }

    fn forge_runtime(&self) -> Result<Vec<Artifact>, crate::error::Error> {
        if !self.bootstrap || option(&self.arguments, "--launchTarget") != Some("forgeclient") {
            return Ok(Vec::new());
        }
        self.mcp_runtime("net.minecraftforge", "https://maven.minecraftforge.net")
    }

    fn neoforge_runtime(&self) -> Result<Vec<Artifact>, crate::error::Error> {
        if !self.bootstrap
            || !matches!(
                option(&self.arguments, "--launchTarget"),
                Some("forgeclient" | "neoforgeclient")
            )
        {
            return Ok(Vec::new());
        }
        // 1.20.1 的早期 NeoForge 仍使用 Forge/MCP 参数和 net.neoforged:forge 坐标。
        if option(&self.arguments, "--fml.forgeGroup") == Some("net.neoforged")
            && option(&self.arguments, "--fml.neoForgeVersion").is_none()
        {
            return self.mcp_runtime("net.neoforged", "https://maven.neoforged.net/releases");
        }
        let neo = self.required("--fml.neoForgeVersion")?;
        let minecraft = self.required("--fml.mcVersion")?;
        let neoform = self.required("--fml.neoFormVersion")?;
        // FancyModLoader 按 mcAndNeoFormVersion 定位客户端，NeoForge 自身只用 neoForgeVersion。
        let mut artifacts = client_libraries(minecraft, neoform)?;
        artifacts.push(Artifact::maven(
            &format!("net.neoforged:neoforge:{neo}:client"),
            None,
        )?);
        artifacts.push(Artifact::maven(
            &format!("net.neoforged:neoforge:{neo}:universal"),
            Some("https://maven.neoforged.net/releases"),
        )?);
        Ok(artifacts)
    }

    fn mcp_runtime(&self, group: &str, repository: &str) -> Result<Vec<Artifact>, crate::error::Error> {
        let forge = self.required("--fml.forgeVersion")?;
        let minecraft = self.required("--fml.mcVersion")?;
        let mcp = self.required("--fml.mcpVersion")?;
        let mut artifacts = client_libraries(minecraft, mcp)?;
        artifacts.push(Artifact::maven(
            &format!("{group}:forge:{minecraft}-{forge}:client"),
            None,
        )?);
        artifacts.push(Artifact::maven(
            &format!("{group}:forge:{minecraft}-{forge}:universal"),
            Some(repository),
        )?);
        for name in ["fmlcore", "javafmllanguage", "mclanguage"] {
            artifacts.push(Artifact::maven(
                &format!("{group}:{name}:{minecraft}-{forge}"),
                Some(repository),
            )?);
        }
        if forge
            .split('.')
            .next()
            .and_then(|major| major.parse::<u32>().ok())
            .is_some_and(|major| major >= 40)
        {
            artifacts.push(Artifact::maven(
                &format!("{group}:lowcodelanguage:{minecraft}-{forge}"),
                Some(repository),
            )?);
        }
        Ok(artifacts)
    }
}

fn client_libraries(minecraft: &str, mappings: &str) -> Result<Vec<Artifact>, crate::error::Error> {
    ["srg", "extra"]
        .into_iter()
        .map(|classifier| {
            Artifact::maven(
                &format!("net.minecraft:client:{minecraft}-{mappings}:{classifier}"),
                None,
            )
        })
        .collect()
}

fn option<'a>(arguments: &'a [String], name: &str) -> Option<&'a str> {
    arguments.iter().enumerate().rev().find_map(|(index, arg)| {
        if arg == name {
            arguments.get(index + 1).map(String::as_str)
        } else {
            arg.strip_prefix(name)
                .and_then(|value| value.strip_prefix('='))
        }
    })
}

fn game_arguments(json: &Value) -> Result<Vec<String>, crate::error::Error> {
    let Some(values) = json.pointer("/arguments/game").and_then(Value::as_array) else {
        return Ok(Vec::new());
    };
    let context = RuleContext::current(false);
    let mut args = Vec::new();
    for value in values {
        if let Some(value) = value.as_str() {
            args.push(value.to_owned());
        } else if rules_allow(value, &context)? {
            match value.get("value") {
                Some(Value::String(value)) => args.push(value.clone()),
                Some(Value::Array(values)) => {
                    for value in values {
                        args.push(
                            value
                                .as_str()
                                .ok_or_else(|| {
                                    crate::error::Error::DownloadFailed("参数 value 必须为字符串".into())
                                })?
                                .to_owned(),
                        );
                    }
                }
                _ => {
                    return Err(crate::error::Error::DownloadFailed(
                        "条件参数缺少有效的 value".into(),
                    ));
                }
            }
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn detect_loader_from_its_own_metadata_not_shared_dependencies() {
        for (json, fallback, expected) in [
            (
                json!({"mainClass":"cpw.mods.bootstraplauncher.BootstrapLauncher", "arguments":{"game":["--launchTarget","forgeclient","--fml.neoForgeVersion","21.1.241"]}}),
                ModLoader::Forge,
                ModLoader::Neoforge,
            ),
            (
                json!({"patches":[{"id":"game"},{"id":"neoforge"}], "libraries":[{"name":"net.fabricmc:sponge-mixin:1"},{"name":"net.minecraftforge:srgutils:1"}]}),
                ModLoader::Minecraft,
                ModLoader::Neoforge,
            ),
            (
                json!({"libraries":[{"name":"net.neoforged.fancymodloader:loader:4.0.43"}]}),
                ModLoader::Forge,
                ModLoader::Neoforge,
            ),
            (
                json!({"arguments":{"game":["--launchTarget=forgeclient","--fml.forgeVersion=47.3.7"]},"libraries":[{"name":"net.fabricmc:sponge-mixin:1"}]}),
                ModLoader::Minecraft,
                ModLoader::Forge,
            ),
            (
                json!({"libraries":[{"name":"net.minecraftforge:fmlloader:1.20.1-47.3.7"}]}),
                ModLoader::Minecraft,
                ModLoader::Forge,
            ),
            (
                json!({"mainClass":"net.fabricmc.loader.impl.launch.knot.KnotClient"}),
                ModLoader::Forge,
                ModLoader::Fabric,
            ),
            (
                json!({"libraries":[{"name":"net.fabricmc:fabric-loader:0.16.10"}]}),
                ModLoader::Neoforge,
                ModLoader::Fabric,
            ),
            (
                json!({"mainClass":"net.minecraft.client.main.Main"}),
                ModLoader::Forge,
                ModLoader::Minecraft,
            ),
            (
                json!({"arguments":{"game":["--launchTarget","forgeclient"]}}),
                ModLoader::Neoforge,
                ModLoader::Neoforge,
            ),
            (
                json!({"arguments":{"game":["--launchTarget","forgeclient"]},"libraries":[{"name":"net.fabricmc:sponge-mixin:1"},{"name":"net.minecraftforge:srgutils:1"}]}),
                ModLoader::Minecraft,
                ModLoader::Minecraft,
            ),
        ] {
            assert_eq!(
                LoaderProfile::from_json(&json, &fallback).unwrap().kind,
                expected,
                "{json}"
            );
        }
    }

    #[test]
    fn disabled_loader_arguments_do_not_change_detection() {
        let json = json!({"mainClass":"net.minecraft.client.main.Main", "arguments":{"game":[
            {"rules":[{"action":"allow","os":{"name":"unknown"}}],"value":["--fml.neoForgeVersion","21.1.241"]}
        ]}});
        let profile = LoaderProfile::from_json(&json, &ModLoader::Minecraft).unwrap();
        assert_eq!(profile.kind, ModLoader::Minecraft);
        assert!(profile.runtime_libraries().unwrap().is_empty());
    }
}
