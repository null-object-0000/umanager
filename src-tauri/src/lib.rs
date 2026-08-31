mod background;
mod clipboard_history;
mod dependency_check;
mod dev_cli_tools;
mod dev_tools;
mod feed;
mod icon;
mod installable;
mod installation;
mod local_deb;
mod network;
mod operation_plan;
mod panel;
mod scanner;
mod session;
mod scripts;
mod source_engine;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use tauri::{Emitter, Manager};
use umanager_catalog::Catalog;

fn require_application<'a>(
    catalog: &'a Catalog,
    application_id: &str,
) -> Result<&'a umanager_catalog::Application, String> {
    catalog
        .by_application_id(application_id)
        .ok_or_else(|| format!("软件源中不存在应用 {application_id}"))
}

#[tauri::command]
async fn scan_packages(app: tauri::AppHandle) -> Result<scanner::ScanResult, String> {
    let catalog = feed::effective_catalog().await?;
    let scan_catalog = catalog.clone();
    let mut result = tauri::async_runtime::spawn_blocking(move || scanner::scan(&scan_catalog))
        .await
        .map_err(|error| format!("扫描任务异常结束：{error}"))??;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;

    for application in catalog
        .applications
        .iter()
        .filter(|item| item.is_auto_installable())
    {
        let installed = result
            .packages
            .iter()
            .any(|item| item.package_name == application.package_name);
        if !installed {
            continue;
        }
        match source_engine::load_details(application, &cache_dir).await {
            Ok(details) => {
                if let Some(item) = result
                    .packages
                    .iter_mut()
                    .find(|item| item.package_name == application.package_name)
                {
                    item.candidate_version = details.candidate_version.clone();
                    item.update_state = details.update_state;
                    item.source_kind = details.source_kind;
                    item.source_url = Some(details.source_url.clone());
                }
            }
            Err(error) => result.warnings.push(format!(
                "{} 更新检查失败：{error}",
                application.display_name
            )),
        }
    }
    Ok(result)
}

#[tauri::command]
async fn get_software_catalog() -> Result<Vec<umanager_catalog::Application>, String> {
    feed::effective_applications().await
}

#[tauri::command]
async fn fetch_app_icon(
    app: tauri::AppHandle,
    app_id: String,
    icon_url: String,
    icon_sha256: String,
) -> Result<String, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let catalog = umanager_catalog::Catalog::load()?;
    let hosts = feed::config(&catalog)
        .map(|config| config.hosts.clone())
        .unwrap_or_default();
    icon::fetch_app_icon(&cache_dir, &app_id, &icon_url, &icon_sha256, &hosts).await
}

#[tauri::command]
async fn get_dev_toolchains() -> Result<Vec<umanager_catalog::DevelopmentToolchain>, String> {
    dev_tools::load_toolchains()
}

#[tauri::command]
async fn get_dev_toolchain_state(
    toolchain_id: String,
) -> Result<dev_tools::DevToolchainState, String> {
    dev_tools::detect_state(toolchain_id).await
}

#[tauri::command]
async fn get_dev_releases(toolchain_id: String) -> Result<Vec<dev_tools::DevRelease>, String> {
    dev_tools::list_remote_versions(toolchain_id).await
}

#[tauri::command]
async fn install_dev_version(
    app: tauri::AppHandle,
    toolchain_id: String,
    version: String,
) -> Result<dev_tools::DevOperationReport, String> {
    let event_app = app.clone();
    let progress: dev_tools::DevProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("dev-operation-progress", payload);
    });
    dev_tools::install_version(toolchain_id, version, progress).await
}

#[tauri::command]
async fn set_dev_default_version(
    app: tauri::AppHandle,
    toolchain_id: String,
    version: String,
) -> Result<dev_tools::DevOperationReport, String> {
    let event_app = app.clone();
    let progress: dev_tools::DevProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("dev-operation-progress", payload);
    });
    dev_tools::set_default_version(toolchain_id, version, progress).await
}

