mod adapters;
mod local_deb;
mod operation_plan;
mod scanner;

use tauri::Manager;

#[tauri::command]
async fn scan_packages() -> Result<scanner::ScanResult, String> {
    tauri::async_runtime::spawn_blocking(scanner::scan)
        .await
        .map_err(|error| format!("扫描任务异常结束：{error}"))?
}

#[tauri::command]
async fn get_vscode_details() -> Result<adapters::vscode::VscodeDetails, String> {
    tauri::async_runtime::spawn_blocking(adapters::vscode::load_details)
        .await
        .map_err(|error| format!("VS Code 详情任务异常结束：{error}"))?
}

#[tauri::command]
async fn get_wechat_details(
    app: tauri::AppHandle,
) -> Result<adapters::wechat::WechatDetails, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    adapters::wechat::load_details(cache_dir).await
}

#[tauri::command]
async fn get_vscode_download_plan(
    app: tauri::AppHandle,
) -> Result<adapters::vscode::download::DownloadPlan, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || adapters::vscode::download::build_plan(&cache_dir))
        .await
        .map_err(|error| format!("VS Code 下载计划任务异常结束：{error}"))?
}

#[tauri::command]
async fn download_vscode_package(
    app: tauri::AppHandle,
) -> Result<adapters::vscode::download::DownloadResult, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    adapters::vscode::download::download_and_verify(cache_dir).await
}

#[tauri::command]
async fn create_vscode_operation_plan(
    app: tauri::AppHandle,
) -> Result<operation_plan::PlanArtifact, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    operation_plan::create_vscode_plan(cache_dir).await
}

#[tauri::command]
async fn run_vscode_operation_dry_run(
    app: tauri::AppHandle,
    plan_id: String,
) -> Result<serde_json::Value, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|error| format!("无法确定 UManager 缓存目录：{error}"))?;
    tauri::async_runtime::spawn_blocking(move || operation_plan::run_dry_run(&cache_dir, &plan_id))
        .await
        .map_err(|error| format!("Polkit dry-run 任务异常结束：{error}"))?
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
    tauri::async_runtime::spawn_blocking(move || {
        operation_plan::execute_local_install(&cache_dir, &plan_id)
    })
    .await
    .map_err(|error| format!("本地安装包安装任务异常结束：{error}"))?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(local_deb::LocalDebState::from_process_arguments())
        .invoke_handler(tauri::generate_handler![
            scan_packages,
            get_vscode_details,
            get_wechat_details,
            get_vscode_download_plan,
            download_vscode_package,
            create_vscode_operation_plan,
            run_vscode_operation_dry_run,
            get_pending_local_deb,
            import_pending_local_deb,
            create_local_deb_operation_plan,
            run_local_deb_dry_run,
            install_local_deb
        ])
        .run(tauri::generate_context!())
        .expect("failed to run UManager");
}
