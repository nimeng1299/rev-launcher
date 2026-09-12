//! 只检查并展示已安装版本的启动命令，不启动游戏。
//! cargo run -p mclib --example launch_command -- <版本目录> <Java路径> <libraries目录> [--prepare-libraries]
use mclib::account::Account;
use mclib::java::java_version::JavaVersion;
use mclib::launch::Launcher;
use mclib::project::game_project::{GameProject, ModLoader};
use mclib::settings::GlobalSettings;
use std::path::PathBuf;

fn main() -> Result<(), String> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 && !(args.len() == 4 && args[3] == "--prepare-libraries") {
        return Err(
            "用法：launch_command <版本目录> <Java路径> <libraries目录> [--prepare-libraries]"
                .into(),
        );
    }
    let path = PathBuf::from(&args[0]);
    let java = JavaVersion::from_path(&PathBuf::from(&args[1])).map_err(|e| format!("{e:?}"))?;
    let name = path
        .file_name()
        .ok_or("无效的版本目录")?
        .to_string_lossy()
        .into_owned();
    let project = GameProject {
        name,
        version: String::new(),
        path,
        game_version: String::new(),
        loader: ModLoader::Minecraft,
        loader_version: String::new(),
    };
    let launcher = Launcher::new(
        Account::new_offline("Player"),
        project,
        vec![],
        GlobalSettings {
            java: Some(java),
            libraries_path: PathBuf::from(&args[2]),
            ..GlobalSettings::default()
        },
    );
    if args.len() == 4 {
        let downloader = launcher
            .download_libraries()
            .map_err(|e| format!("{e:?}"))?;
        while !downloader.is_finished() {
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if !downloader.is_all_success() {
            let failed: Vec<_> = downloader
                .tasks()
                .values()
                .filter(|task| !task.status().is_success())
                .map(|task| task.filename())
                .collect();
            return Err(format!("支持库下载失败：{failed:?}"));
        }
        println!("支持库准备完成（{} 个下载任务）", downloader.tasks().len());
    }
    let command = launcher.launch_command().map_err(|e| format!("{e:?}"))?;
    println!(
        "Java: {}\n工作目录: {}",
        command.java.display(),
        command.working_directory.display()
    );
    for argument in command.arguments {
        println!(
            "{:?}",
            argument.replace("00000000-0000-0000-0000-000000000000", "<redacted>")
        );
    }
    Ok(())
}