#[tauri::command]
async fn uninstall_dev_version(
    app: tauri::AppHandle,
    toolchain_id: String,
    version: String,
) -> Result<dev_tools::DevOperationReport, String> {
    let event_app = app.clone();
    let progress: dev_tools::DevProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("dev-operation-progress", payload);
    });
    dev_tools::uninstall_version(toolchain_id, version, progress).await
}

#[tauri::command]
async fn get_dev_tools() -> Result<Vec<umanager_catalog::DevelopmentTool>, String> {
    dev_cli_tools::load_tools()
}

#[tauri::command]
async fn get_dev_tool_state(tool_id: String) -> Result<dev_cli_tools::DevToolState, String> {
    dev_cli_tools::detect_state(tool_id).await
}

#[tauri::command]
async fn install_dev_tool(
    app: tauri::AppHandle,
    tool_id: String,
) -> Result<dev_cli_tools::DevToolReport, String> {
    let event_app = app.clone();
    let progress: dev_cli_tools::DevToolProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("dev-tool-progress", payload);
    });
    dev_cli_tools::install(tool_id, progress).await
}

#[tauri::command]
async fn update_dev_tool(
    app: tauri::AppHandle,
    tool_id: String,
) -> Result<dev_cli_tools::DevToolReport, String> {
    let event_app = app.clone();
    let progress: dev_cli_tools::DevToolProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("dev-tool-progress", payload);
    });
    dev_cli_tools::update(tool_id, progress).await
}

#[tauri::command]
async fn uninstall_dev_tool(
    app: tauri::AppHandle,
    tool_id: String,
) -> Result<dev_cli_tools::DevToolReport, String> {
    let event_app = app.clone();
    let progress: dev_cli_tools::DevToolProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("dev-tool-progress", payload);
    });
    dev_cli_tools::uninstall(tool_id, progress).await
}

#[tauri::command]
async fn get_application_details(
    app: tauri::AppHandle,
    application_id: String,
) -> Result<source_engine::ApplicationDetails, String> {
    let catalog = feed::effective_catalog().await?;
    let application = require_application(&catalog, &application_id)?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    source_engine::load_details(application, &cache_dir).await
}

#[tauri::command]
async fn get_download_plan(
    app: tauri::AppHandle,
    application_id: String,
) -> Result<source_engine::DownloadPlan, String> {
    let catalog = feed::effective_catalog().await?;
    let application = require_application(&catalog, &application_id)?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    source_engine::build_download_plan(application, &cache_dir).await
}

#[tauri::command]
async fn download_package(
    app: tauri::AppHandle,
    application_id: String,
) -> Result<source_engine::DownloadResult, String> {
    let catalog = feed::effective_catalog().await?;
    let application = require_application(&catalog, &application_id)?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let event_app = app.clone();
    let progress: source_engine::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("apt-download-progress", payload);
    });
    source_engine::download_and_verify(application, cache_dir, progress).await
}

#[tauri::command]
async fn create_operation_plan(
    app: tauri::AppHandle,
    application_id: String,
) -> Result<operation_plan::PlanArtifact, String> {
    let catalog = feed::effective_catalog().await?;
    let application = require_application(&catalog, &application_id)?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    operation_plan::create_install_plan(&catalog, application, cache_dir).await
}

#[tauri::command]
async fn run_operation_dry_run(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::run_install_dry_run(&cache_dir, &plan_id)
    })
    .await
    .map_err(|error| format!("Polkit dry-run 任务异常结束：{error}"))?
}

#[tauri::command]
async fn install_package(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let event_app = app.clone();
    let progress: operation_plan::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("operation-progress", payload);
    });
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::execute_install(&cache_dir, &plan_id, progress)
    })
    .await
    .map_err(|error| format!("安装任务异常结束：{error}"))?
}

#[tauri::command]
async fn get_installable_applications(
    app: tauri::AppHandle,
) -> Result<Vec<installable::InstallableApplication>, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    installable::load_applications(&cache_dir).await
}

