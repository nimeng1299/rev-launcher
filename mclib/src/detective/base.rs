use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};

use crate::detective::curseforge::{find_curse_info_with_rev_ua, fingerprint};
use crate::detective::mod_info::{CurseforgeInfo, ModInfo, ModrinthInfo};
use crate::detective::modrinth::{find_modrinth_info_with_rev_ua, modrinth_sha1};
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
    use super::serialize_mods;
    use super::SerializeModsProgress;
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
}