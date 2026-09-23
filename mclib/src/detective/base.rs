use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};

use sharingan::downloader::{DownloadBuilder, Downloader};
use sharingan::status::DownloadFailure;
use sharingan::task::DownloadOptions;

use crate::detective::curseforge::{
    find_curse_file_info_with_rev_ua, find_curse_info_with_rev_ua, fingerprint,
};
use crate::detective::mod_info::{CurseforgeInfo, ModInfo, ModrinthInfo};
use crate::detective::modrinth::modrinth_info::ModrinthInfo as ModrinthVersion;
use crate::detective::modrinth::{
    find_modrinth_info_with_rev_ua, find_modrinth_version_with_rev_ua, modrinth_sha1,
};
use crate::error::Error;
use crate::project::game_project::GameProject;

fn detective_path(project: &GameProject) -> PathBuf {
    project.path.join(".rev_launcher")
}

/// 可以被序列化/反序列化的资源类别，和项目下的资源目录一一对应。
///
/// 元数据（描述每个文件的 TOML）写在
/// `<项目>/.rev_launcher/<dir_name>/` 下，资源本身在 `<项目>/<dir_name>/` 下。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceKind {
    /// `mods` 目录，`.jar` 文件。
    Mod,
    /// `resourcepacks` 目录，`.zip` 文件。
    ResourcePack,
    /// `shaderpacks` 目录，`.zip` 文件。
    Shader,
}

impl ResourceKind {
    /// 项目目录下的资源文件夹名，同时是 `.rev_launcher` 下的元数据目录名。
    pub fn dir_name(self) -> &'static str {
        match self {
            Self::Mod => "mods",
            Self::ResourcePack => "resourcepacks",
            Self::Shader => "shaderpacks",
        }
    }

    /// 参与序列化的文件后缀（不带点，比较时忽略大小写）。
    pub fn ext(self) -> &'static str {
        match self {
            Self::Mod => "jar",
            Self::ResourcePack | Self::Shader => "zip",
        }
    }
}

/// [`serialize_mods`] / [`serialize_resources`] 在工作线程中发出的进度事件。
///
/// 通过返回的 channel 接收；channel 关闭（for 循环结束 / `recv` 返回 `Err`）
/// 表示整个序列化流程结束。
#[derive(Debug, Clone)]
pub enum SerializeModsProgress {
    /// 正在查询 CurseForge 指纹
    QueryCurseforge,
    /// 正在查询 Modrinth
    QueryModrinth,
    /// 开始逐个序列化 jar，total 为 jar 总数
    WriteStart { total: usize },
    /// 第 index 个 jar 序列化完成
    WriteDone { index: usize, filename: String },
    /// 全部完成
    Done { total: usize },
}

/// 在独立线程中扫描项目资源目录下的所有文件，通过 CurseForge 指纹和
/// Modrinth sha1 批量查询资源信息，并把每个文件的 [`ModInfo`] 用 toml_edit
/// 序列化到 `<项目>/.rev_launcher/<资源目录>/<文件名>.toml`（文件名在原名后追加
/// `.toml`）。
///
/// 两个平台都没有匹配到的资源也会写入只含 filename 的 TOML 文件。
///
/// 返回接收进度的 channel，事件见 [`SerializeModsProgress`]；
/// 线程执行中出错时 channel 会先发出 `Err` 再关闭。
pub fn serialize_resources(
    project: &GameProject,
    kind: ResourceKind,
) -> Result<mpsc::Receiver<Result<SerializeModsProgress, Error>>, Error> {
    let out_dir = detective_path(project).join(kind.dir_name());
    std::fs::create_dir_all(&out_dir)?;

    let resource_dir = project.path.join(kind.dir_name());
    let files = list_files(&resource_dir, kind.ext())?;

    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let result = serialize_resources_in_thread(files, out_dir, &tx);
        if let Err(e) = result {
            let _ = tx.send(Err(e));
        }
        // tx 在此 drop，接收端遍历随之结束
    });

    Ok(rx)
}

/// 序列化 mods 目录；等价于 `serialize_resources(project, ResourceKind::Mod)`。
pub fn serialize_mods(
    project: &GameProject,
) -> Result<mpsc::Receiver<Result<SerializeModsProgress, Error>>, Error> {
    serialize_resources(project, ResourceKind::Mod)
}

