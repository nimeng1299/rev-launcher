use std::collections::{HashMap, HashSet};
use std::time::Duration;

use indicatif::{HumanBytes, MultiProgress, ProgressBar, ProgressStyle};
use mclib::detective::base::deserialize_mods;
use mclib::project::game_project::get_game_project;
use rfd::FileDialog;
use sharingan::status::DownloadStatus;

fn main() -> Result<(), String> {
    let Some(folder) = FileDialog::new()
        .set_title("选择一个project文件夹")
        .pick_folder()
    else {
        return Err("未选择文件夹".to_string());
    };
    let project = get_game_project(&folder).map_err(|e| format!("读取项目信息失败：{e}"))?;
    let downloader =
        deserialize_mods(&project, 10).map_err(|e| format!("启动反序列化失败：{e}"))?;
    let total = downloader.tasks().len();
    let mp = MultiProgress::new();
    let main_pb = mp.add(ProgressBar::new(total as u64));
    main_pb.set_style(
        ProgressStyle::with_template(
            "{spinner} [{elapsed_precise}] {bar:30.cyan/blue} {pos}/{len} {msg}",
        )
        .unwrap()
        .progress_chars("=>-"),
    );
    main_pb.enable_steady_tick(Duration::from_millis(100));
    let file_style = ProgressStyle::with_template(
        "  {msg} [{bar:25.cyan/blue}] {bytes}/{total_bytes} eta:{eta}",
    )
    .unwrap()
    .progress_chars("=>-");
    let waiting_style = ProgressStyle::with_template("  {spinner} {msg}").unwrap();
    let mut file_bars: HashMap<usize, ProgressBar> = HashMap::new();
    let mut reported_failures = HashSet::new();

    loop {
        let (mut downloaded, mut skipped, mut failed) = (0, 0, 0);
        for (&id, task) in downloader.tasks() {
            let status = task.status();
            if status.is_finished() {
                if let Some(pb) = file_bars.remove(&id) {
                    pb.finish_and_clear();
                }
                if status.is_failed() {
                    failed += 1;
                    if reported_failures.insert(id) {
                        main_pb.println(format!(
                            "下载失败：{}（{:?}）",
                            task.filename(),
                            task.failed_reason()
                        ));
                    }
                } else if task.options().skip_download {
                    skipped += 1;
                } else {
                    downloaded += 1;
                }
                continue;
            }
            if matches!(status, DownloadStatus::Ready) {
                continue;
            }
            let pb = file_bars.entry(id).or_insert_with(|| {
                let pb = mp.insert_after(&main_pb, ProgressBar::new_spinner());
                pb.enable_steady_tick(Duration::from_millis(100));
                pb
            });
            if matches!(status, DownloadStatus::Preparing) {
                pb.set_style(waiting_style.clone());
                pb.set_message(format!("{}：查询下载信息 / 检查本地文件", task.filename()));
            } else {
                let progress = task.progress();
                if let Some(size) = progress.total() {
                    pb.set_length(size);
                    pb.set_style(file_style.clone());
                } else {
                    pb.set_style(waiting_style.clone());
                }
                pb.set_position(progress.downloaded());
                pb.set_message(format!(
                    "{}（{}/s）",
                    task.filename(),
                    HumanBytes(progress.speed() as u64)
                ));
            }
        }
        let finished = downloaded + skipped + failed;
        main_pb.set_position(finished as u64);
        main_pb.set_message(format!("下载 {downloaded}，跳过 {skipped}，失败 {failed}"));
        // 使用本轮读到的终态数量，避免最后一个任务刚结束时漏报结果。
        if finished == total {
            main_pb.finish_with_message(format!(
                "完成：共 {total} 个，下载 {downloaded}，跳过 {skipped}，失败 {failed}",
            ));
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
