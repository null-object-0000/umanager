//! X11/XWayland 下等待剪贴板变化，不在空闲时反复传输、解码同一张图片。
//! 初始化或连接失败时由调用方回退到轮询；不改变 arboard 的内容读取路径。

use x11rb::connection::Connection;
use x11rb::protocol::{
    Event,
    xfixes::{ConnectionExt as _, SelectionEventMask},
    xproto::{ConnectionExt as _, CreateWindowAux, WindowClass},
};
use x11rb::rust_connection::RustConnection;

pub struct ClipboardWatch {
    connection: RustConnection,
    selection: u32,
}

impl ClipboardWatch {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let (connection, screen) = x11rb::connect(None)?;
        let version = connection.xfixes_query_version(2, 0)?.reply()?;
        if version.major_version < 2 {
            return Err("XFixes selection events unavailable".into());
        }
        let window = connection.generate_id()?;
        connection
            .create_window(
                0,
                window,
                connection.setup().roots[screen].root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )?
            .check()?;
        let selection = connection.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        connection
            .xfixes_select_selection_input(
                window,
                selection,
                SelectionEventMask::SET_SELECTION_OWNER
                    | SelectionEventMask::SELECTION_WINDOW_DESTROY
                    | SelectionEventMask::SELECTION_CLIENT_CLOSE,
            )?
            .check()?;
        connection.flush()?;
        Ok(Self {
            connection,
            selection,
        })
    }

    pub fn wait(&self) -> Result<(), Box<dyn std::error::Error>> {
        loop {
            if matches!(self.connection.wait_for_event()?, Event::XfixesSelectionNotify(event)
                if event.selection == self.selection)
            {
                // 快速连续复制时只读取最终内容，避免依次请求已经过时的剪贴板。
                while self.connection.poll_for_event()?.is_some() {}
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    #[ignore = "requires an isolated X11 display: xvfb-run cargo test clipboard_watch -- --ignored"]
    fn detects_updates_from_the_same_owner_without_idle_wakeups() {
        let watcher = ClipboardWatch::new().unwrap();
        let (changed, received) = mpsc::channel();
        std::thread::spawn(move || {
            loop {
                watcher.wait().unwrap();
                if changed.send(()).is_err() {
                    break;
                }
            }
        });
        assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
        let (connection, screen) = x11rb::connect(None).unwrap();
        let owner = connection.generate_id().unwrap();
        connection
            .create_window(
                0,
                owner,
                connection.setup().roots[screen].root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )
            .unwrap()
            .check()
            .unwrap();
        let selection = connection
            .intern_atom(false, b"CLIPBOARD")
            .unwrap()
            .reply()
            .unwrap()
            .atom;
        for _ in 0..2 {
            connection
                .set_selection_owner(owner, selection, x11rb::CURRENT_TIME)
                .unwrap()
                .check()
                .unwrap();
            connection.flush().unwrap();
            received
                .recv_timeout(Duration::from_secs(2))
                .expect("selection change");
        }
        assert!(received.recv_timeout(Duration::from_millis(100)).is_err());
    }
}
