//! Durable orchestration for managed .deb updates. History is never authorization:
//! approvals exist only in memory and bind the exact ordered immutable plans.
use crate::{
    download_queue::{self, DownloadJob, DownloadJobStatus, DownloadQueue},
    feed, operation_plan, scanner, source_engine,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
use umanager_plan::OperationPlan;

const EVENT: &str = "batch-tasks-changed";
const MAX_BATCHES: usize = 100;
const MAX_ITEMS: usize = 100;
const MAX_LOGS: usize = 120;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BatchStatus {
    Preparing,
    Downloading,
    AwaitingReview,
    Reviewing,
    AwaitingConfirmation,
    Installing,
    Completed,
    Paused,
    Interrupted,
    Canceled,
}
impl BatchStatus {
    fn active(self) -> bool {
        matches!(
            self,
            Self::Preparing
                | Self::Downloading
                | Self::AwaitingReview
                | Self::Reviewing
                | Self::AwaitingConfirmation
                | Self::Installing
        )
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemStatus {
    Downloading,
    Ready,
    Reviewed,
    Installing,
    Succeeded,
    Failed,
    Blocked,
    Unknown,
    Canceled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchItem {
    application_id: String,
    package_name: String,
    display_name: String,
    installed_version: String,
    target_version: String,
    architecture: String,
    sha256: String,
    size: u64,
    download_url: String,
    download_job_id: Option<String>,
    status: ItemStatus,
    plan_id: Option<String>,
    expires_at_unix_seconds: Option<u64>,
    error: Option<String>,
    logs: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchTask {
    id: String,
    created_at_unix_seconds: u64,
    updated_at_unix_seconds: u64,
    status: BatchStatus,
    items: Vec<BatchItem>,
    message: String,
    review_id: Option<String>,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    revision: u64,
    batches: Vec<BatchTask>,
    #[serde(default)]
    storage_error: Option<String>,
}
#[derive(Clone)]
struct Approval {
    batch_id: String,
    review_id: String,
    plans: Vec<OperationPlan>,
}
struct Store {
    snapshot: Snapshot,
    approval: Option<Approval>,
    load_error: Option<String>,
}
pub struct BatchTasks {
    store: Mutex<Store>,
    path: PathBuf,
    work: Arc<tokio::sync::Mutex<()>>,
    download_changed: tokio::sync::Notify,
}
impl BatchTasks {
    fn snapshot(&self) -> Result<Snapshot, String> {
        let store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(error) = &store.load_error {
            return Err(error.clone());
        }
        Ok(store.snapshot.clone())
    }
    /// Commit the durable transition before exposing it or performing an effect.
    fn change<T>(&self, f: impl FnOnce(&mut Snapshot) -> Result<T, String>) -> Result<T, String> {
        let mut store = self.store.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(error) = &store.load_error {
            return Err(error.clone());
        }
        let mut next = store.snapshot.clone();
        let result = f(&mut next)?;
        next.revision += 1;
        trim_history(&mut next)?;
        next.storage_error = None;
        if let Err(error) = save(&self.path, &next) {
            recover(&mut store.snapshot);
            store.approval = None;
            store.snapshot.storage_error = Some(error.clone());
            store.snapshot.revision += 1;
            return Err(error);
        }
        store.snapshot = next;
        Ok(result)
    }
    fn clear_approval(&self) {
        self.store
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .approval = None;
    }
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn trim_history(snapshot: &mut Snapshot) -> Result<(), String> {
    const MAX_BYTES: usize = 8 * 1024 * 1024;
    while serde_json::to_vec(snapshot)
        .map_err(|e| e.to_string())?
        .len()
        > MAX_BYTES
    {
        if snapshot.batches.len() > 1 && !snapshot.batches[0].status.active() {
            snapshot.batches.remove(0);
        } else {
            let mut reduced = false;
            for batch in &mut snapshot.batches {
                for item in &mut batch.items {
                    if item.logs.len() > 1 {
                        item.logs.drain(..item.logs.len() / 2);
                        reduced = true;
                    }
                }
            }
            if !reduced {
                return Err("任务记录过大，已停止后续操作".into());
            }
        }
    }
    Ok(())
}
fn save(path: &Path, snapshot: &Snapshot) -> Result<(), String> {
    let parent = path.parent().ok_or("任务记录目录无效")?;
    fs::create_dir_all(parent).map_err(|e| format!("无法创建任务记录目录：{e}"))?;
    let tmp = path.with_extension("tmp");
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)
            .map_err(|e| format!("无法写入任务记录：{e}"))?;
        file.write_all(&serde_json::to_vec(snapshot).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&tmp, path).map_err(|e| format!("无法保存任务记录：{e}"))?;
        fs::File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| format!("无法同步任务记录目录：{e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
fn recover(snapshot: &mut Snapshot) {
    for batch in &mut snapshot.batches {
        batch.review_id = None;
        if batch.status.active() {
            batch.status = BatchStatus::Interrupted;
            batch.message =
                "上次运行已中断；安装结果可能尚未记录。请重新检查未完成项，不会自动安装。".into();
            batch.updated_at_unix_seconds = now();
            for item in &mut batch.items {
                if matches!(item.status, ItemStatus::Installing | ItemStatus::Unknown) {
                    item.status = ItemStatus::Unknown;
                    item.error = Some("上次安装结果未能确认，请重新检查实际安装状态".into());
                } else if item.status != ItemStatus::Succeeded && item.status != ItemStatus::Failed
                {
                    item.status = ItemStatus::Blocked;
                    item.error = Some("需重新检查本机状态并生成新计划".into());
                }
            }
        }
    }
}
pub fn initialize(app: &AppHandle) -> Result<(), String> {
    let path = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("batch-tasks.json");
    // A crash may leave an unpublished temporary record. It never contains approval.
    let _ = fs::remove_file(path.with_extension("tmp"));
    let loaded = match fs::read(&path) {
        Ok(bytes) if bytes.len() <= 16 * 1024 * 1024 => serde_json::from_slice::<Snapshot>(&bytes)
            .map_err(|e| format!("任务记录损坏，已停止批量操作以保留原记录：{e}")),
        Ok(_) => Err("任务记录超过大小限制".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Snapshot::default()),
        Err(e) => Err(format!("无法读取任务记录：{e}")),
    };
    let (snapshot, load_error) = match loaded {
        Ok(mut snapshot) => {
            recover(&mut snapshot);
            snapshot.revision += 1;
            let error = save(&path, &snapshot).err();
            (snapshot, error)
        }
        Err(e) => (Snapshot::default(), Some(e)),
    };
    app.manage(BatchTasks {
        store: Mutex::new(Store {
            snapshot,
            approval: None,
            load_error,
        }),
        path,
        work: Arc::new(tokio::sync::Mutex::new(())),
        download_changed: tokio::sync::Notify::new(),
    });
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            let tasks = app.state::<BatchTasks>();
            // Create the notification future before inspecting state, matching
            // the download scheduler's no-lost-wakeup pattern. Idle means no timer.
            let notified = tasks.download_changed.notified();
            let mut downloading: Vec<_> = {
                let store = tasks.store.lock().unwrap_or_else(|p| p.into_inner());
                store
                    .snapshot
                    .batches
                    .iter()
                    .filter(|b| b.status == BatchStatus::Downloading)
                    .cloned()
                    .collect()
            };
            if !downloading.is_empty() {
                let jobs = app.state::<DownloadQueue>().snapshot();
                let mut changed = false;
                for batch in &mut downloading {
                    changed |= sync_downloads(batch, &jobs);
                }
                if changed {
                    let _ = tasks.change(|current| {
                        for batch in &mut current.batches {
                            if batch.status == BatchStatus::Downloading {
                                sync_downloads(batch, &jobs);
                            }
                        }
                        Ok(())
                    });
                    // Also expose persistence errors; no operation follows them.
                    publish(&app);
                }
            }
            notified.await;
        }
    });
    Ok(())
}
fn sync_downloads(batch: &mut BatchTask, jobs: &[DownloadJob]) -> bool {
    if batch.status != BatchStatus::Downloading {
        return false;
    }
    let mut changed = false;
    for item in &mut batch.items {
        if item.status != ItemStatus::Downloading {
            continue;
        }
        let job = jobs
            .iter()
            .find(|j| Some(&j.job_id) == item.download_job_id.as_ref());
        let error = match job {
            Some(job) => match job.status {
                DownloadJobStatus::Ready => {
                    if job.result.as_ref().is_some_and(|r| {
                        r.verified
                            && r.version == item.target_version
                            && r.actual_sha256 == item.sha256
                            && r.actual_size == item.size
                            && r.package_name == item.package_name
                            && r.architecture == item.architecture
                    }) {
                        item.status = ItemStatus::Ready;
                        changed = true;
                        None
                    } else {
                        Some("下载结果与本次选择的版本或摘要不一致，请重新检查".into())
                    }
                }
                DownloadJobStatus::Error => {
                    Some(job.error.clone().unwrap_or_else(|| "下载失败".into()))
                }
                DownloadJobStatus::Canceled => Some("下载已取消".into()),
                _ => None,
            },
            None => Some("对应下载任务已移除，请重新检查".into()),
        };
        if let Some(error) = error {
            item.status = ItemStatus::Failed;
            item.error = Some(error);
            changed = true;
        }
    }
    if !batch
        .items
        .iter()
        .any(|i| i.status == ItemStatus::Downloading)
    {
        if batch.items.iter().any(|i| i.status == ItemStatus::Ready) {
            batch.status = BatchStatus::AwaitingReview;
            batch.message = "下载已结束；只复核下载成功的项目，失败项可稍后重新检查。".into();
        } else {
            batch.status = BatchStatus::Paused;
            batch.message = "没有可复核的安装包，请重新检查失败项。".into();
        }
        changed = true;
    }
    if changed {
        batch.updated_at_unix_seconds = now();
    }
    changed
}
pub(crate) fn downloads_changed(app: &AppHandle) {
    if let Some(tasks) = app.try_state::<BatchTasks>() {
        tasks.download_changed.notify_one();
    }
}
struct PublishOnDrop<'a>(&'a AppHandle);
impl Drop for PublishOnDrop<'_> {
    fn drop(&mut self) {
        publish(self.0);
    }
}
fn publish(app: &AppHandle) {
    if let Ok(snapshot) = app.state::<BatchTasks>().snapshot() {
        let _ = app.emit(EVENT, snapshot);
    }
}
fn batch_mut<'a>(snapshot: &'a mut Snapshot, id: &str) -> Result<&'a mut BatchTask, String> {
    snapshot
        .batches
        .iter_mut()
        .find(|b| b.id == id)
        .ok_or_else(|| "批量任务不存在".into())
}
fn get_batch(app: &AppHandle, id: &str) -> Result<BatchTask, String> {
    app.state::<BatchTasks>()
        .snapshot()?
        .batches
        .into_iter()
        .find(|b| b.id == id)
        .ok_or_else(|| "批量任务不存在".into())
}
fn cache(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_cache_dir().map_err(|e| e.to_string())
}
fn log(item: &mut BatchItem, message: String) {
    // Bound UTF-8 by characters; logs are display-only and rendered as text.
    item.logs.push(message.chars().take(2000).collect());
    if item.logs.len() > MAX_LOGS {
        item.logs.remove(0);
    }
}
fn validate_selection(ids: &[String]) -> Result<(), String> {
    if ids.is_empty() || ids.len() > MAX_ITEMS {
        return Err(format!("请选择 1–{MAX_ITEMS} 个受管软件"));
    }
    let unique: HashSet<_> = ids.iter().collect();
    if unique.len() != ids.len() {
        return Err("批量清单包含重复软件".into());
    }
    Ok(())
}
#[tauri::command]
pub fn list_batch_tasks(tasks: tauri::State<'_, BatchTasks>) -> Result<Snapshot, String> {
    tasks.snapshot()
}

