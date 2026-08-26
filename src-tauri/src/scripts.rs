use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};
use tauri::{Emitter, Manager};

/// Built-in, user-level maintenance scripts. These are compiled into the app
/// (never served from the feed), because running a script means executing code
/// as the current user. They always run as the desktop user, never via root or
/// the Polkit helper, and each action carries a fixed argument list (never
/// shell-interpreted, never user-provided).

const MAX_LOG_LINE_CHARS: usize = 2_000;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptAction {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    /// Fixed actions the UI exposes; each maps to a fixed argv suffix.
    pub actions: Vec<ScriptAction>,
    /// Always true: scripts run as the current desktop user, never as root.
    pub user_level: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptProgressEvent {
    pub script_id: String,
    /// `running` for streamed output, `completed` for the final system message.
    pub phase: String,
    pub stream: String,
    pub message: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptRunReport {
    pub script_id: String,
    pub success: bool,
}

struct BuiltinAction {
    id: &'static str,
    label: &'static str,
    args: &'static [&'static str],
}

struct BuiltinScript {
    id: &'static str,
    name: &'static str,
    description: &'static str,
    user_level: bool,
    actions: Vec<BuiltinAction>,
    content: &'static str,
}

fn builtin_scripts() -> Vec<BuiltinScript> {
    vec![
        BuiltinScript {
            id: "chatgpt-logout-fix",
            name: "ChatGPT 登出修复",
            description: "修复 ChatGPT Desktop 退出登录后无法打开的问题：停止进程、备份并清除过期登录态，然后重新启动应用。",
            user_level: true,
            actions: vec![
                BuiltinAction { id: "run", label: "运行", args: &[] },
                BuiltinAction { id: "dry-run", label: "试运行", args: &["--dry-run"] },
            ],
            content: include_str!("../resources/scripts/chatgpt-logout-fix.sh"),
        },
        BuiltinScript {
            id: "fcitx5-ime-config",
            name: "Fcitx 5 输入法配置",
            description: "应用或恢复本机 Fcitx 5 中文输入法配置：环境变量、登录自启动、拼音输入、候选框主题与 Kimpanel 扩展。改动前会自动备份，可从备份恢复。",
            user_level: true,
            actions: vec![
                BuiltinAction { id: "apply", label: "应用配置", args: &["apply"] },
                BuiltinAction { id: "apply-dry-run", label: "应用（试运行）", args: &["apply", "--dry-run"] },
                BuiltinAction { id: "restore", label: "恢复最近备份", args: &["restore"] },
                BuiltinAction { id: "status", label: "查看状态", args: &["status"] },
            ],
            content: include_str!("../resources/scripts/fcitx5-ime-config.sh"),
        },
    ]
}

fn find_script(script_id: &str) -> Result<BuiltinScript, String> {
    builtin_scripts()
        .into_iter()
        .find(|script| script.id == script_id)
        .ok_or_else(|| format!("未找到内置脚本 {script_id}"))
}

fn running_registry() -> &'static Mutex<HashMap<String, Child>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Child>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn is_running(script_id: &str) -> bool {
    running_registry()
        .lock()
        .map(|guard| guard.contains_key(script_id))
        .unwrap_or(false)
}

pub fn list() -> Vec<ScriptDefinition> {
    builtin_scripts()
        .into_iter()
        .map(|script| ScriptDefinition {
            id: script.id.to_owned(),
            name: script.name.to_owned(),
            description: script.description.to_owned(),
            user_level: script.user_level,
            actions: script
                .actions
                .iter()
                .map(|action| ScriptAction {
                    id: action.id.to_owned(),
                    label: action.label.to_owned(),
                })
                .collect(),
        })
        .collect()
}

/// Run a built-in script action to completion, streaming stdout/stderr.
pub async fn run(
    app: tauri::AppHandle,
    script_id: String,
    action_id: String,
) -> Result<ScriptRunReport, String> {
    tauri::async_runtime::spawn_blocking(move || run_sync(&app, &script_id, &action_id))
        .await
        .map_err(|error| format!("脚本执行任务异常结束：{error}"))?
}

