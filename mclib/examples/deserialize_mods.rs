use std::collections::HashMap;
use std::time::Duration;

use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressStyle};
use mclib::detective::base::{deserialize_mods, DeserializeModsProgress};
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

    let rx = deserialize_mods(&project, 10).map_err(|e| format!("启动反序列化失败：{e}"))?;

    let mp = MultiProgress::new();

    // 主进度条：按 mod 个数计数，每个 mod 产生一个终止事件
    let main_pb = mp.add(ProgressBar::new_spinner());
    main_pb.enable_steady_tick(Duration::from_millis(100));
    main_pb.set_style(ProgressStyle::with_template("{spinner} {msg}").unwrap());

    let main_style = ProgressStyle::with_template(
        "{spinner} [{elapsed_precise}] {bar:30.cyan/blue} {pos}/{len} {msg}",
    )
    .unwrap()
    .progress_chars("=>-");

    // 等到拿到文件大小后使用的文件进度条样式
    let file_style = ProgressStyle::with_template(
        "  {msg} [{bar:25.cyan/blue}] {bytes}/{total_bytes} eta:{eta}",
    )
    .unwrap()
    .progress_chars("=>-");
    // 还没拿到文件大小时的样式
    let file_waiting_style =
        ProgressStyle::with_template("  {spinner} {msg}").unwrap();

    // 每个下载中的文件对应一条进度条（首个 Downloading 事件时才创建，
    // 因此同时最多只有正在下载的文件有条数）
    let mut file_bars: HashMap<String, ProgressBar> = HashMap::new();
    let mut total = 0usize;
    let mut resolved = 0usize;

    for event in rx {
        match event {
            Ok(DeserializeModsProgress::Start { total: n }) => {
                total = n;
                main_pb.set_style(main_style.clone());
                main_pb.set_length(total as u64);
                main_pb.set_message("解析 mod 信息中……");
            }
            Ok(DeserializeModsProgress::Resolved { .. }) => {
                resolved += 1;
                main_pb.set_message(format!("解析完成 {resolved}/{total}"));
            }
            Ok(DeserializeModsProgress::DownloadStart { .. }) => {
                main_pb.set_message("下载中……");
            }
            Ok(DeserializeModsProgress::Downloading {
                filename,
                downloaded,
                total: file_total,
                speed,
            }) => {
                // 回退重试时复用已有进度条
                let pb = match file_bars.get(&filename) {
                    Some(pb) => pb.clone(),
                    None => {
                        let pb = mp.insert_after(&main_pb, ProgressBar::new_spinner());
                        pb.enable_steady_tick(Duration::from_millis(100));
                        pb.set_style(file_waiting_style.clone());
                        file_bars.insert(filename.clone(), pb.clone());
                        pb
                    }
                };
                if let Some(file_total) = file_total {
                    // 拿到文件大小后升级成带进度和 eta 的样式
                    if pb.length() != Some(file_total) {
                        pb.set_length(file_total);
                        pb.set_style(file_style.clone());
                    }
                    pb.set_position(downloaded);
                }
                pb.set_message(format!(
                    "{filename}（{}/s）",
                    HumanBytes(speed as u64)
                ));
            }
            Ok(DeserializeModsProgress::Downloaded { filename }) => {
                if let Some(pb) = file_bars.remove(&filename) {
                    pb.finish_and_clear();
                }
                main_pb.inc(1);
                main_pb.set_message(format!("已下载 {filename}"));
            }
            Ok(DeserializeModsProgress::Skipped { filename }) => {
                main_pb.inc(1);
                main_pb.set_message(format!("已存在，跳过 {filename}"));
            }
            Ok(DeserializeModsProgress::Failed { filename, message }) => {
                if let Some(pb) = file_bars.remove(&filename) {
                    pb.finish_and_clear();
                }
                main_pb.inc(1);
                main_pb.println(format!("下载失败：{filename}（{message}）"));
            }
            Ok(DeserializeModsProgress::Done {
                total,
                downloaded,
                skipped,
                failed,
            }) => {
                main_pb.finish_with_message(format!(
                    "完成：共 {total} 个，下载 {downloaded}，跳过 {skipped}，失败 {failed}"
                ));
            }
            Err(e) => {
                for pb in file_bars.values() {
                    pb.finish_and_clear();
                }
                main_pb.finish_and_clear();
                return Err(format!("反序列化失败：{e}"));
            }
        }
    }

    Ok(())
}
