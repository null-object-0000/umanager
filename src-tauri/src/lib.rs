mod dev_cli_tools;
mod dev_tools;
mod feed;
mod installable;
mod installation;
mod local_deb;
mod network;
mod operation_plan;
mod scanner;
mod source_engine;

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
    let catalog = Catalog::load()?;
    let mut result = tauri::async_runtime::spawn_blocking(scanner::scan)
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
    Ok(Catalog::load()?.applications)
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
    let catalog = Catalog::load()?;
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
    let catalog = Catalog::load()?;
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
    let catalog = Catalog::load()?;
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
    let catalog = Catalog::load()?;
    let application = require_application(&catalog, &application_id)?;
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    operation_plan::create_install_plan(application, cache_dir).await
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
}#[tauri::command]
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
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::create_removal_plan(&cache_dir, &package_name)
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(local_deb::LocalDebState::from_process_arguments())
        .setup(|app| {
            network::initialize(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_installation_info,
            get_network_settings,
            set_network_settings,
            get_feed_status,
            scan_packages,
            get_software_catalog,
            get_dev_toolchains,
            get_dev_toolchain_state,
            get_dev_releases,
            install_dev_version,
            set_dev_default_version,
            uninstall_dev_version,
            get_dev_tools,
            get_dev_tool_state,
            install_dev_tool,
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
            install_self_update
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UManager");
}