fn run_sync(app: &tauri::AppHandle, script_id: &str, action_id: &str) -> Result<ScriptRunReport, String> {
    let script = find_script(script_id)?;
    let action = script
        .actions
        .iter()
        .find(|action| action.id == action_id)
        .ok_or_else(|| format!("脚本 {script_id} 没有动作 {action_id}"))?;
    if is_running(script_id) {
        return Err("该脚本已在运行".to_owned());
    }

    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let scripts_dir = cache_dir.join("scripts");
    std::fs::create_dir_all(&scripts_dir)
        .map_err(|error| format!("无法创建脚本缓存目录：{error}"))?;
    let script_path = scripts_dir.join(format!("{script_id}.sh"));
    std::fs::write(&script_path, script.content)
        .map_err(|error| format!("无法写入脚本缓存：{error}"))?;
    set_private_permissions(&script_path)?;

    let mut command = Command::new("/bin/bash");
    command.arg(&script_path);
    for arg in action.args {
        command.arg(arg);
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Inherit the app's environment (HOME, PATH, DISPLAY, DBus) so the script
    // can stop/relaunch the desktop app / fcitx as the same user.

    let mut child = command
        .spawn()
        .map_err(|error| format!("无法启动脚本：{error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "无法读取脚本输出".to_owned())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "无法读取脚本错误输出".to_owned())?;

    {
        let mut registry = running_registry()
            .lock()
            .map_err(|_| "脚本运行状态锁失效".to_owned())?;
        registry.insert(script_id.to_owned(), child);
    }

    emit(app, script_id, "running", "system", &format!("开始执行「{}」", action.label));

    let stdout_id = script_id.to_owned();
    let stderr_id = script_id.to_owned();
    let stdout_emitter = app.clone();
    let stderr_emitter = app.clone();
    let stdout_thread = std::thread::spawn(move || forward_lines(stdout, "stdout", &stdout_emitter, &stdout_id));
    let stderr_thread = std::thread::spawn(move || forward_lines(stderr, "stderr", &stderr_emitter, &stderr_id));

    let owned_child = {
        let mut registry = running_registry()
            .lock()
            .map_err(|_| "脚本运行状态锁失效".to_owned())?;
        registry.remove(script_id)
    };

    let Some(mut child) = owned_child else {
        emit(app, script_id, "completed", "system", "脚本已停止");
        return Err("脚本已停止".to_owned());
    };

    let status = child
        .wait()
        .map_err(|error| format!("无法等待脚本结束：{error}"))?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let code = status.code().map(|value| value.to_string()).unwrap_or_else(|| "未知".to_owned());
    if status.success() {
        emit(app, script_id, "completed", "system", &format!("脚本运行完成（退出码 {code}）"));
    } else {
        emit(app, script_id, "completed", "system", &format!("脚本运行失败（退出码 {code}）"));
    }

    Ok(ScriptRunReport {
        script_id: script_id.to_owned(),
        success: status.success(),
    })
}

/// Stop a running script. Returns true when a running script was found and killed.
pub async fn stop(script_id: String) -> bool {
    tauri::async_runtime::spawn_blocking(move || stop_sync(&script_id))
        .await
        .unwrap_or(false)
}

fn stop_sync(script_id: &str) -> bool {
    let child = running_registry()
        .lock()
        .ok()
        .and_then(|mut guard| guard.remove(script_id));
    let Some(mut child) = child else {
        return false;
    };
    let _ = child.kill();
    let _ = child.wait();
    true
}

fn emit(app: &tauri::AppHandle, script_id: &str, phase: &str, stream: &str, message: &str) {
    let event = ScriptProgressEvent {
        script_id: script_id.to_owned(),
        phase: phase.to_owned(),
        stream: stream.to_owned(),
        message: message.to_owned(),
    };
    let _ = app.emit("script-progress", event);
}

fn forward_lines(reader: impl Read, stream: &'static str, app: &tauri::AppHandle, script_id: &str) {
    for line in BufReader::new(reader).split(b'\n') {
        let Ok(line) = line else { continue };
        let sanitized = sanitize_line(&String::from_utf8_lossy(&line));
        if sanitized.is_empty() {
            continue;
        }
        emit(app, script_id, "running", stream, &sanitized);
    }
}

fn sanitize_line(input: &str) -> String {
    let mut output = String::with_capacity(input.len().min(MAX_LOG_LINE_CHARS));
    for character in input.chars().take(MAX_LOG_LINE_CHARS) {
        if character == '\t' {
            output.push_str("    ");
        } else if !character.is_control() {
            output.push(character);
        }
    }
    output
}

fn set_private_permissions(path: &PathBuf) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("无法设置脚本缓存权限：{error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_scripts_are_registered_with_actions() {
        let scripts = builtin_scripts();
        assert_eq!(scripts.len(), 2);

        let chatgpt = scripts.iter().find(|script| script.id == "chatgpt-logout-fix").unwrap();
        assert_eq!(chatgpt.actions.len(), 2);
        assert!(chatgpt.content.contains("ChatGPT executable not found"));

        let fcitx = scripts.iter().find(|script| script.id == "fcitx5-ime-config").unwrap();
        assert_eq!(fcitx.actions.len(), 4);
        assert!(fcitx.content.contains("GTK_IM_MODULE=fcitx"));
        assert!(fcitx.actions.iter().any(|action| action.id == "apply"));
        assert!(fcitx.actions.iter().any(|action| action.id == "restore"));
    }

    #[test]
    fn list_returns_serializable_definitions() {
        let definitions = list();
        assert_eq!(definitions.len(), 2);
        assert!(definitions.iter().all(|definition| definition.user_level));
        let fcitx = definitions.iter().find(|definition| definition.id == "fcitx5-ime-config").unwrap();
        assert_eq!(fcitx.actions[0].id, "apply");
    }
}
