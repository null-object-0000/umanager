mod background;
mod clipboard_history;
mod dependency_check;
mod dev_cli_tools;
mod dev_tools;
mod feed;
mod icon;
mod installable;
mod installation;
mod launcher;
mod local_deb;
mod network;
mod operation_plan;
mod panel;
mod scanner;
mod session;
mod scripts;
mod source_engine;
mod translation;

use std::path::PathBuf;
use std::process::{Command, Stdio};
use tauri::{Emitter, Manager};
use umanager_catalog::Catalog;

/// Resolve an application by id for the generic store commands. In addition to
/// the managed application catalog, the `umanager` self-update source resolves
/// here so UManager's own update flows through the exact same download / verify /
/// plan / install commands as every other application (the plan itself is still
/// created with the dedicated `installSelfUpdate` action below).
fn require_application(catalog: &Catalog, application_id: &str) -> Result<umanager_catalog::Application, String> {
    if let Some(application) = catalog.by_application_id(application_id) {
        return Ok(application.clone());
    }
    if let Some(source) = catalog.self_update_source()
        && source.application_id == application_id
    {
        return Ok(source.to_application());
    }
    Err(format!("软件源中不存在应用 {application_id}"))
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

    // UManager itself is a managed package too: surface it in the store so its
    // self-update goes through the same "软件 / 更新" flow as every other app.
    if let Some(self_app) = catalog
        .self_update_source()
        .map(umanager_catalog::SelfUpdateSource::to_application)
    {
        let detected = tauri::async_runtime::spawn_blocking(installation::detect)
            .await
            .ok()
            .and_then(|outcome| outcome.ok());
        if let Some(info) = detected.filter(|info| info.can_self_remove) {
            if !result
                .packages
                .iter()
                .any(|item| item.package_name == self_app.package_name)
            {
                match source_engine::load_details(&self_app, &cache_dir).await {
                    Ok(details) => {
                        result.packages.push(scanner::ManagedPackage {
                            package_name: self_app.package_name.clone(),
                            display_name: self_app.display_name.clone(),
                            vendor: self_app.vendor.clone(),
                            installed_version: info.package_version.unwrap_or_default(),
                            candidate_version: details.candidate_version.clone(),
                            architecture: self_app.architecture.clone(),
                            source_kind: details.source_kind,
                            source_url: Some(details.source_url.clone()),
                            update_state: details.update_state,
                            homepage: self_app.homepage.clone(),
                        });
                        result
                            .packages
                            .sort_by(|left, right| left.display_name.cmp(&right.display_name));
                    }
                    Err(error) => result.warnings.push(format!("UManager 更新检查失败：{error}")),
                }
            }
        }
    }
    Ok(result)
}

#[tauri::command]
async fn get_software_catalog() -> Result<Vec<umanager_catalog::Application>, String> {
    let mut applications = feed::effective_applications().await?;
    // Include the self-update source so the store can resolve UManager's own
    // entry (source kind, accent color, description) exactly like other apps.
    let catalog = Catalog::load()?;
    if let Some(source) = catalog.self_update_source()
        && !applications
            .iter()
            .any(|application| application.application_id == source.application_id)
    {
        applications.push(source.to_application());
    }
    Ok(applications)
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
    source_engine::load_details(&application, &cache_dir).await
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
    source_engine::build_download_plan(&application, &cache_dir).await
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
    source_engine::download_and_verify(&application, cache_dir, progress).await
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
    if application.application_id == installation::APPLICATION_ID {
        return operation_plan::create_self_update_plan(&cache_dir).await;
    }
    operation_plan::create_install_plan(&catalog, &application, cache_dir).await
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
async fn get_llm_settings() -> Result<translation::LlmSettings, String> {
    Ok(translation::current())
}

#[tauri::command]
async fn set_llm_settings(
    app: tauri::AppHandle,
    settings: translation::LlmSettings,
) -> Result<translation::LlmSettings, String> {
    tauri::async_runtime::spawn_blocking(move || translation::update(&app, settings))
        .await
        .map_err(|error| format!("保存 LLM 设置任务异常结束：{error}"))?
}

#[tauri::command]
async fn translate_changelog(
    app: tauri::AppHandle,
    text: String,
    request_id: String,
) -> Result<String, String> {
    translation::translate_streaming(&app, &request_id, &text).await
}

#[tauri::command]
async fn test_llm_connection(settings: Option<translation::LlmSettings>) -> Result<String, String> {
    translation::test_connection(settings).await
}

#[tauri::command]
async fn get_feed_status() -> Result<feed::FeedStatus, String> {
    Ok(feed::status())
}

/// Per-source status for the v3 「软件源」 registry (design §6.3). Empty for a
/// v2 feed (no discoverable sources).
#[tauri::command]
async fn get_feed_source_statuses() -> Result<Vec<feed::FeedSourceStatus>, String> {
    Ok(feed::source_statuses())
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

/// Launch an installed managed desktop application. Runs as the current user
/// (no Polkit); the application id is resolved against the signed catalog so the
/// package name passed to `dpkg-query -L` is never free-form user input.
#[tauri::command]
async fn launch_application(application_id: String) -> Result<(), String> {
    let catalog = feed::effective_catalog().await?;
    let application = require_application(&catalog, &application_id)?;
    tauri::async_runtime::spawn_blocking(move || launcher::launch(&application))
        .await
        .map_err(|error| format!("启动任务异常结束：{error}"))?
}

/// Open an external changelog / release-page URL in the default browser. The
/// URL is re-validated (http/https only) inside `launcher::open_url`.
#[tauri::command]
async fn open_external_url(url: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || launcher::open_url(&url))
        .await
        .map_err(|error| format!("打开链接任务异常结束：{error}"))?
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
        if package_name == installation::PACKAGE_NAME {
            operation_plan::create_self_removal_plan(&cache_dir)
        } else {
            operation_plan::create_removal_plan(&catalog, &cache_dir, &package_name)
        }
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
            translation::initialize(app.handle());
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
            get_llm_settings,
            set_llm_settings,
            translate_changelog,
            test_llm_connection,
            get_feed_status,
            get_feed_source_statuses,
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
            launch_application,
            open_external_url,
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
            notify_download_complete,
            session::get_session_info,
            panel::hide_clipboard_panel,
            background::get_clipboard_hotkey,
            background::set_clipboard_hotkey,
            clipboard_history::list_clipboard_history,
            clipboard_history::clipboard_history_revision,
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