#[tauri::command]
pub async fn start_batch_update(
    app: AppHandle,
    application_ids: Vec<String>,
) -> Result<Snapshot, String> {
    let _publish_on_return = PublishOnDrop(&app);
    validate_selection(&application_ids)?;
    let tasks = app.state::<BatchTasks>();
    let _work = tasks
        .work
        .clone()
        .try_lock_owned()
        .map_err(|_| "批量操作正在进行，请稍候")?;
    if tasks.snapshot()?.batches.iter().any(|b| b.status.active()) {
        return Err("已有未完成的批量任务，请先处理或结束该任务".into());
    }
    let catalog = feed::effective_catalog().await?;
    let cache_dir = cache(&app)?;
    let mut items = Vec::new();
    let mut packages = HashSet::new();
    // Validate the complete selection before enqueuing anything. Frontend versions
    // are never accepted as installation facts.
    for id in application_ids {
        let application = catalog
            .by_application_id(&id)
            .filter(|a| {
                a.is_auto_installable() && a.application_id != crate::installation::APPLICATION_ID
            })
            .ok_or_else(|| format!("{id} 不支持批量更新；UManager 自更新需单独确认"))?;
        if !packages.insert(application.package_name.clone()) {
            return Err("不能重复更新同一软件包".into());
        }
        let details = source_engine::load_details(application, &cache_dir).await?;
        if !details.trusted
            || !matches!(details.update_state, scanner::UpdateState::UpdateAvailable)
        {
            return Err(format!(
                "{} 当前没有可信的可用更新，请刷新列表",
                application.display_name
            ));
        }
        let installed = details
            .installed_version
            .ok_or("批量更新只支持已安装的软件")?;
        let plan = source_engine::build_download_plan(application, &cache_dir).await?;
        items.push(BatchItem {
            application_id: id,
            package_name: application.package_name.clone(),
            display_name: application.display_name.clone(),
            installed_version: installed,
            target_version: plan.version,
            architecture: plan.architecture,
            sha256: plan.expected_sha256.ok_or("签名源缺少 SHA-256")?,
            size: plan.expected_size,
            download_url: plan.download_url,
            download_job_id: None,
            status: ItemStatus::Downloading,
            plan_id: None,
            expires_at_unix_seconds: None,
            error: None,
            logs: vec![],
        });
    }
    let id = format!(
        "batch-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    tasks.change(|snapshot| {
        while snapshot.batches.len() >= MAX_BATCHES {
            snapshot.batches.remove(0);
        }
        snapshot.batches.push(BatchTask {
            id: id.clone(),
            created_at_unix_seconds: now(),
            updated_at_unix_seconds: now(),
            status: BatchStatus::Preparing,
            items,
            message: "后台下载与校验中；完成后需要授权复核和再次确认。".into(),
            review_id: None,
        });
        Ok(())
    })?;
    publish(&app);
    // Existing downloads can be shared; their exact result is checked against
    // this batch's pinned version/hash before any plan can be authorized.
    let batch = get_batch(&app, &id)?;
    for (index, item) in batch.items.iter().enumerate() {
        if let Some(job) = app.state::<DownloadQueue>().snapshot().iter().find(|j| {
            j.application_id == item.application_id && j.status == DownloadJobStatus::Ready
        }) {
            if !job.result.as_ref().is_some_and(|r| {
                r.version == item.target_version
                    && r.actual_sha256 == item.sha256
                    && r.actual_size == item.size
            }) {
                download_queue::invalidate_ready(
                    &app,
                    &job.job_id,
                    "软件源已变化，重新下载本次选择的安装包".into(),
                );
            }
        }
        let outcome = download_queue::enqueue_download(
            app.clone(),
            item.application_id.clone(),
            Some(item.target_version.clone()),
        )
        .await;
        tasks.change(|snapshot| {
            let target = &mut batch_mut(snapshot, &id)?.items[index];
            match outcome {
                Ok(jobs) => {
                    target.download_job_id = jobs
                        .iter()
                        .find(|j| j.application_id == item.application_id)
                        .map(|j| j.job_id.clone());
                }
                Err(error) => {
                    target.status = ItemStatus::Failed;
                    target.error = Some(error);
                }
            }
            Ok(())
        })?;
    }
    tasks.change(|s| {
        batch_mut(s, &id)?.status = BatchStatus::Downloading;
        Ok(())
    })?;
    downloads_changed(&app);
    publish(&app);
    tasks.snapshot()
}

