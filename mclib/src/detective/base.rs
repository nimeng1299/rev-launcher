use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use sharingan::downloader::{DownloadBuilder, Downloader};
use sharingan::status::{DownloadFailure, DownloadStatus};
use sharingan::task::TaskBuilder;

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

fn detective_path(project: &GameProject) -> PathBuf{
    project.path.join(".rev_launcher")
}

/// [`serialize_mods`] 在工作线程中发出的进度事件。
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

/// 在独立线程中扫描游戏 mods 目录下的所有 jar，通过 CurseForge 指纹和
/// Modrinth sha1 批量查询 mod 信息，并把每个 jar 的 [`ModInfo`] 用 toml_edit
/// 序列化到 `<项目>/.rev_launcher/mods/<文件名>.toml`（文件名在原名后追加
/// `.toml`）。
///
/// 两个平台都没有匹配到的 mod 也会写入只含 filename 的 TOML 文件。
///
/// 返回接收进度的 channel，事件见 [`SerializeModsProgress`]；
/// 线程执行中出错时 channel 会先发出 `Err` 再关闭。
pub fn serialize_mods(
    project: &GameProject,
) -> Result<mpsc::Receiver<Result<SerializeModsProgress, Error>>, Error> {
    let out_dir = detective_path(project).join("mods");
    std::fs::create_dir_all(&out_dir)?;

    let mod_path = project.path.join("mods");
    let jars = list_jars(&mod_path)?;

    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let result = serialize_mods_in_thread(jars, out_dir, &tx);
        if let Err(e) = result {
            let _ = tx.send(Err(e));
        }
        // tx 在此 drop，接收端遍历随之结束
    });

    Ok(rx)
}

