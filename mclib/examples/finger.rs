use std::path::PathBuf;
use mclib::detective::curseforge::find_curse_info_with_rev_ua;
use rfd::FileDialog;
use mclib::detective::modrinth::{find_modrinth_info, find_modrinth_info_with_rev_ua};

fn main() -> Result<(), String> {
    let files = FileDialog::new()
        .add_filter("mods", &["jar"])
        .set_directory("/")
        .pick_files();

    let Some(path_bufs) = files else {
        return Err("No such file or directory".to_string());
    };

    let info = match find_curse_info_with_rev_ua(path_bufs.clone()) {
        Ok(info) => info,
        Err(error) => return Err(format!("指纹匹配失败：{error}")),
    };

    println!("{info:#?}");



    let info = match find_modrinth_info_with_rev_ua(<PathBuf as Clone>::clone(&path_bufs[0])) {
        Ok(info) => info,
        Err(error) => return Err(format!("指纹匹配失败：{error}")),
    };

    println!("{info:#?}");

    Ok(())
}