#[tauri::command]
async fn get_network_settings() -> Result<network::NetworkSettings, String> {
    Ok(network::current())
}

#[tauri::command]
async fn get_feed_status() -> Result<feed::FeedStatus, String> {
    Ok(feed::status())
}

/// Force a metadata-feed refresh right now (used by the Settings "立即刷新"
/// button) and return the updated status. The fetch error itself is surfaced via
/// `FeedStatus.last_error`, so the caller always gets a coherent snapshot.
#[tauri::command]
async fn refresh_feed() -> Result<feed::FeedStatus, String> {
    let _ = feed::refresh_once(true).await;
    Ok(feed::status())
}

#[tauri::command]
async fn get_categories() -> Option<feed::CategoryCatalog> {
    feed::category_catalog().await
}

#[tauri::command]
async fn list_scripts() -> Vec<scripts::ScriptDefinition> {
    scripts::list()
}

#[tauri::command]
async fn run_script(
    app: tauri::AppHandle,
    script_id: String,
    action_id: String,
) -> Result<scripts::ScriptRunReport, String> {
    scripts::run(app, script_id, action_id).await
}

#[tauri::command]
async fn stop_script(script_id: String) -> bool {
    scripts::stop(script_id).await
}

#[tauri::command]
async fn set_network_settings(
    app: tauri::AppHandle,
    settings: network::NetworkSettings,
) -> Result<network::NetworkSettings, String> {
    tauri::async_runtime::spawn_blocking(move || network::update(&app, settings))
        .await
        .map_err(|error| format!("保存网络设置任务异常结束：{error}"))?
}

#[tauri::command]
async fn get_installation_info() -> Result<installation::InstallationInfo, String> {
    tauri::async_runtime::spawn_blocking(installation::detect)
        .await
        .map_err(|error| format!("安装形态检测任务异常结束：{error}"))?
}

/// Relaunch UManager and exit the current process. Used after a self-update,
/// where the on-disk binary has already been replaced by dpkg but the running
/// process still holds the old image.
///
/// Self-update is only possible for a `.deb` install, whose new binary dpkg has
/// just placed at `/usr/bin/umanager`. Launch that path directly when it exists
/// instead of resolving the current executable: after `dpkg --install` replaces a
/// running binary, `/proc/self/exe` points at the deleted inode, so any detection
/// based on it is inherently fragile. Fall back to the current executable only for
/// dev/portable launches (where the restart button cannot normally be reached).
#[tauri::command]
async fn restart_app(app: tauri::AppHandle) -> Result<(), String> {
    let installed = PathBuf::from("/usr/bin/umanager");
    let executable = if installed.exists() {
        installed
    } else {
        std::env::current_exe()
            .map_err(|error| format!("无法确定 UManager 可执行文件位置：{error}"))?
    };
    Command::new(&executable)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("无法启动新的 UManager：{error}"))?;
    app.exit(0);
    Ok(())
}

#[tauri::command]
async fn get_pending_local_deb(
    state: tauri::State<'_, local_deb::LocalDebState>,
) -> Result<Option<local_deb::LocalDebInspection>, String> {
    let Some(path) = state.pending_path()? else {
        return Ok(None);
    };
    tauri::async_runtime::spawn_blocking(move || {
        let temporary_state = local_deb::LocalDebState::from_path_for_command(path);
        local_deb::inspect_pending(&temporary_state)
    })
    .await
    .map_err(|error| format!("本地安装包检查任务异常结束：{error}"))?
}

#[tauri::command]
async fn import_pending_local_deb(
    app: tauri::AppHandle,
    state: tauri::State<'_, local_deb::LocalDebState>,
) -> Result<local_deb::LocalDebInspection, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let path = state
        .pending_path()?
        .ok_or_else(|| "UManager 不是通过本地 .deb 启动的".to_owned())?;
    tauri::async_runtime::spawn_blocking(move || {
        let temporary_state = local_deb::LocalDebState::from_path_for_command(path);
        local_deb::import_pending(&temporary_state, &cache_dir)
    })
    .await
    .map_err(|error| format!("本地安装包导入任务异常结束：{error}"))?
}