/// 线程内的实际序列化流程，成功时返回写入的文件数量
fn serialize_mods_in_thread(
    jars: Vec<PathBuf>,
    out_dir: PathBuf,
    tx: &Sender<Result<SerializeModsProgress, Error>>,
) -> Result<usize, Error> {
    // 接收端被丢弃时 send 会失败，忽略即可（toml 仍会继续写到磁盘）
    let send = |event: SerializeModsProgress| {
        let _ = tx.send(Ok(event));
    };

    if jars.is_empty() {
        send(SerializeModsProgress::WriteStart { total: 0 });
        send(SerializeModsProgress::Done { total: 0 });
        return Ok(0);
    }

    send(SerializeModsProgress::QueryCurseforge);
    let curse_matches = find_curse_info_with_rev_ua(jars.iter().collect::<Vec<_>>())?;
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
    let modrinth_versions = find_modrinth_info_with_rev_ua(jars.iter().collect::<Vec<_>>())?;

    let total = jars.len();
    send(SerializeModsProgress::WriteStart { total });

    for (index, jar) in jars.iter().enumerate() {
        let Some(filename) = jar.file_name().and_then(|name| name.to_str()) else {
            continue;
        };

        let curseforge = fingerprint(jar)
            .ok()
            .map(|fp| fp as i64)
            .and_then(|fp| curse_by_fingerprint.get(&fp).cloned());

        let modrinth = modrinth_sha1(jar)
            .ok()
            .and_then(|sha1| modrinth_versions.get(&sha1))
            .map(|version| ModrinthInfo {
                id: version.id.clone(),
            });

        let info = ModInfo {
            filename: filename.to_string(),
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

    send(SerializeModsProgress::Done { total });
    Ok(total)
}

// ---------------------------------------------------------------------------
// 反序列化：读取 toml 信息并下载 mod
// ---------------------------------------------------------------------------

/// [`deserialize_mods`] 在工作线程中发出的进度事件。
///
/// 通过返回的 channel 接收；channel 关闭表示整个流程结束。
/// 每个 mod 最终恰好产生一个 [`DeserializeModsProgress::Skipped`]、
/// [`DeserializeModsProgress::Downloaded`] 或 [`DeserializeModsProgress::Failed`]
/// 事件。
#[derive(Debug, Clone)]
pub enum DeserializeModsProgress {
    /// 开始处理，total 为待处理的 mod 数量（toml 文件数量）
    Start { total: usize },
    /// 单个 mod 在 mods 目录已存在且 sha1 一致，跳过下载
    Skipped { filename: String },
    /// 单个 mod 解析完成（拿到了下载信息），等待进入下载队列
    Resolved { filename: String },
    /// 单个 mod 开始下载
    DownloadStart { filename: String },
    /// 单个文件的实时下载进度（仅用于展示，每当已下载字节数变化时发出一次）
    Downloading {
        filename: String,
        downloaded: u64,
        /// 文件总大小，还没拿到 Content-Length 时为 None
        total: Option<u64>,
        /// 下载速度，单位为 字节/秒
        speed: f64,
    },
    /// 单个 mod 下载完成
    Downloaded { filename: String },
    /// 单个 mod 失败（两个渠道都不可用）
    Failed { filename: String, message: String },
    /// 全部完成
    Done {
        total: usize,
        downloaded: usize,
        skipped: usize,
        failed: usize,
    },
}

/// 反序列化：读取 `<项目>/.rev_launcher/mods/*.toml` 中保存的 [`ModInfo`]，
/// 在独立线程中把对应的 mod 下载到游戏的 mods 目录。
///
/// - 先用多线程并发解析所有 mod 的下载信息（优先 Modrinth，其次 CurseForge），
///   全部解析完成后才交给 sharingan 下载器并发下载；
/// - 目标文件已存在且 sha1 一致时跳过下载；
/// - 下载失败时换另一个渠道重试一次，下载完成后校验文件 sha1。
///
/// 返回接收进度的 channel，事件见 [`DeserializeModsProgress`]；
/// 线程执行中出错时 channel 会先发出 `Err` 再关闭。
pub fn deserialize_mods(
    project: &GameProject,
    threads: usize,
) -> Result<mpsc::Receiver<Result<DeserializeModsProgress, Error>>, Error> {
    let toml_dir = detective_path(project).join("mods");
    let mods = read_mod_infos(&toml_dir)?;

    let mods_dir = project.path.join("mods");
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let result = deserialize_mods_in_thread(mods, mods_dir, threads, &tx);
        if let Err(e) = result {
            let _ = tx.send(Err(e));
        }
    });

    Ok(rx)
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

/// 解析阶段单个 mod 的结果
#[derive(Clone)]
enum ResolvedMod {
    /// 已存在且哈希一致，跳过
    Skipped,
    /// 可以下载
    Ready(ResolvedDownload),
    /// 两个渠道都不可用
    Failed(String),
}

/// 解析阶段的并发线程数
const RESOLVE_THREADS: usize = 4;

/// 解析单个 mod：查询渠道下载信息，并检查文件是否已存在可跳过。
fn resolve_mod(mod_info: &ModInfo, mods_dir: &Path) -> ResolvedMod {
    match resolve_download_info(mod_info) {
        Err(message) => ResolvedMod::Failed(message),
        Ok(resolved) => {
            let target = mods_dir.join(&mod_info.filename);
            if target.is_file()
                && modrinth_sha1(&target).is_ok_and(|sha1| sha1 == resolved.sha1)
            {
                ResolvedMod::Skipped
            } else {
                ResolvedMod::Ready(resolved)
            }
        }
    }
}

/// 解析 mod 的下载地址与 sha1：优先 Modrinth，查询失败或信息不全时再用
/// CurseForge。
fn resolve_download_info(mod_info: &ModInfo) -> Result<ResolvedDownload, String> {
    let mut errors: Vec<String> = vec![];

    if let Some(mr) = &mod_info.modrinth {
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

    if let Some(cf) = &mod_info.curseforge {
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

/// 解析另一个渠道的下载信息（下载失败时的备选渠道）。
fn resolve_other_channel(mod_info: &ModInfo, used: DownloadChannel) -> Option<ResolvedDownload> {
    match used {
        DownloadChannel::Curseforge => mod_info.modrinth.as_ref().and_then(|mr| {
            find_modrinth_version_with_rev_ua(&mr.id)
                .ok()
                .and_then(|version| primary_version_file(&version))
                .map(|(url, sha1)| ResolvedDownload {
                    url,
                    sha1,
                    channel: DownloadChannel::Modrinth,
                })
        }),
        DownloadChannel::Modrinth => mod_info.curseforge.as_ref().and_then(|cf| {
            find_curse_file_info_with_rev_ua(cf.project_id, cf.file_id)
                .ok()
                .filter(|info| !info.download_url.is_empty())
                .and_then(|info| {
                    let sha1 = info.sha1().map(|sha1| sha1.to_string())?;
                    Some(ResolvedDownload {
                        url: info.download_url,
                        sha1,
                        channel: DownloadChannel::Curseforge,
                    })
                })
        }),
    }
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

/// 向下载器提交一个下载任务，完成后用 sha1 校验文件完整性。
fn submit_download(
    downloader: &mut Downloader,
    task_id: usize,
    mods_dir: &Path,
    resolved: &ResolvedDownload,
    filename: &str,
) {
    let expected_sha1 = resolved.sha1.clone();
    let task = TaskBuilder::new(task_id)
        .url(resolved.url.clone())
        .path(mods_dir.to_path_buf())
        .filename(filename.to_string())
        .overwrite(true)
        .validator(move |path| {
            if modrinth_sha1(&path).is_ok_and(|sha1| sha1 == expected_sha1) {
                Ok(())
            } else {
                Err(DownloadFailure::ValidationError)
            }
        })
        .build();
    downloader.add_task(task);
}

/// 线程内的实际下载流程
fn deserialize_mods_in_thread(
    mods: Vec<ModInfo>,
    mods_dir: PathBuf,
    threads: usize,
    tx: &Sender<Result<DeserializeModsProgress, Error>>,
) -> Result<usize, Error> {
    // 接收端被丢弃时 send 会失败，忽略即可（下载仍会继续）
    let send = |event: DeserializeModsProgress| {
        let _ = tx.send(Ok(event));
    };

    let total = mods.len();
    send(DeserializeModsProgress::Start { total });

    std::fs::create_dir_all(&mods_dir)?;

    // 任务 id -> mod 下标；mod 下标 -> 解析时使用的渠道
    let mut task_mods: HashMap<usize, usize> = HashMap::new();
    let mut channels: Vec<Option<DownloadChannel>> = vec![None; total];
    let mut next_task_id = 0usize;

    let (mut downloaded, mut skipped, mut failed) = (0usize, 0usize, 0usize);

    // 解析阶段：多线程并发查询每个 mod 的下载信息（含跳过检查），
    // 解析线程直接发出 Resolved / Skipped / Failed 事件
    let outcomes: Vec<ResolvedMod> = if mods.is_empty() {
        vec![]
    } else {
        std::thread::scope(|scope| {
            let resolve_threads = RESOLVE_THREADS.min(total);
            let chunk_size = total.div_ceil(resolve_threads);
            let mods_dir = &mods_dir;

            let mut handles = vec![];
            for (chunk_idx, chunk) in mods.chunks(chunk_size).enumerate() {
                let start = chunk_idx * chunk_size;
                let tx = tx.clone();
                handles.push((
                    start,
                    scope.spawn(move || {
                        let send = |event: DeserializeModsProgress| {
                            let _ = tx.send(Ok(event));
                        };
                        chunk
                            .iter()
                            .map(|mod_info| {
                                let outcome = resolve_mod(mod_info, &mods_dir);
                                match &outcome {
                                    ResolvedMod::Failed(message) => send(
                                        DeserializeModsProgress::Failed {
                                            filename: mod_info.filename.clone(),
                                            message: message.clone(),
                                        },
                                    ),
                                    ResolvedMod::Skipped => send(
                                        DeserializeModsProgress::Skipped {
                                            filename: mod_info.filename.clone(),
                                        },
                                    ),
                                    ResolvedMod::Ready(_) => send(
                                        DeserializeModsProgress::Resolved {
                                            filename: mod_info.filename.clone(),
                                        },
                                    ),
                                }
                                outcome
                            })
                            .collect::<Vec<_>>()
                    }),
                ));
            }

            let mut outcomes: Vec<Option<ResolvedMod>> = vec![None; total];
            for (start, handle) in handles {
                for (offset, outcome) in handle
                    .join()
                    .expect("解析线程不应 panic")
                    .into_iter()
                    .enumerate()
                {
                    outcomes[start + offset] = Some(outcome);
                }
            }
            outcomes.into_iter().map(Option::unwrap).collect()
        })
    };

    // 提交阶段：解析全部完成后，把可以下载的 mod 一次性交给下载器
    let mut downloader = DownloadBuilder::default().thread_num(threads).build();
    for (index, mod_info) in mods.iter().enumerate() {
        match &outcomes[index] {
            ResolvedMod::Failed(_) => failed += 1, // 事件已在解析线程发出
            ResolvedMod::Skipped => skipped += 1,
            ResolvedMod::Ready(resolved) => {
                channels[index] = Some(resolved.channel);
                send(DeserializeModsProgress::DownloadStart {
                    filename: mod_info.filename.clone(),
                });
                submit_download(
                    &mut downloader,
                    next_task_id,
                    &mods_dir,
                    resolved,
                    &mod_info.filename,
                );
                task_mods.insert(next_task_id, index);
                next_task_id += 1;
            }
        }
    }

    // 下载阶段：轮询任务状态，单个任务失败时换另一个渠道重试一次
    let mut handled: HashSet<usize> = HashSet::new();
    // 上次上报下载进度的字节数，用于去重
    let mut last_reported: HashMap<usize, u64> = HashMap::new();
    loop {
        // 先上报未完成任务的实时下载进度
        let progress_updates: Vec<DeserializeModsProgress> = downloader
            .tasks()
            .iter()
            .filter(|(id, task)| !handled.contains(id) && !task.status().is_finished())
            .filter_map(|(id, task)| {
                let downloaded = task.progress().downloaded();
                if last_reported.get(id) == Some(&downloaded) {
                    return None;
                }
                last_reported.insert(*id, downloaded);
                Some(DeserializeModsProgress::Downloading {
                    filename: mods[task_mods[id]].filename.clone(),
                    downloaded,
                    total: task.progress().total(),
                    speed: task.progress().speed(),
                })
            })
            .collect();
        for event in progress_updates {
            send(event);
        }

        let finished: Vec<(usize, DownloadStatus)> = downloader
            .tasks()
            .iter()
            .filter(|(id, task)| !handled.contains(id) && task.status().is_finished())
            .map(|(id, task)| (*id, task.status()))
            .collect();

        for (id, status) in finished {
            handled.insert(id);
            let index = task_mods[&id];
            let mod_info = &mods[index];
            let filename = mod_info.filename.clone();

            if status.is_success() {
                downloaded += 1;
                send(DeserializeModsProgress::Downloaded { filename });
                continue;
            }

            let used = channels[index].expect("提交过下载任务的 mod 一定记录了渠道");
            match resolve_other_channel(mod_info, used) {
                Some(resolved) => {
                    channels[index] = Some(resolved.channel);
                    send(DeserializeModsProgress::DownloadStart {
                        filename: filename.clone(),
                    });
                    submit_download(
                        &mut downloader,
                        next_task_id,
                        &mods_dir,
                        &resolved,
                        &filename,
                    );
                    task_mods.insert(next_task_id, index);
                    next_task_id += 1;
                }
                None => {
                    failed += 1;
                    send(DeserializeModsProgress::Failed {
                        filename,
                        message: "下载失败，且没有其他渠道可用".to_string(),
                    });
                }
            }
        }

        if downloader.tasks().keys().all(|id| handled.contains(id)) {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    send(DeserializeModsProgress::Done {
        total,
        downloaded,
        skipped,
        failed,
    });
    Ok(total)
}

/// 列出目录下所有 jar 文件（扩展名不区分大小写），按路径排序。
/// 目录不存在视为没有 jar。
fn list_jars(dir: &Path) -> Result<Vec<PathBuf>, Error> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
    };

    let mut jars: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("jar"))
        })
        .collect();
    jars.sort();
    Ok(jars)
}

#[cfg(test)]
mod tests {
    use super::list_jars;
    use super::read_mod_infos;
    use super::serialize_mods;
    use super::deserialize_mods;
    use super::SerializeModsProgress;
    use super::DeserializeModsProgress;
    use crate::detective::mod_info::{CurseforgeInfo, ModInfo, ModrinthInfo};
    use crate::project::game_project::{GameProject, ModLoader};

    #[test]
    fn list_jars_test() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jar"), b"jar").unwrap();
        std::fs::write(dir.path().join("b.JAR"), b"jar").unwrap();
        std::fs::write(dir.path().join("c.zip"), b"zip").unwrap();
        std::fs::write(dir.path().join("jar.txt"), b"text").unwrap();
        std::fs::create_dir(dir.path().join("d.jar")).unwrap();

        let jars = list_jars(dir.path()).unwrap();
        let names: Vec<String> = jars
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["a.jar", "b.JAR"]);
    }

    #[test]
    fn list_jars_missing_dir_test() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not_exist");
        assert!(list_jars(&missing).unwrap().is_empty());
    }

    #[test]
    fn mod_info_toml_format_test() {
        let info = ModInfo {
            filename: "sodium.jar".to_string(),
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
            curseforge: None,
            modrinth: None,
        };
        let toml = toml_edit::ser::to_string_pretty(&info).unwrap();
        assert_eq!(toml.trim(), "filename = \"unknown.jar\"");
    }

    /// 没有 jar 时线程只发 WriteStart{0} 和 Done{0}，不触发网络请求
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
            matches!(events.first(), Some(Ok(SerializeModsProgress::WriteStart { total: 0 }))),
            "events: {events:?}"
        );
        assert!(
            matches!(events.last(), Some(Ok(SerializeModsProgress::Done { total: 0 }))),
            "events: {events:?}"
        );
        assert_eq!(events.len(), 2);
    }

    /// serialize 写出的 toml 能被 read_mod_infos 读回
    #[test]
    fn read_mod_infos_round_trip_test() {
        let dir = tempfile::tempdir().unwrap();
        let full = ModInfo {
            filename: "sodium.jar".to_string(),
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

    /// 没有 toml 时线程只发 Start{0} 和 Done{0,0,0,0}，不触发网络请求
    #[test]
    fn deserialize_mods_progress_empty_test() {
        let dir = tempfile::tempdir().unwrap();
        let project = GameProject {
            name: "test".to_string(),
            version: "".to_string(),
            path: dir.path().to_path_buf(),
            game_version: "1.21".to_string(),
            loader: ModLoader::Minecraft,
            loader_version: "1.21".to_string(),
        };

        let rx = deserialize_mods(&project, 10).unwrap();
        let events: Vec<_> = rx.into_iter().collect();

        assert!(
            matches!(events.first(), Some(Ok(DeserializeModsProgress::Start { total: 0 }))),
            "events: {events:?}"
        );
        assert!(
            matches!(
                events.last(),
                Some(Ok(DeserializeModsProgress::Done {
                    total: 0,
                    downloaded: 0,
                    skipped: 0,
                    failed: 0
                }))
            ),
            "events: {events:?}"
        );
        assert_eq!(events.len(), 2);
    }
}