/// 线程内的实际序列化流程，成功时返回处理（写入或跳过）的文件数量。
/// 对应 toml 已存在且记录的 sha1 与当前文件一致时跳过该文件，不重新查询和写入。
fn serialize_resources_in_thread(
    files: Vec<PathBuf>,
    out_dir: PathBuf,
    tx: &Sender<Result<SerializeModsProgress, Error>>,
) -> Result<usize, Error> {
    // 接收端被丢弃时 send 会失败，忽略即可（toml 仍会继续写到磁盘）
    let send = |event: SerializeModsProgress| {
        let _ = tx.send(Ok(event));
    };

    if files.is_empty() {
        send(SerializeModsProgress::WriteStart { total: 0 });
        send(SerializeModsProgress::Done { total: 0 });
        return Ok(0);
    }

    let total = files.len();
    send(SerializeModsProgress::WriteStart { total });

    // 预计算每个文件的 sha1；已序列化过且哈希一致的视为有效，跳过后续查询
    let mut sha1_by_file: HashMap<&PathBuf, String> = HashMap::new();
    let mut fresh: HashSet<&PathBuf> = HashSet::new();
    for (index, file) in files.iter().enumerate() {
        let Some(filename) = file.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Ok(sha1) = modrinth_sha1(file) else {
            continue;
        };
        let toml_path = out_dir.join(format!("{}.toml", filename));
        if let Ok(content) = std::fs::read_to_string(&toml_path)
            && let Ok(info) = toml_edit::de::from_str::<ModInfo>(&content)
            && info.sha1 == sha1
        {
            fresh.insert(file);
            send(SerializeModsProgress::WriteDone {
                index,
                filename: filename.to_string(),
            });
            continue;
        }
        sha1_by_file.insert(file, sha1);
    }

    let to_query: Vec<&PathBuf> = files.iter().filter(|file| !fresh.contains(file)).collect();
    if !to_query.is_empty() {
        send(SerializeModsProgress::QueryCurseforge);
        let curse_matches = find_curse_info_with_rev_ua(to_query.clone())?;
        let mut curse_by_fingerprint: HashMap<i64, CurseforgeInfo> = HashMap::new();
        for m in curse_matches.exact_matches {
            curse_by_fingerprint.insert(
                m.file.file_fingerprint,
                CurseforgeInfo {
                    project_id: m.id,
                    file_id: m.file.id,
                },
            );
        }

        send(SerializeModsProgress::QueryModrinth);
        let modrinth_versions = find_modrinth_info_with_rev_ua(to_query)?;

        for (index, file) in files.iter().enumerate() {
            if fresh.contains(file) {
                continue;
            }
            let Some(filename) = file.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            let curseforge = fingerprint(file)
                .ok()
                .map(|fp| fp as i64)
                .and_then(|fp| curse_by_fingerprint.get(&fp).cloned());

            let modrinth = sha1_by_file
                .get(file)
                .and_then(|sha1| modrinth_versions.get(sha1))
                .map(|version| ModrinthInfo {
                    id: version.id.clone(),
                });

            let info = ModInfo {
                filename: filename.to_string(),
                sha1: sha1_by_file.get(file).cloned().unwrap_or_default(),
                curseforge,
                modrinth,
            };

            let toml = toml_edit::ser::to_string_pretty(&info)?;
            std::fs::write(out_dir.join(format!("{}.toml", filename)), toml)?;

            send(SerializeModsProgress::WriteDone {
                index,
                filename: filename.to_string(),
            });
        }
    }

    send(SerializeModsProgress::Done { total });
    Ok(total)
}

// ---------------------------------------------------------------------------
// 反序列化：读取 toml 信息并下载资源文件
// ---------------------------------------------------------------------------