#[tauri::command]
async fn create_local_deb_operation_plan(
    app: tauri::AppHandle,
    sha256: String,
) -> Result<operation_plan::PlanArtifact, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || local_deb::create_plan(&cache_dir, &sha256))
        .await
        .map_err(|error| format!("本地安装包计划任务异常结束：{error}"))?
}

#[tauri::command]
async fn run_local_deb_dry_run(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::run_local_dry_run(&cache_dir, &plan_id)
    })
    .await
    .map_err(|error| format!("本地安装包 dry-run 任务异常结束：{error}"))?
}

#[tauri::command]
async fn install_local_deb(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let event_app = app.clone();
    let progress: operation_plan::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("operation-progress", payload);
    });
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::execute_local_install(&cache_dir, &plan_id, progress)
    })
    .await
    .map_err(|error| format!("本地安装包安装任务异常结束：{error}"))?
}

#[tauri::command]
async fn create_removal_operation_plan(
    app: tauri::AppHandle,
    package_name: String,
) -> Result<operation_plan::RemovalPlanArtifact, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let catalog = feed::effective_catalog().await?;
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::create_removal_plan(&catalog, &cache_dir, &package_name)
    })
    .await
    .map_err(|error| format!("卸载计划任务异常结束：{error}"))?
}

#[tauri::command]
async fn run_removal_dry_run(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::run_removal_dry_run(&cache_dir, &plan_id)
    })
    .await
    .map_err(|error| format!("卸载前特权复核任务异常结束：{error}"))?
}

#[tauri::command]
async fn remove_managed_package(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let event_app = app.clone();
    let progress: operation_plan::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("operation-progress", payload);
    });
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::execute_removal(&cache_dir, &plan_id, progress)
    })
    .await
    .map_err(|error| format!("卸载任务异常结束：{error}"))?
}

#[tauri::command]
async fn create_self_removal_operation_plan(
    app: tauri::AppHandle,
) -> Result<operation_plan::RemovalPlanArtifact, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::create_self_removal_plan(&cache_dir)
    })
    .await
    .map_err(|error| format!("UManager 卸载计划任务异常结束：{error}"))?
}

#[tauri::command]
async fn run_self_removal_dry_run(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::run_self_removal_dry_run(&cache_dir, &plan_id)
    })
    .await
    .map_err(|error| format!("UManager 卸载前复核任务异常结束：{error}"))?
}

#[tauri::command]
async fn remove_umanager(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let event_app = app.clone();
    let progress: operation_plan::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("operation-progress", payload);
    });
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::execute_self_removal(&cache_dir, &plan_id, progress)
    })
    .await
    .map_err(|error| format!("UManager 卸载任务异常结束：{error}"))?
}

fn self_update_application() -> Result<umanager_catalog::Application, String> {
    let catalog = Catalog::load()?;
    catalog
        .self_update_source()
        .map(umanager_catalog::SelfUpdateSource::to_application)
        .ok_or_else(|| "软件源未配置 UManager 自更新".to_owned())
}

#[tauri::command]
async fn get_self_update_status(
    app: tauri::AppHandle,
) -> Result<source_engine::ApplicationDetails, String> {
    let application = self_update_application()?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    // "检查更新" must reflect the live feed, not a stale-while-revalidate
    // cache copy that can sit up to 30 minutes behind a newly published release.
    // Force a refresh first; a failed fetch is non-fatal (the status and the
    // on-disk cache still serve a consistent snapshot).
    let _ = feed::refresh_once(true).await;
    source_engine::load_details(&application, &cache_dir).await
}

#[tauri::command]
async fn download_self_update(
    app: tauri::AppHandle,
) -> Result<source_engine::DownloadResult, String> {
    let application = self_update_application()?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let event_app = app.clone();
    let progress: source_engine::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("self-update-download-progress", payload);
    });
    source_engine::download_and_verify(&application, cache_dir, progress).await
}

