fn main() {
    // Wayland 会话下桌面会忽略应用对窗口的定位请求，导致快捷面板只能被居中。
    // 这里在「Wayland + 可用的 XWayland」时强制 GTK 走 X11 后端：X11 应用可以自己
    // 摆放窗口，快捷面板才能贴到右上角托盘旁。纯 Wayland 无 XWayland 时保持默认，
    // 避免因无 X11 显示而启动失败。
    #[cfg(target_os = "linux")]
    {
        let on_wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
        let has_xwayland = std::env::var_os("DISPLAY").is_some();
        if on_wayland && has_xwayland {
            unsafe {
                // SAFETY: 在 main 启动最早期、尚未启动任何线程与 GTK 初始化前设置环境变量。
                std::env::set_var("GDK_BACKEND", "x11");
            }
        }
    }

    umanager_lib::run();
}