/// 读取 `<项目>/.rev_launcher/<资源目录>/*.toml` 并提交下载任务，立即返回下载器。
///
/// 文件读取和创建资源目录失败会直接返回 `Err`；平台查询、下载和校验错误记录在
/// 各任务的 `failed_reason()` 中。查询与下载共用 `threads` 个 Worker（至少一个）。
/// 优先使用 Modrinth，失败时尝试 CurseForge；下载失败后换渠道重试一次。
/// 本地文件 SHA-1 一致时跳过，任务成功且 `task.options().skip_download` 为 `true`。
/// 调用方通过 `tasks()`、`is_finished()` 和 `is_all_success()` 查询进度与结果。
/// 需要保留返回的下载器直到任务结束；丢弃下载器会取消任务。
pub fn deserialize_resources(
    project: &GameProject,
    kind: ResourceKind,
    threads: usize,
) -> Result<Downloader, Error> {
    let infos = read_mod_infos(&detective_path(project).join(kind.dir_name()))?;
    let resource_dir = project.path.join(kind.dir_name());
    std::fs::create_dir_all(&resource_dir)?;
    let mut downloader = DownloadBuilder::new().thread_num(threads.max(1)).build();
    for info in infos {
        let path = resource_dir.clone();
        downloader.download(move |builder| {
            builder
                .path(path)
                .filename(info.filename.clone())
                .overwrite(true)
                .before_download(move |options| {
                    let resolved = resolve_download_info(&info, None)
                        .map_err(DownloadFailure::PreparationError)?;
                    let used = resolved.channel;
                    let mut options = prepare_mod_download(options, resolved);
                    options.add_fallback(move |options| {
                        let resolved = resolve_download_info(&info, Some(used))
                            .map_err(DownloadFailure::PreparationError)?;
                        Ok(prepare_mod_download(options, resolved))
                    });
                    Ok(options)
                })
                .build()
        });
    }
    Ok(downloader)
}

/// 反序列化 mods 目录；等价于 `deserialize_resources(project, ResourceKind::Mod, threads)`。
pub fn deserialize_mods(project: &GameProject, threads: usize) -> Result<Downloader, Error> {
    deserialize_resources(project, ResourceKind::Mod, threads)
}

/// 从目录读取所有 toml 文件并反序列化为 [`ModInfo`]，按 filename 排序。
/// 目录不存在视为空。
fn read_mod_infos(dir: &Path) -> Result<Vec<ModInfo>, Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };

    let mut infos = vec![];
    for entry in entries.flatten() {
        let path = entry.path();
        if !(path.is_file()
            && path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("toml")))
        {
            continue;
        }

        let content = std::fs::read_to_string(&path)?;
        let info: ModInfo = toml_edit::de::from_str(&content).map_err(|e| Error::Deserialize {
            path: path.display().to_string(),
            message: e.to_string(),
        })?;
        infos.push(info);
    }

    infos.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(infos)
}

/// mod 下载使用的渠道
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DownloadChannel {
    Curseforge,
    Modrinth,
}

/// 解析出的单个 mod 的下载信息
#[derive(Clone)]
struct ResolvedDownload {
    url: String,
    sha1: String,
    channel: DownloadChannel,
}

/// 解析 mod 的下载地址与 sha1：优先 Modrinth，查询失败或信息不全时再用
/// CurseForge。重试时排除已经使用过的渠道。
fn resolve_download_info(
    mod_info: &ModInfo,
    excluded: Option<DownloadChannel>,
) -> Result<ResolvedDownload, String> {
    let mut errors: Vec<String> = vec![];

    if let Some(mr) = &mod_info.modrinth
        && excluded != Some(DownloadChannel::Modrinth)
    {
        match find_modrinth_version_with_rev_ua(&mr.id) {
            Ok(version) => match primary_version_file(&version) {
                Some((url, sha1)) => {
                    return Ok(ResolvedDownload {
                        url,
                        sha1,
                        channel: DownloadChannel::Modrinth,
                    });
                }
                None => errors.push("Modrinth version 没有可用的文件".to_string()),
            },
            Err(e) => errors.push(format!("Modrinth 查询失败：{e}")),
        }
    }

    if let Some(cf) = &mod_info.curseforge
        && excluded != Some(DownloadChannel::Curseforge)
    {
        match find_curse_file_info_with_rev_ua(cf.project_id, cf.file_id) {
            Ok(info) => {
                if info.download_url.is_empty() {
                    errors.push("CurseForge 文件信息缺少下载地址".to_string());
                } else if let Some(sha1) = info.sha1().map(|sha1| sha1.to_string()) {
                    return Ok(ResolvedDownload {
                        url: info.download_url,
                        sha1,
                        channel: DownloadChannel::Curseforge,
                    });
                } else {
                    errors.push("CurseForge 文件信息缺少 sha1".to_string());
                }
            }
            Err(e) => errors.push(format!("CurseForge 查询失败：{e}")),
        }
    }

    if errors.is_empty() {
        errors.push("没有可用的下载渠道".to_string());
    }
    Err(errors.join("；"))
}