fn plan_matches(item: &BatchItem, plan: &OperationPlan) -> Result<(), String> {
    plan.verify_integrity()?;
    plan.validate_time(now())?;
    let p = &plan.payload;
    if p.action != umanager_plan::OperationAction::InstallVerifiedWebsiteDeb
        || p.application_id != item.application_id
        || p.package_name != item.package_name
        || p.installed_version.as_deref() != Some(&item.installed_version)
        || p.target_version != item.target_version
        || p.sha256 != item.sha256
        || p.size != item.size
        || p.architecture != item.architecture
    {
        return Err("软件、本机版本或签名源已变化；请重新检查并生成新任务".into());
    }
    Ok(())
}
fn review_digest(batch_id: &str, plans: &[OperationPlan]) -> String {
    let bytes = serde_json::to_vec(&(
        batch_id,
        plans.iter().map(|p| &p.plan_id).collect::<Vec<_>>(),
    ))
    .expect("serializable plan IDs");
    format!("{:x}", Sha256::digest(bytes))
}
fn verified_report(
    value: &serde_json::Value,
    plan: &OperationPlan,
    dry_run: bool,
) -> Result<(), String> {
    if value["planId"].as_str() != Some(&plan.plan_id)
        || value["verified"] != true
        || value["dryRun"] != dry_run
        || value["systemModified"] != !dry_run
    {
        return Err("helper 返回的复核或执行结果不匹配".into());
    }
    Ok(())
}
#[tauri::command]
pub async fn review_batch_update(app: AppHandle, batch_id: String) -> Result<Snapshot, String> {
    let _publish_on_return = PublishOnDrop(&app);
    let tasks = app.state::<BatchTasks>();
    let _work = tasks
        .work
        .clone()
        .try_lock_owned()
        .map_err(|_| "批量操作正在进行")?;
    let _mutation = operation_plan::MUTATION_LOCK
        .try_lock()
        .map_err(|_| "另一个软件安装或卸载正在执行")?;
    let batch = get_batch(&app, &batch_id)?;
    if !matches!(
        batch.status,
        BatchStatus::AwaitingReview | BatchStatus::AwaitingConfirmation
    ) {
        return Err("当前任务尚不能复核，请重新检查未完成项".into());
    }
    tasks.clear_approval();
    tasks.change(|s| {
        let b = batch_mut(s, &batch_id)?;
        b.status = BatchStatus::Reviewing;
        b.review_id = None;
        b.message = "正在锁定计划并逐项请求管理员复核；不会安装软件。".into();
        Ok(())
    })?;
    publish(&app);
    let result = prepare_review(&app, &batch_id).await;
    match result {
        Ok(plans) => {
            let review_id = review_digest(&batch_id, &plans);
            tasks.change(|s| {
                let b = batch_mut(s, &batch_id)?;
                b.status = BatchStatus::AwaitingConfirmation;
                b.review_id = Some(review_id.clone());
                b.updated_at_unix_seconds = now();
                b.message = "所有待安装项目已通过管理员复核；核对清单后再次确认执行。".into();
                Ok(())
            })?;
            tasks
                .store
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .approval = Some(Approval {
                batch_id,
                review_id,
                plans,
            });
        }
        Err(error) => {
            pause(&app, &batch_id, &error)?;
        }
    }
    publish(&app);
    tasks.snapshot()
}
async fn prepare_review(app: &AppHandle, batch_id: &str) -> Result<Vec<OperationPlan>, String> {
    let batch = get_batch(app, batch_id)?;
    let catalog = feed::effective_catalog().await?;
    let cache_dir = cache(app)?;
    let mut plans = Vec::new();
    for (index, item) in batch
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| matches!(i.status, ItemStatus::Ready | ItemStatus::Reviewed))
    {
        let result = async {
            let application = catalog
                .by_application_id(&item.application_id)
                .ok_or("软件已不在受管目录")?;
            let artifact =
                operation_plan::create_install_plan(&catalog, application, cache_dir.clone())
                    .await?;
            plan_matches(item, &artifact.plan)?;
            let deps = &artifact.missing_dependencies;
            if !deps.installable.is_empty() || !deps.unavailable.is_empty() {
                return Err(format!(
                    "依赖尚未满足，请先处理：{}",
                    deps.installable
                        .iter()
                        .chain(&deps.unavailable)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("；")
                ));
            }
            let dir = cache_dir.clone();
            let plan = artifact.plan.clone();
            let report = tauri::async_runtime::spawn_blocking(move || {
                operation_plan::run_install_dry_run(&dir, &plan.plan_id)
            })
            .await
            .map_err(|e| e.to_string())??;
            verified_report(&report, &artifact.plan, true)?;
            Ok::<_, String>(artifact.plan)
        }
        .await;
        match result {
            Ok(plan) => {
                app.state::<BatchTasks>().change(|s| {
                    let target = &mut batch_mut(s, batch_id)?.items[index];
                    target.status = ItemStatus::Reviewed;
                    target.plan_id = Some(plan.plan_id.clone());
                    target.expires_at_unix_seconds = Some(plan.payload.expires_at_unix_seconds);
                    target.error = None;
                    log(target, "管理员复核通过（未安装）".into());
                    Ok(())
                })?;
                plans.push(plan);
                publish(app);
            }
            Err(error) => {
                if error.starts_with(source_engine::CACHE_REVALIDATION_ERROR) {
                    if let Some(job_id) = &item.download_job_id {
                        download_queue::invalidate_ready(app, job_id, error.clone());
                    }
                }
                mark_failed(app, batch_id, index, &error)?;
                return Err(format!(
                    "{} 复核失败：{error}。本批次未执行安装。",
                    item.display_name
                ));
            }
        }
    }
    if plans.is_empty() {
        return Err("没有可复核的更新项".into());
    }
    let preflight_plans = plans.clone();
    tauri::async_runtime::spawn_blocking(move || {
        crate::dependency_check::batch_preflight(&preflight_plans)
    })
    .await
    .map_err(|e| e.to_string())??;
    for plan in &plans {
        plan.validate_time(now())?;
    }
    Ok(plans)
}
fn mark_failed(app: &AppHandle, id: &str, index: usize, error: &str) -> Result<(), String> {
    app.state::<BatchTasks>().change(|s| {
        let i = &mut batch_mut(s, id)?.items[index];
        i.status = ItemStatus::Failed;
        i.error = Some(error.into());
        log(i, error.into());
        Ok(())
    })
}
fn pause(app: &AppHandle, id: &str, error: &str) -> Result<(), String> {
    app.state::<BatchTasks>().clear_approval();
    app.state::<BatchTasks>().change(|s| {
        pause_batch(batch_mut(s, id)?, error);
        Ok(())
    })
}
fn pause_batch(batch: &mut BatchTask, error: &str) {
    batch.status = BatchStatus::Paused;
    batch.review_id = None;
    batch.message = error.into();
    batch.updated_at_unix_seconds = now();
    for item in &mut batch.items {
        if item.status == ItemStatus::Installing {
            item.status = ItemStatus::Unknown;
            item.error = Some("安装结果未能确认，请重新检查实际安装状态".into());
        } else if matches!(item.status, ItemStatus::Reviewed | ItemStatus::Ready) {
            item.status = ItemStatus::Blocked;
        }
    }
}
fn take_approval(
    store: &mut Store,
    batch: &BatchTask,
    review_id: &str,
) -> Result<Approval, String> {
    if batch.status != BatchStatus::AwaitingConfirmation
        || batch.review_id.as_deref() != Some(review_id)
    {
        return Err("复核清单已变化，请重新授权复核".into());
    }
    let approved = store
        .approval
        .as_ref()
        .filter(|a| a.batch_id == batch.id && a.review_id == review_id)
        .ok_or("复核已失效，请重新授权复核")?;
    if review_digest(&batch.id, &approved.plans) != review_id {
        return Err("复核摘要不一致".into());
    }
    let reviewed: Vec<_> = batch
        .items
        .iter()
        .filter(|i| i.status == ItemStatus::Reviewed)
        .collect();
    if reviewed.len() != approved.plans.len() || reviewed.is_empty() {
        return Err("复核项目不一致".into());
    }
    for (item, plan) in reviewed.into_iter().zip(&approved.plans) {
        plan_matches(item, plan)?;
        if item.plan_id.as_deref() != Some(&plan.plan_id) {
            return Err("计划已变化".into());
        }
    }
    Ok(store.approval.take().expect("approval checked"))
}
#[tauri::command]
pub async fn execute_batch_update(
    app: AppHandle,
    batch_id: String,
    review_id: String,
) -> Result<Snapshot, String> {
    let _publish_on_return = PublishOnDrop(&app);
    let tasks = app.state::<BatchTasks>();
    let work = tasks
        .work
        .clone()
        .try_lock_owned()
        .map_err(|_| "批量操作正在进行")?;
    let mutation = operation_plan::MUTATION_LOCK
        .try_lock()
        .map_err(|_| "另一个软件安装或卸载正在执行")?;
    let batch = get_batch(&app, &batch_id)?;
    let approval = {
        let mut store = tasks.store.lock().unwrap_or_else(|p| p.into_inner());
        take_approval(&mut store, &batch, &review_id)?
    };
    tasks.change(|s| {
        let b = batch_mut(s, &batch_id)?;
        b.status = BatchStatus::Installing;
        b.review_id = None;
        b.message = "正在逐项安装；失败时停止后续项目。请勿退出程序。".into();
        b.updated_at_unix_seconds = now();
        Ok(())
    })?;
    publish(&app);
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let (_work, _mutation) = (work, mutation);
        if let Err(error) = execute_approved(&handle, &approval).await {
            let message = format!("已停止后续安装：{error}。请重新检查本机状态后处理未完成项。");
            if let Err(storage_error) = pause(&handle, &approval.batch_id, &message) {
                // No further effects are allowed if the journal cannot be saved.
                // Show an explicit unknown/interrupted state even when disk is full.
                let tasks = handle.state::<BatchTasks>();
                let mut store = tasks.store.lock().unwrap_or_else(|p| p.into_inner());
                if let Ok(batch) = batch_mut(&mut store.snapshot, &approval.batch_id) {
                    pause_batch(batch, &message);
                    batch.status = BatchStatus::Interrupted;
                }
                store.snapshot.storage_error = Some(storage_error);
                store.snapshot.revision += 1;
            }
        }
        publish(&handle);
    });
    tasks.snapshot()
}
async fn run_reviewed_sequence<'a, T, Validate, Validation, Execute, Execution>(
    items: &'a [T],
    validate: Validate,
    mut execute: Execute,
) -> Result<(), String>
where
    Validate: FnOnce(&'a [T]) -> Validation,
    Validation: std::future::Future<Output = Result<(), String>>,
    Execute: FnMut(usize, &'a T) -> Execution,
    Execution: std::future::Future<Output = Result<(), String>>,
{
    validate(items).await?;
    for (index, item) in items.iter().enumerate() {
        execute(index, item).await?;
    }
    Ok(())
}
async fn execute_approved(app: &AppHandle, approval: &Approval) -> Result<(), String> {
    let cache_dir = cache(app)?;
    let catalog = feed::effective_catalog().await?;
    // Recheck the complete approved set before the first write, then independently
    // validate every plan again immediately before its helper execution.
    run_reviewed_sequence(
        &approval.plans,
        |_| async {
            let preflight_plans = approval.plans.clone();
            tauri::async_runtime::spawn_blocking(move || {
                crate::dependency_check::batch_preflight(&preflight_plans)
            })
            .await
            .map_err(|e| e.to_string())??;
            for plan in &approval.plans {
                plan.validate_time(now())?;
                let dir = cache_dir.clone();
                let plan_id = plan.plan_id.clone();
                let report = tauri::async_runtime::spawn_blocking(move || {
                    operation_plan::run_install_dry_run(&dir, &plan_id)
                })
                .await
                .map_err(|e| e.to_string())??;
                verified_report(&report, plan, true)?;
            }
            Ok(())
        },
        |offset, plan| {
            let cache_dir = &cache_dir;
            let catalog = &catalog;
            async move {
                let remaining = approval.plans[offset..].to_vec();
                tauri::async_runtime::spawn_blocking(move || {
                    crate::dependency_check::batch_preflight(&remaining)
                })
                .await
                .map_err(|e| e.to_string())??;
                let batch = get_batch(app, &approval.batch_id)?;
                let index = batch
                    .items
                    .iter()
                    .position(|i| i.plan_id.as_deref() == Some(&plan.plan_id))
                    .ok_or("任务计划不一致")?;
                plan_matches(&batch.items[index], plan)?;
                // A dependency updated by an earlier item can invalidate a later item.
                let path = PathBuf::from(&plan.payload.deb_path);
                let gap = tauri::async_runtime::spawn_blocking(move || {
                    crate::dependency_check::dependency_gap(&path)
                })
                .await
                .map_err(|e| e.to_string())?;
                if !gap.installable.is_empty() || !gap.unavailable.is_empty() {
                    mark_failed(
                        app,
                        &approval.batch_id,
                        index,
                        "依赖状态发生变化，已停止安装",
                    )?;
                    return Err("依赖状态发生变化".into());
                }
                app.state::<BatchTasks>().change(|s| {
                    let b = batch_mut(s, &approval.batch_id)?;
                    b.items[index].status = ItemStatus::Installing;
                    b.updated_at_unix_seconds = now();
                    Ok(())
                })?;
                publish(app);
                let handle = app.clone();
                let id = approval.batch_id.clone();
                let progress: operation_plan::ProgressCallback = Arc::new(move |event| {
                    // Logs are bounded and persisted with the result. Progress snapshots
                    // are emitted immediately, avoiding fsync for every output line.
                    {
                        let tasks = handle.state::<BatchTasks>();
                        let mut store = tasks.store.lock().unwrap_or_else(|p| p.into_inner());
                        if let Ok(b) = batch_mut(&mut store.snapshot, &id) {
                            log(&mut b.items[index], event.message.clone());
                            store.snapshot.revision += 1;
                        }
                    }
                    let _ = handle.emit("operation-progress", event);
                    publish(&handle);
                });
                let dir = cache_dir.clone();
                let plan_id = plan.plan_id.clone();
                let outcome = tauri::async_runtime::spawn_blocking(move || {
                    operation_plan::execute_install(&dir, &plan_id, progress)
                })
                .await
                .map_err(|e| e.to_string())
                .and_then(|r| r)
                .and_then(|report| verified_report(&report, plan, false));
                if let Err(error) = outcome {
                    mark_failed(app, &approval.batch_id, index, &error)?;
                    return Err(error);
                }
                // Verify real dpkg state; a successful helper invocation alone is not the
                // final task result (maintainer scripts may have changed package state).
                let catalog = catalog.clone();
                let scan = tauri::async_runtime::spawn_blocking(move || scanner::scan(&catalog))
                    .await
                    .map_err(|e| e.to_string())??;
                if !scan.packages.iter().any(|p| {
                    p.package_name == plan.payload.package_name
                        && p.installed_version == plan.payload.target_version
                        && p.architecture == plan.payload.architecture
                }) {
                    mark_failed(
                        app,
                        &approval.batch_id,
                        index,
                        "安装后未检测到完整安装的目标版本",
                    )?;
                    return Err("安装后状态核对失败".into());
                }
                app.state::<BatchTasks>().change(|s| {
                    let b = batch_mut(s, &approval.batch_id)?;
                    b.items[index].status = ItemStatus::Succeeded;
                    b.updated_at_unix_seconds = now();
                    log(
                        &mut b.items[index],
                        "目标版本安装成功并已核对本机状态".into(),
                    );
                    Ok(())
                })?;
                if let Some(job_id) = &batch.items[index].download_job_id {
                    download_queue::remove_download(app.clone(), job_id.clone());
                }
                publish(app);
                Ok(())
            }
        },
    )
    .await?;
    app.state::<BatchTasks>().change(|s| {
        let b = batch_mut(s, &approval.batch_id)?;
        let all = b.items.iter().all(|i| i.status == ItemStatus::Succeeded);
        b.status = if all {
            BatchStatus::Completed
        } else {
            BatchStatus::Paused
        };
        b.message = if all {
            "本批次所有更新均已完成。"
        } else {
            "可安装项目已完成；其余项目需要重新检查。"
        }
        .into();
        b.updated_at_unix_seconds = now();
        Ok(())
    })
}
#[tauri::command]
pub async fn cancel_batch_update(app: AppHandle, batch_id: String) -> Result<Snapshot, String> {
    let _publish_on_return = PublishOnDrop(&app);
    let tasks = app.state::<BatchTasks>();
    let _work = tasks
        .work
        .clone()
        .try_lock_owned()
        .map_err(|_| "正在复核或安装，无法结束；请等待操作完成")?;
    tasks.change(|s| {
        let b = batch_mut(s, &batch_id)?;
        if !matches!(
            b.status,
            BatchStatus::Preparing
                | BatchStatus::Downloading
                | BatchStatus::AwaitingReview
                | BatchStatus::AwaitingConfirmation
                | BatchStatus::Paused
                | BatchStatus::Interrupted
        ) {
            return Err("当前任务不能结束".into());
        }
        let was_active = b.status.active();
        b.status = BatchStatus::Canceled;
        b.review_id = None;
        b.updated_at_unix_seconds = now();
        if was_active {
            b.message = "批量任务已结束；共享的下载任务与缓存仍保留，可在下载页管理。".into();
            for i in &mut b.items {
                if i.status != ItemStatus::Succeeded && i.status != ItemStatus::Failed {
                    i.status = ItemStatus::Canceled;
                }
            }
        } else {
            b.message = "已结束本批次跟进；原有结果与日志保留。".into();
        }
        Ok(())
    })?;
    {
        let mut store = tasks.store.lock().unwrap_or_else(|p| p.into_inner());
        if store
            .approval
            .as_ref()
            .is_some_and(|a| a.batch_id == batch_id)
        {
            store.approval = None;
        }
    }
    publish(&app);
    tasks.snapshot()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use umanager_plan::{OperationAction, PLAN_SCHEMA_VERSION, PlanPayload};

    fn item(id: &str) -> BatchItem {
        BatchItem {
            application_id: id.into(),
            package_name: id.into(),
            display_name: id.into(),
            installed_version: "1.0".into(),
            target_version: "2.0".into(),
            architecture: "amd64".into(),
            sha256: "a".repeat(64),
            size: 100,
            download_url: "https://example.org/app.deb".into(),
            download_job_id: Some(format!("job-{id}")),
            status: ItemStatus::Reviewed,
            plan_id: None,
            expires_at_unix_seconds: None,
            error: None,
            logs: vec![],
        }
    }
    fn plan(item: &BatchItem) -> OperationPlan {
        OperationPlan::new(PlanPayload {
            schema_version: PLAN_SCHEMA_VERSION,
            action: OperationAction::InstallVerifiedWebsiteDeb,
            application_id: item.application_id.clone(),
            package_name: item.package_name.clone(),
            installed_version: Some(item.installed_version.clone()),
            target_version: item.target_version.clone(),
            architecture: item.architecture.clone(),
            deb_path: "/home/test/.cache/downloads/app.deb".into(),
            sha256: item.sha256.clone(),
            size: item.size,
            created_at_unix_seconds: now() - 1,
            expires_at_unix_seconds: now() + 800,
            catalog_json: None,
            catalog_signature: None,
            source_ref: None,
            source_endorsement: None,
            source_catalog_json: None,
            source_catalog_signature: None,
        })
        .unwrap()
    }
    fn approved() -> (BatchTask, Approval) {
        let mut items = vec![item("first"), item("second")];
        let plans: Vec<_> = items.iter().map(plan).collect();
        for (item, plan) in items.iter_mut().zip(&plans) {
            item.plan_id = Some(plan.plan_id.clone());
            item.expires_at_unix_seconds = Some(plan.payload.expires_at_unix_seconds);
        }
        let digest = review_digest("batch", &plans);
        (
            BatchTask {
                id: "batch".into(),
                created_at_unix_seconds: now(),
                updated_at_unix_seconds: now(),
                status: BatchStatus::AwaitingConfirmation,
                items,
                message: String::new(),
                review_id: Some(digest.clone()),
            },
            Approval {
                batch_id: "batch".into(),
                review_id: digest,
                plans,
            },
        )
    }
    fn store(approval: Option<Approval>) -> Store {
        Store {
            snapshot: Snapshot::default(),
            approval,
            load_error: None,
        }
    }
    fn job(item: &BatchItem) -> DownloadJob {
        DownloadJob {
            job_id: item.download_job_id.clone().unwrap(),
            application_id: item.application_id.clone(),
            package_name: item.package_name.clone(),
            display_name: item.display_name.clone(),
            version: Some(item.target_version.clone()),
            status: DownloadJobStatus::Ready,
            progress: None,
            error: None,
            result: Some(source_engine::DownloadResult {
                plan: source_engine::DownloadPlan {
                    application_id: item.application_id.clone(),
                    package_name: item.package_name.clone(),
                    version: item.target_version.clone(),
                    architecture: item.architecture.clone(),
                    source_kind: scanner::SourceKind::OfficialWebsite,
                    repository_url: None,
                    download_url: item.download_url.clone(),
                    file_name: "app.deb".into(),
                    expected_size: item.size,
                    expected_sha256: Some(item.sha256.clone()),
                    target_path: "/tmp/app.deb".into(),
                    release_tag: None,
                    asset_name: None,
                    website_version: None,
                },
                actual_size: item.size,
                actual_sha256: item.sha256.clone(),
                package_name: item.package_name.clone(),
                version: item.target_version.clone(),
                architecture: item.architecture.clone(),
                reused_existing_file: false,
                verified: true,
            }),
            enqueued_at_unix_seconds: now(),
            finished_at_unix_seconds: Some(now()),
        }
    }
    fn download_batch() -> BatchTask {
        let (mut batch, _) = approved();
        batch.status = BatchStatus::Downloading;
        batch.review_id = None;
        for i in &mut batch.items {
            i.status = ItemStatus::Downloading;
        }
        batch
    }
    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "umanager-batch-test-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn empty_duplicate_and_oversized_selections_are_rejected() {
        assert!(validate_selection(&[]).is_err());
        assert!(validate_selection(&["a".into(), "a".into()]).is_err());
        assert!(validate_selection(&(0..101).map(|i| i.to_string()).collect::<Vec<_>>()).is_err());
        assert!(validate_selection(&["a".into(), "b".into()]).is_ok());
    }
    #[test]
    fn download_results_are_bound_to_job_version_hash_size_architecture_and_identity() {
        for mismatch in 0..8 {
            let mut batch = download_batch();
            let mut jobs: Vec<_> = batch.items.iter().map(job).collect();
            let r = jobs[0].result.as_mut().unwrap();
            match mismatch {
                0 => r.version = "3.0".into(),
                1 => r.actual_sha256 = "b".repeat(64),
                2 => r.actual_size += 1,
                3 => r.architecture = "arm64".into(),
                4 => r.package_name = "other".into(),
                5 => r.verified = false,
                6 => jobs[0].job_id = "different-job".into(),
                _ => {}
            }
            sync_downloads(&mut batch, &jobs);
            assert_eq!(
                batch.items[0].status,
                if mismatch == 7 {
                    ItemStatus::Ready
                } else {
                    ItemStatus::Failed
                }
            );
            assert_eq!(batch.status, BatchStatus::AwaitingReview);
            assert!(batch.review_id.is_none());
        }
    }
    #[test]
    fn no_review_is_possible_while_any_download_is_running() {
        let mut batch = download_batch();
        let mut jobs: Vec<_> = batch.items.iter().map(job).collect();
        jobs[1].status = DownloadJobStatus::Downloading;
        sync_downloads(&mut batch, &jobs);
        assert_eq!(batch.status, BatchStatus::Downloading);
        assert_eq!(batch.items[0].status, ItemStatus::Ready);
        jobs[1].status = DownloadJobStatus::Error;
        jobs[1].error = Some("offline".into());
        sync_downloads(&mut batch, &jobs);
        assert_eq!(batch.status, BatchStatus::AwaitingReview);
        assert_eq!(batch.items[1].error.as_deref(), Some("offline"));
    }
    #[test]
    fn all_failed_or_canceled_downloads_pause_without_authorization() {
        let mut batch = download_batch();
        let mut jobs: Vec<_> = batch.items.iter().map(job).collect();
        for job in &mut jobs {
            job.status = DownloadJobStatus::Canceled;
        }
        sync_downloads(&mut batch, &jobs);
        assert_eq!(batch.status, BatchStatus::Paused);
        assert!(batch.items.iter().all(|i| i.status == ItemStatus::Failed));
    }
    #[test]
    fn canceled_batch_cannot_be_revived_by_download_completion() {
        let mut batch = download_batch();
        batch.status = BatchStatus::Canceled;
        let jobs: Vec<_> = batch.items.iter().map(job).collect();
        sync_downloads(&mut batch, &jobs);
        assert_eq!(batch.status, BatchStatus::Canceled);
    }
    #[test]
    fn approval_is_memory_only_exact_and_single_use() {
        let (batch, approval) = approved();
        let mut state = store(Some(approval));
        let digest = batch.review_id.as_ref().unwrap();
        assert!(take_approval(&mut state, &batch, "wrong-digest").is_err());
        assert!(state.approval.is_some());
        assert!(take_approval(&mut state, &batch, digest).is_ok());
        assert!(take_approval(&mut state, &batch, digest).is_err());
        assert!(take_approval(&mut store(None), &batch, digest).is_err());
    }
    #[test]
    fn reordered_added_removed_or_changed_plans_cannot_reuse_confirmation() {
        for mutation in 0..6 {
            let (mut batch, mut approval) = approved();
            let digest = batch.review_id.clone().unwrap();
            match mutation {
                0 => approval.plans.reverse(),
                1 => {
                    approval.plans.pop();
                }
                2 => batch.items[0].target_version = "3.0".into(),
                3 => batch.items[0].sha256 = "c".repeat(64),
                4 => batch.items[0].plan_id = Some("d".repeat(64)),
                _ => batch.items[0].status = ItemStatus::Failed,
            }
            assert!(take_approval(&mut store(Some(approval)), &batch, &digest).is_err());
        }
    }
    #[test]
    fn expired_or_tampered_plan_requires_new_review() {
        let (batch, mut approval) = approved();
        let digest = batch.review_id.clone().unwrap();
        approval.plans[0].payload.expires_at_unix_seconds = now() - 1;
        assert!(take_approval(&mut store(Some(approval)), &batch, &digest).is_err());
        let mut target = item("a");
        let mut expired = plan(&target);
        expired.payload.created_at_unix_seconds = now() - 1000;
        expired.payload.expires_at_unix_seconds = now() - 100;
        expired = OperationPlan::new(expired.payload).unwrap();
        target.plan_id = Some(expired.plan_id.clone());
        assert!(plan_matches(&target, &expired).is_err());
    }
    #[test]
    fn changing_installed_version_or_action_invalidates_a_plan() {
        let mut item = item("a");
        let original = plan(&item);
        item.installed_version = "1.1".into();
        assert!(plan_matches(&item, &original).is_err());
        let mut altered = original.payload;
        altered.action = OperationAction::InstallLocalDeb;
        item.installed_version = "1.0".into();
        assert!(plan_matches(&item, &OperationPlan::new(altered).unwrap()).is_err());
    }
    #[test]
    fn helper_report_must_match_exact_plan_and_phase() {
        let plan = plan(&item("a"));
        let report = serde_json::json!({"planId": plan.plan_id, "verified": true, "dryRun": true, "systemModified": false});
        assert!(verified_report(&report, &plan, true).is_ok());
        assert!(verified_report(&report, &plan, false).is_err());
        for field in ["planId", "verified", "dryRun", "systemModified"] {
            let mut changed = report.clone();
            changed[field] = serde_json::Value::Null;
            assert!(verified_report(&changed, &plan, true).is_err());
        }
    }
    #[test]
    fn restart_invalidates_every_active_phase_and_preserves_success_and_failures() {
        for status in [
            BatchStatus::Preparing,
            BatchStatus::Downloading,
            BatchStatus::AwaitingReview,
            BatchStatus::Reviewing,
            BatchStatus::AwaitingConfirmation,
            BatchStatus::Installing,
        ] {
            let (mut batch, _) = approved();
            batch.status = status;
            batch.items[0].status = ItemStatus::Succeeded;
            let mut snapshot = Snapshot {
                batches: vec![batch],
                ..Snapshot::default()
            };
            recover(&mut snapshot);
            let b = &snapshot.batches[0];
            assert_eq!(b.status, BatchStatus::Interrupted);
            assert!(b.review_id.is_none());
            assert_eq!(b.items[0].status, ItemStatus::Succeeded);
            assert_eq!(b.items[1].status, ItemStatus::Blocked);
        }
    }
    #[test]
    fn interrupted_installation_is_unknown_rather_than_reported_as_not_executed() {
        let (mut batch, _) = approved();
        batch.status = BatchStatus::Installing;
        batch.items[0].status = ItemStatus::Succeeded;
        batch.items[1].status = ItemStatus::Installing;
        let mut snapshot = Snapshot {
            batches: vec![batch],
            ..Snapshot::default()
        };
        recover(&mut snapshot);
        assert_eq!(snapshot.batches[0].items[0].status, ItemStatus::Succeeded);
        assert_eq!(snapshot.batches[0].items[1].status, ItemStatus::Unknown);
        assert!(
            snapshot.batches[0].items[1]
                .error
                .as_deref()
                .unwrap()
                .contains("实际安装状态")
        );
    }

    #[test]
    fn failure_preserves_completed_items_and_blocks_the_rest() {
        let (mut batch, _) = approved();
        batch.items.push(item("third"));
        batch.items[0].status = ItemStatus::Succeeded;
        batch.items[1].status = ItemStatus::Failed;
        batch.items[1].error = Some("dpkg failed".into());
        pause_batch(&mut batch, "Stopped");
        assert_eq!(batch.items[0].status, ItemStatus::Succeeded);
        assert_eq!(batch.items[1].status, ItemStatus::Failed);
        assert_eq!(batch.items[2].status, ItemStatus::Blocked);
        assert!(batch.review_id.is_none());
    }
    #[test]
    fn history_round_trips_logs_versions_and_results_without_approval() {
        let dir = temp_dir();
        let path = dir.join("history.json");
        let (mut batch, _) = approved();
        batch.status = BatchStatus::Completed;
        batch.items[0].status = ItemStatus::Succeeded;
        log(&mut batch.items[0], "recorded log".into());
        let snapshot = Snapshot {
            revision: 4,
            batches: vec![batch],
            storage_error: None,
        };
        save(&path, &snapshot).unwrap();
        let loaded: Snapshot = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(loaded.batches[0].items[0].logs, vec!["recorded log"]);
        assert_eq!(loaded.batches[0].items[0].target_version, "2.0");
        assert_eq!(loaded.batches[0].items[0].status, ItemStatus::Succeeded);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!path.with_extension("tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn persistence_failure_does_not_commit_a_transition_or_start_effects() {
        let dir = temp_dir();
        let file = dir.join("file");
        fs::write(&file, b"not a directory").unwrap();
        let tasks = BatchTasks {
            store: Mutex::new(store(None)),
            path: file.join("history.json"),
            work: Arc::new(tokio::sync::Mutex::new(())),
            download_changed: tokio::sync::Notify::new(),
        };
        assert!(
            tasks
                .change(|snapshot| {
                    snapshot.batches.push(approved().0);
                    Ok(())
                })
                .is_err()
        );
        let snapshot = tasks.snapshot().unwrap();
        assert!(snapshot.batches.is_empty());
        assert!(snapshot.storage_error.is_some());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn logs_are_bounded_and_preserve_utf8() {
        let mut item = item("a");
        for _ in 0..MAX_LOGS + 10 {
            log(&mut item, "安装".repeat(3000));
        }
        assert_eq!(item.logs.len(), MAX_LOGS);
        assert_eq!(item.logs[0].chars().count(), 2000);
    }
    #[test]
    fn download_progress_without_a_phase_change_does_not_write_history() {
        let mut batch = download_batch();
        let mut jobs: Vec<_> = batch.items.iter().map(job).collect();
        for job in &mut jobs {
            job.status = DownloadJobStatus::Downloading;
        }
        assert!(!sync_downloads(&mut batch, &jobs));
        jobs[0].status = DownloadJobStatus::Ready;
        assert!(sync_downloads(&mut batch, &jobs));
        assert!(!sync_downloads(&mut batch, &jobs));
    }

    #[test]
    fn history_budget_trims_logs_without_dropping_the_current_batch() {
        let (mut batch, _) = approved();
        batch.items[0].logs = vec!["中".repeat(512 * 1024); 6];
        let mut snapshot = Snapshot {
            batches: vec![batch],
            ..Snapshot::default()
        };
        trim_history(&mut snapshot).unwrap();
        assert_eq!(snapshot.batches[0].id, "batch");
        assert_eq!(snapshot.batches[0].items[0].logs.len(), 3);
        assert!(serde_json::to_vec(&snapshot).unwrap().len() <= 8 * 1024 * 1024);
    }

    #[test]
    fn journal_write_failure_revokes_approval_and_marks_pending_results_unknown() {
        let dir = temp_dir();
        let file = dir.join("file");
        fs::write(&file, b"not a directory").unwrap();
        let (batch, approval) = approved();
        let mut state = store(Some(approval));
        state.snapshot.batches.push(batch);
        let tasks = BatchTasks {
            store: Mutex::new(state),
            path: file.join("history.json"),
            work: Arc::new(tokio::sync::Mutex::new(())),
            download_changed: tokio::sync::Notify::new(),
        };
        assert!(
            tasks
                .change(|snapshot| {
                    snapshot.batches[0].status = BatchStatus::Installing;
                    Ok(())
                })
                .is_err()
        );
        let store = tasks.store.lock().unwrap();
        assert!(store.approval.is_none());
        assert_eq!(store.snapshot.batches[0].status, BatchStatus::Interrupted);
        assert!(store.snapshot.batches[0].review_id.is_none());
        drop(store);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn preflight_failure_never_runs_any_installation() {
        tauri::async_runtime::block_on(async {
            let count = AtomicUsize::new(0);
            let result = run_reviewed_sequence(
                &[1, 2, 3],
                |_| async { Err("changed state".into()) },
                |_, _| {
                    count.fetch_add(1, Ordering::SeqCst);
                    std::future::ready(Ok(()))
                },
            )
            .await;
            assert!(result.is_err());
            assert_eq!(count.load(Ordering::SeqCst), 0);
        });
    }
    #[test]
    fn serial_execution_stops_at_first_failure_and_never_retries() {
        tauri::async_runtime::block_on(async {
            let effects = Mutex::new(Vec::new());
            let result = run_reviewed_sequence(
                &[1, 2, 3],
                |_| async { Ok(()) },
                |_, item| {
                    effects.lock().unwrap().push(*item);
                    std::future::ready(if *item == 2 {
                        Err("failed".into())
                    } else {
                        Ok(())
                    })
                },
            )
            .await;
            assert!(result.is_err());
            assert_eq!(*effects.lock().unwrap(), vec![1, 2]);
        });
    }
    #[test]
    fn complete_validation_precedes_serial_execution() {
        tauri::async_runtime::block_on(async {
            let events = Mutex::new(Vec::new());
            run_reviewed_sequence(
                &[1, 2],
                |items| {
                    for i in items {
                        events.lock().unwrap().push(format!("verify-{i}"));
                    }
                    std::future::ready(Ok(()))
                },
                |_, i| {
                    events.lock().unwrap().push(format!("install-{i}"));
                    std::future::ready(Ok(()))
                },
            )
            .await
            .unwrap();
            assert_eq!(
                *events.lock().unwrap(),
                vec!["verify-1", "verify-2", "install-1", "install-2"]
            );
        });
    }
}
