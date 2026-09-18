use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use mclib::detective::base::{serialize_mods, SerializeModsProgress};
use mclib::project::game_project::get_game_project;
use rfd::FileDialog;

fn main() -> Result<(), String> {
    let Some(folder) = FileDialog::new()
        .set_title("选择一个project文件夹")
        .pick_folder()
    else {
        return Err("未选择文件夹".to_string());
    };

    let project =
        get_game_project(&folder).map_err(|e| format!("读取项目信息失败：{e}"))?;
    println!(
        "项目：{}（游戏版本 {}，加载器 {:?} {}）",
        project.name, project.game_version, project.loader, project.loader_version
    );

    let rx = serialize_mods(&project).map_err(|e| format!("启动序列化失败：{e}"))?;

    // 查询阶段长度未知，用 spinner；拿到 jar 总数后切换成进度条
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(100));
    pb.set_style(ProgressStyle::with_template("{spinner} {msg}").unwrap());

    for event in rx {
        match event {
            Ok(SerializeModsProgress::QueryCurseforge) => {
                pb.set_message("正在查询 CurseForge 指纹……");
            }
            Ok(SerializeModsProgress::QueryModrinth) => {
                pb.set_message("正在查询 Modrinth……");
            }
            Ok(SerializeModsProgress::WriteStart { total }) => {
                pb.set_style(
                    ProgressStyle::with_template(
                        "{spinner} [{elapsed_precise}] {bar:30.cyan/blue} {pos}/{len} {msg}",
                    )
                    .unwrap()
                    .progress_chars("=>-"),
                );
                pb.set_length(total as u64);
            }
            Ok(SerializeModsProgress::WriteDone { filename, .. }) => {
                pb.inc(1);
                pb.set_message(filename);
            }
            Ok(SerializeModsProgress::Done { total }) => {
                pb.finish_with_message(format!("完成，共 {total} 个 mod"));
            }
            Err(e) => {
                pb.finish_and_clear();
                return Err(format!("mod序列化失败：{e}"));
            }
        }
    }

    // 打印生成的 toml 文件内容
    let out_dir = project.path.join(".rev_launcher").join("mods");
    let mut files: Vec<_> = std::fs::read_dir(&out_dir)
        .map_err(|e| format!("读取输出目录失败：{e}"))?
        .flatten()
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort_by_key(|entry| entry.file_name());

    for file in files {
        let content =
            std::fs::read_to_string(file.path()).map_err(|e| format!("读取文件失败：{e}"))?;
        println!("--- {} ---\n{}", file.file_name().to_string_lossy(), content);
    }

    Ok(())
}