/// 取 Modrinth version 的 primary 文件（没有则取第一个），返回 (url, sha1)。
fn primary_version_file(version: &ModrinthVersion) -> Option<(String, String)> {
    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())?;
    if file.url.is_empty() || file.hashes.sha1.is_empty() {
        return None;
    }
    Some((file.url.clone(), file.hashes.sha1.clone()))
}

/// 下载地址和校验规则都由前置回调确定；本地文件正确时直接跳过。
fn prepare_mod_download(
    mut options: DownloadOptions,
    resolved: ResolvedDownload,
) -> DownloadOptions {
    options.url = resolved.url;
    let expected_sha1 = resolved.sha1;
    let validator = Arc::new(move |path: PathBuf| match modrinth_sha1(&path) {
        Ok(sha1) if sha1 == expected_sha1 => Ok(()),
        Err(Error::Io(error)) => Err(DownloadFailure::IOError(error)),
        _ => Err(DownloadFailure::ValidationError),
    });
    let target = options.path.join(&options.filename);
    options.skip_download = target.is_file() && validator(target).is_ok();
    options.validator = Some(validator);
    options
}

/// 列出目录下所有 `ext` 后缀的文件（扩展名不区分大小写），按路径排序。
/// 目录不存在视为没有文件。
fn list_files(dir: &Path, ext: &str) -> Result<Vec<PathBuf>, Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };

    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|file_ext| file_ext.eq_ignore_ascii_case(ext))
        })
        .collect();
    files.sort();
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::SerializeModsProgress;
    use super::deserialize_mods;
    use super::deserialize_resources;
    use super::list_files;
    use super::read_mod_infos;
    use super::serialize_mods;
    use super::serialize_resources;
    use super::serialize_resources_in_thread;
    use super::{DownloadChannel, ResourceKind, ResolvedDownload, prepare_mod_download, resolve_download_info};
    use crate::detective::mod_info::{CurseforgeInfo, ModInfo, ModrinthInfo};
    use crate::project::game_project::{GameProject, ModLoader};
    use sharingan::status::DownloadFailure;
    use sharingan::task::DownloadOptions;

    fn test_project(path: &std::path::Path) -> GameProject {
        GameProject {
            name: "test".into(),
            version: "".into(),
            path: path.to_path_buf(),
            game_version: "1.21".into(),
            loader: ModLoader::Minecraft,
            loader_version: "1.21".into(),
        }
    }

    #[test]
    fn deserialize_mods_returns_tasks_with_failures_and_zero_threads() {
        let directory = tempfile::tempdir().unwrap();
        let project = test_project(directory.path());
        let metadata = directory.path().join(".rev_launcher/mods");
        std::fs::create_dir_all(&metadata).unwrap();
        for filename in ["a.jar", "b.jar"] {
            let info = ModInfo {
                filename: filename.into(),
                sha1: String::new(),
                curseforge: None,
                modrinth: None,
            };
            std::fs::write(
                metadata.join(format!("{filename}.toml")),
                toml_edit::ser::to_string_pretty(&info).unwrap(),
            )
            .unwrap();
        }
        let mut downloader = deserialize_mods(&project, 0).unwrap();
        assert_eq!(downloader.thread_num, 1);
        assert_eq!(downloader.tasks().len(), 2);
        // 返回的是可继续使用的原始下载器，新增任务 id 不会与 mod 任务冲突。
        downloader.download(|builder| {
            builder
                .filename("extra.jar".into())
                .before_download(|mut options| {
                    options.skip_download = true;
                    Ok(options)
                })
                .build()
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !downloader.is_finished() {
            assert!(std::time::Instant::now() < deadline, "任务结束超时");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(downloader.tasks().len(), 3);
        for id in 0..2 {
            let task = &downloader.tasks()[&id];
            assert!(task.status().is_failed());
            assert!(
                matches!(&*task.failed_reason(), DownloadFailure::PreparationError(message)
                if message.contains("没有可用的下载渠道"))
            );
        }
        assert!(downloader.tasks()[&2].status().is_success());
        assert!(!downloader.is_all_success());
    }

    #[test]
    fn deserialize_mods_reports_setup_errors_directly() {
        let directory = tempfile::tempdir().unwrap();
        let project = test_project(directory.path());
        let metadata = directory.path().join(".rev_launcher/mods");
        std::fs::create_dir_all(&metadata).unwrap();
        std::fs::write(metadata.join("invalid.toml"), "[invalid").unwrap();
        assert!(matches!(
            deserialize_mods(&project, 1),
            Err(super::Error::Deserialize { .. })
        ));
        std::fs::remove_file(metadata.join("invalid.toml")).unwrap();
        std::fs::write(directory.path().join("mods"), "not a directory").unwrap();
        assert!(matches!(
            deserialize_mods(&project, 1),
            Err(super::Error::Io(_))
        ));
    }

    #[test]
    fn prepared_mod_skips_only_matching_file_and_validates_downloaded_content() {
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("cached.jar");
        let content = b"correct mod";
        let resolved = ResolvedDownload {
            url: "http://unused.invalid/mod.jar".into(),
            sha1: crate::detective::modrinth::modrinth_sha1_bytes(content),
            channel: DownloadChannel::Modrinth,
        };
        let initial = DownloadOptions {
            path: directory.path().to_path_buf(),
            filename: "cached.jar".into(),
            ..DownloadOptions::default()
        };
        assert!(!prepare_mod_download(initial.clone(), resolved.clone()).skip_download);
        std::fs::write(&target, content).unwrap();
        let prepared = prepare_mod_download(initial.clone(), resolved.clone());
        assert!(prepared.skip_download);
        assert_eq!(prepared.url, resolved.url);
        let validator = prepared.validator.unwrap();
        validator(target.clone()).unwrap();

        std::fs::write(&target, b"corrupt").unwrap();
        assert!(!prepare_mod_download(initial, resolved).skip_download);
        assert!(matches!(
            validator(target),
            Err(DownloadFailure::ValidationError)
        ));
        assert!(matches!(
            validator(directory.path().join("missing.jar")),
            Err(DownloadFailure::IOError(_))
        ));
    }

    #[test]
    fn retry_excludes_the_used_channel() {
        let info = ModInfo {
            filename: "mod.jar".into(),
            sha1: String::new(),
            curseforge: None,
            modrinth: Some(ModrinthInfo {
                id: "must-not-be-queried".into(),
            }),
        };
        assert!(
            matches!(resolve_download_info(&info, Some(DownloadChannel::Modrinth)),
            Err(message) if message == "没有可用的下载渠道")
        );
        let info = ModInfo {
            filename: "mod.jar".into(),
            sha1: String::new(),
            modrinth: None,
            curseforge: Some(CurseforgeInfo {
                project_id: 0,
                file_id: 0,
            }),
        };
        assert!(
            matches!(resolve_download_info(&info, Some(DownloadChannel::Curseforge)),
            Err(message) if message == "没有可用的下载渠道")
        );
    }

    #[test]
    fn list_files_test() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jar"), b"jar").unwrap();
        std::fs::write(dir.path().join("b.JAR"), b"jar").unwrap();
        std::fs::write(dir.path().join("c.zip"), b"zip").unwrap();
        std::fs::write(dir.path().join("jar.txt"), b"text").unwrap();
        std::fs::create_dir(dir.path().join("d.jar")).unwrap();

        let jars = list_files(dir.path(), "jar").unwrap();
        let names: Vec<String> = jars
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.jar", "b.JAR"]);

        // 按后缀过滤：zip 目录只收 zip
        let zips: Vec<String> = list_files(dir.path(), "zip")
            .unwrap()
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(zips, vec!["c.zip"]);
    }

    #[test]
    fn list_jars_missing_dir_test() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not_exist");
        assert!(list_files(&missing, "jar").unwrap().is_empty());
    }

    #[test]
    fn mod_info_toml_format_test() {
        let info = ModInfo {
            filename: "sodium.jar".to_string(),
            sha1: String::new(),
            curseforge: Some(CurseforgeInfo {
                project_id: 394468,
                file_id: 5594023,
            }),
            modrinth: Some(ModrinthInfo {
                id: "RncWhTxD".to_string(),
            }),
        };
        let toml = toml_edit::ser::to_string_pretty(&info).unwrap();
        assert!(toml.contains("filename = \"sodium.jar\""), "toml: {toml}");
        assert!(toml.contains("[curseforge]"), "toml: {toml}");
        assert!(toml.contains("project_id = 394468"), "toml: {toml}");
        assert!(toml.contains("file_id = 5594023"), "toml: {toml}");
        assert!(toml.contains("[modrinth]"), "toml: {toml}");
        assert!(toml.contains("id = \"RncWhTxD\""), "toml: {toml}");

        // 没匹配到信息时只序列化 filename
        let info = ModInfo {
            filename: "unknown.jar".to_string(),
            sha1: String::new(),
            curseforge: None,
            modrinth: None,
        };
        let toml = toml_edit::ser::to_string_pretty(&info).unwrap();
        assert_eq!(toml.trim(), "filename = \"unknown.jar\"");
    }

    /// 资源目录为空时线程只发 WriteStart{0} 和 Done{0}，不触发网络请求
    #[test]
    fn serialize_mods_progress_empty_test() {
        let dir = tempfile::tempdir().unwrap();
        let project = GameProject {
            name: "test".to_string(),
            version: "".to_string(),
            path: dir.path().to_path_buf(),
            game_version: "1.21".to_string(),
            loader: ModLoader::Minecraft,
            loader_version: "1.21".to_string(),
        };

        let rx = serialize_mods(&project).unwrap();
        let events: Vec<_> = rx.into_iter().collect();

        assert!(
            matches!(
                events.first(),
                Some(Ok(SerializeModsProgress::WriteStart { total: 0 }))
            ),
            "events: {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(Ok(SerializeModsProgress::Done { total: 0 }))
            ),
            "events: {events:?}"
        );
        assert_eq!(events.len(), 2);
    }

    /// toml 已存在且记录的 sha1 与当前文件一致时直接跳过：
    /// 不发起平台查询，也不重写 toml。
    #[test]
    fn serialize_mods_in_thread_skips_fresh_toml_test() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path().join(".rev_launcher/mods");
        std::fs::create_dir_all(&out_dir).unwrap();

        let jar_path = dir.path().join("sodium.jar");
        std::fs::write(&jar_path, b"jar content").unwrap();
        let sha1 = crate::detective::modrinth::modrinth_sha1_bytes(b"jar content");
        let info = ModInfo {
            filename: "sodium.jar".into(),
            sha1,
            curseforge: None,
            modrinth: None,
        };
        let toml_path = out_dir.join("sodium.jar.toml");
        let original = toml_edit::ser::to_string_pretty(&info).unwrap();
        std::fs::write(&toml_path, &original).unwrap();

        let (tx, rx) = std::sync::mpsc::channel();
        let handled = serialize_resources_in_thread(vec![jar_path], out_dir, &tx).unwrap();
        drop(tx);
        let events: Vec<_> = rx.into_iter().collect();

        assert_eq!(handled, 1);
        assert!(
            matches!(events.first(), Some(Ok(SerializeModsProgress::WriteStart { total: 1 }))),
            "events: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(Ok(SerializeModsProgress::Done { total: 1 }))),
            "events: {events:?}"
        );
        // 只有 WriteStart/WriteDone/Done，没有 Query 事件
        assert_eq!(events.len(), 3, "events: {events:?}");
        assert_eq!(std::fs::read_to_string(&toml_path).unwrap(), original);
    }

    /// serialize 写出的 toml 能被 read_mod_infos 读回
    #[test]
    fn read_mod_infos_round_trip_test() {
        let dir = tempfile::tempdir().unwrap();
        let full = ModInfo {
            filename: "sodium.jar".to_string(),
            sha1: String::new(),
            curseforge: Some(CurseforgeInfo {
                project_id: 394468,
                file_id: 5594023,
            }),
            modrinth: Some(ModrinthInfo {
                id: "RncWhTxD".to_string(),
            }),
        };
        let only_name = ModInfo {
            filename: "unknown.jar".to_string(),
            sha1: String::new(),
            curseforge: None,
            modrinth: None,
        };
        for info in [&full, &only_name] {
            std::fs::write(
                dir.path().join(format!("{}.toml", info.filename)),
                toml_edit::ser::to_string_pretty(info).unwrap(),
            )
            .unwrap();
        }
        // 非 toml 文件应被忽略
        std::fs::write(dir.path().join("readme.txt"), "not a mod").unwrap();

        let infos = read_mod_infos(dir.path()).unwrap();
        assert_eq!(infos.len(), 2);
        assert_eq!(infos[0].filename, "sodium.jar");
        assert_eq!(infos[0].curseforge.as_ref().unwrap().project_id, 394468);
        assert_eq!(infos[0].modrinth.as_ref().unwrap().id, "RncWhTxD");
        assert_eq!(infos[1].filename, "unknown.jar");
        assert!(infos[1].curseforge.is_none());
        assert!(infos[1].modrinth.is_none());
    }

    /// 没有 toml 时返回空下载器，不触发网络请求。
    #[test]
    fn deserialize_mods_empty_downloader_test() {
        let dir = tempfile::tempdir().unwrap();
        let downloader = deserialize_mods(&test_project(dir.path()), 10).unwrap();
        assert!(downloader.tasks().is_empty());
        assert!(downloader.is_finished());
        assert!(downloader.is_all_success());
    }

    /// 反序列化其它资源类别：toml 读自 `.rev_launcher/resourcepacks`，
    /// 下载目标目录是项目的 `resourcepacks`。
    #[test]
    fn deserialize_resources_uses_kind_dirs_test() {
        let directory = tempfile::tempdir().unwrap();
        let project = test_project(directory.path());
        let metadata = directory.path().join(".rev_launcher/resourcepacks");
        std::fs::create_dir_all(&metadata).unwrap();
        let info = ModInfo {
            filename: "pack.zip".into(),
            sha1: String::new(),
            curseforge: None,
            modrinth: None,
        };
        std::fs::write(
            metadata.join("pack.zip.toml"),
            toml_edit::ser::to_string_pretty(&info).unwrap(),
        )
        .unwrap();

        let downloader =
            deserialize_resources(&project, ResourceKind::ResourcePack, 2).unwrap();
        assert_eq!(downloader.tasks().len(), 1);
        let task = &downloader.tasks()[&0];
        assert_eq!(task.filename(), "pack.zip");
        assert_eq!(task.path(), &directory.path().join("resourcepacks"));
        // 资源目录被自动创建出来。
        assert!(directory.path().join("resourcepacks").is_dir());
    }

    /// 序列化其它资源类别：扫描 `shaderpacks` 里的 zip，元数据写到
    /// `.rev_launcher/shaderpacks`。
    #[test]
    fn serialize_resources_shaderpacks_test() {
        let directory = tempfile::tempdir().unwrap();
        let project = test_project(directory.path());
        let shaders = directory.path().join("shaderpacks");
        std::fs::create_dir_all(&shaders).unwrap();
        std::fs::write(shaders.join("bsl.zip"), b"shader").unwrap();
        // jar 不应被 shaderpacks 扫描收进来。
        std::fs::write(shaders.join("mod.jar"), b"jar").unwrap();

        // 预写一份 sha1 匹配的 toml，让这次序列化跳过网络查询。
        let sha1 = crate::detective::modrinth::modrinth_sha1_bytes(b"shader");
        let info = ModInfo {
            filename: "bsl.zip".into(),
            sha1,
            curseforge: None,
            modrinth: None,
        };
        let out_dir = directory.path().join(".rev_launcher/shaderpacks");
        std::fs::create_dir_all(&out_dir).unwrap();
        std::fs::write(
            out_dir.join("bsl.zip.toml"),
            toml_edit::ser::to_string_pretty(&info).unwrap(),
        )
        .unwrap();

        let rx = serialize_resources(&project, ResourceKind::Shader).unwrap();
        let events: Vec<_> = rx.into_iter().collect();
        assert!(
            matches!(
                events.first(),
                Some(Ok(SerializeModsProgress::WriteStart { total: 1 }))
            ),
            "events: {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(Ok(SerializeModsProgress::Done { total: 1 }))
            ),
            "events: {events:?}"
        );
    }
}
