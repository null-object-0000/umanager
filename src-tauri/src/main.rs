fn main() {
    if let Err(error) = umanager_lib::archive_launcher::refresh_installed() {
        eprintln!("Antigravity 启动项更新：{error}");
    }
    umanager_lib::run();
}