#[tauri::command]
async fn create_self_update_operation_plan(
    app: tauri::AppHandle,
) -> Result<operation_plan::PlanArtifact, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    operation_plan::create_self_update_plan(&cache_dir).await
}

#[tauri::command]
async fn run_self_update_dry_run(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::run_self_update_dry_run(&cache_dir, &plan_id)
    })
    .await
    .map_err(|error| format!("UManager 自更新 dry-run 任务异常结束：{error}"))?
}

#[tauri::command]
async fn install_self_update(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    let event_app = app.clone();
    let progress: operation_plan::ProgressCallback = std::sync::Arc::new(move |payload| {
        let _ = event_app.emit("operation-progress", payload);
    });
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::execute_self_update(&cache_dir, &plan_id, progress)
    })
    .await
    .map_err(|error| format!("UManager 自更新任务异常结束：{error}"))?
}

#[tauri::command]
fn notify_download_complete(title: String, body: String) -> Result<(), String> {
    notify_rust::Notification::new()
        .appname("UManager")
        .summary(&title)
        .body(&body)
        .show()
        .map(|_| ())
        .map_err(|error| format!("无法发送系统通知：{error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.iter().any(|arg| arg == panel::TOGGLE_ARG) {
                panel::toggle(app);
            } else {
                // 用户点 dock/桌面图标启动第二个实例（普通启动）时，恢复并聚焦主窗口，
                // 否则「关闭即隐藏到托盘」后点图标会毫无反应。
                background::show_window(app);
            }
        }))
        .manage(local_deb::LocalDebState::from_process_arguments())
        .setup(|app| {
            network::initialize(app.handle());
            feed::initialize(app.handle());
            clipboard_history::initialize(app.handle());
            background::initialize(app.handle())?;
            if std::env::args().any(|arg| arg == panel::TOGGLE_ARG) {
                if let Some(main_window) = app.get_webview_window("main") {
                    let _ = main_window.hide();
                }
                panel::toggle(app.handle());
            }
            Ok(())
        })
        .on_window_event(background::handle_window_event)
        .invoke_handler(tauri::generate_handler![
            get_installation_info,
            restart_app,
            get_network_settings,
            set_network_settings,
            get_feed_status,
            refresh_feed,
            get_categories,
            list_scripts,
            run_script,
            stop_script,
            scan_packages,
            get_software_catalog,
            fetch_app_icon,
            get_dev_toolchains,
            get_dev_toolchain_state,
            get_dev_releases,
            install_dev_version,
            set_dev_default_version,
            uninstall_dev_version,
            get_dev_tools,
            get_dev_tool_state,
            install_dev_tool,
            update_dev_tool,
            uninstall_dev_tool,
            get_application_details,
            get_download_plan,
            download_package,
            create_operation_plan,
            run_operation_dry_run,
            install_package,
            get_installable_applications,
            get_pending_local_deb,
            import_pending_local_deb,
            create_local_deb_operation_plan,
            run_local_deb_dry_run,
            install_local_deb,
            create_removal_operation_plan,
            run_removal_dry_run,
            remove_managed_package,
            create_self_removal_operation_plan,
            run_self_removal_dry_run,
            remove_umanager,
            get_self_update_status,
            download_self_update,
            create_self_update_operation_plan,
            run_self_update_dry_run,
            install_self_update,
            notify_download_complete,
            session::get_session_info,
            panel::hide_clipboard_panel,
            background::get_clipboard_hotkey,
            background::set_clipboard_hotkey,
            clipboard_history::list_clipboard_history,
            clipboard_history::copy_clipboard_entry,
            clipboard_history::get_clipboard_image,
            clipboard_history::set_clipboard_entry_pinned,
            clipboard_history::delete_clipboard_entry,
            clipboard_history::clear_clipboard_history,
            clipboard_history::drag_clipboard_image
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UManager");
}
