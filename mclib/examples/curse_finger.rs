use mclib::detective::curseforge::find_curse_info_with_rev_ua;
use rfd::FileDialog;

fn main() -> Result<(), String> {
    let files = FileDialog::new()
        .add_filter("mods", &["jar"])
        .set_directory("/")
        .pick_files();

    let Some(path_bufs) = files else {
        return Err("No such file or directory".to_string());
    };

    let info = match find_curse_info_with_rev_ua(path_bufs) {
        Ok(info) => info,
        Err(error) => return Err(format!("指纹匹配失败：{error}")),
    };

    println!("{info:#?}");

    Ok(())
}
