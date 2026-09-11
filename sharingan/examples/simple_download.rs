use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use sharingan::downloader::DownloadBuilder;
use std::time::Duration;

fn simplify_size(size: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    if size == 0 {
        return "0.00 B".to_string();
    }

    let mut unit_idx = 0;
    let mut size_f = size as f64;

    while size_f >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size_f /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.2} {}", size_f, UNITS[unit_idx])
}

fn speed_size(size: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];

    if size == 0f64 {
        return "0B/s".to_string();
    }

    let mut unit_idx = 0;
    let mut size_f = size as f64;

    while size_f >= 1024.0 && unit_idx < UNITS.len() - 1 {
        size_f /= 1024.0;
        unit_idx += 1;
    }

    format!("{:.2} {}/s", size_f, UNITS[unit_idx])
}

fn main() {
    let thread_num = 4;
    let task_num = 10;

    let mut downloader = DownloadBuilder::default().thread_num(thread_num).build();

    let m = MultiProgress::new();
    let sty = ProgressStyle::with_template("[{elapsed_precise}] {bar:100.red/blue} {wide_msg}")
        .unwrap()
        .progress_chars("---");
    let n = 1000;
    let mut pb_vec = vec![];
    for _i in 0..task_num {
        let pb = m.add(ProgressBar::new(n));
        pb.set_style(sty.clone());
        pb.set_message("ready");
        pb_vec.push(pb);
    }
    m.println("starting!").unwrap();

    downloader.download(|builder| {
        builder
            .url("https://testfile.to/dl/10mb".to_string())
            .path(std::env::current_dir().unwrap().join("test"))
            .filename("test10mb.bin".to_string())
            .overwrite(true)
            .validator(|path|{
                if std::fs::metadata(&path).map_err(|e|e.to_string())?.len() > u64::MAX {
                    Ok(())
                }else {
                    Err(format!("download failed: {}", path.to_string_lossy()))
                }
            })
            .build()
    });

    for i in 0..task_num - 1 {
        let min_len = 0;
        downloader.download(move |builder| {
            builder
                .url("https://testfile.to/dl/1mb".to_string())
                .path(std::env::current_dir().unwrap().join("test"))
                .filename(format!("test{}.bin", i))
                .overwrite(true)
                .validator(move |path|{
                    if std::fs::metadata(&path).map_err(|e|e.to_string())?.len() > min_len {
                        Ok(())
                    }else {
                        Err(format!("download failed: {}", path.to_string_lossy()))
                    }
                })
                .build()
        });
    }

    loop {
        let map = downloader.tasks();
        for (id, task) in map.iter() {
            let pb = &pb_vec[id.clone()];
            let status = task.status();
            let p = task.progress();
            if !status.is_finished() {
                let show_total = if let Some(total) = p.total() {
                    pb.set_position(p.downloaded() * 100 / total);
                    total
                } else {
                    pb.enable_steady_tick(Duration::from_millis(100));
                    0
                };
                pb.set_message(format!(
                    "{} / {}\tspeed: {}\t{}",
                    simplify_size(p.downloaded()),
                    simplify_size(show_total),
                    speed_size(p.speed()),
                    task.filename()
                ));
            } else {
                pb.finish_with_message("done");
            }
        }
        for guard in downloader.finish_map().iter() {
            let task = guard.val();
            let _ = m.println(format!(
                "{} finish, code: {:?}, save path: {:?}",
                task.filename(),
                task.status(),
                task.path().join(task.filename())
            ));
            downloader.finish_map().remove(&task.id());
        }
        if downloader.is_finished() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    m.clear().unwrap();
}
