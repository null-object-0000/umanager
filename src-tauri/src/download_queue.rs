//! 后台下载队列。
//!
//! 安装 / 更新都是两阶段：先把官方安装包下载并校验进缓存，再由用户确认、走特权 helper
//! 执行。下载这一步要花几分钟（数百 MB），所以不能把用户锁在详情抽屉里：
//!
//! - 队列常驻在 Tauri managed state，前端随时可以 `list_downloads` 取快照；离开详情页、
//!   切到别的页面、甚至关掉主窗口（程序常驻托盘）都不会中断下载；
//! - 并发数由用户设置（1–3，默认串行），避免几个数百 MB 的包互相抢带宽；
//! - 同一个应用只保留一个未完成任务：重复点「获取 / 更新」不会重复下载；
//! - 取消：排队中的直接出队；下载中的置取消位，下载 / 校验循环按块检查后走既有的
//!   错误路径删除 `.tmp`，因此不会留下半个包；
//! - 状态变化通过 `download-queue-changed` 推送**整份快照**。任务数量很少，而进度本身
//!   已被 `source_engine` 限流到 250ms 一次，快照比增量协议更难写错。
//!
//! 安全边界不变：队列只调用 `source_engine::download_and_verify`，域名白名单、HTTPS、
//! 大小 / SHA-256 / `.deb` 元数据校验全部照旧；队列本身不做任何特权操作。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

use crate::source_engine::{self, CancelCheck, DownloadProgress, DownloadResult};

const CONFIG_FILE_NAME: &str = "downloads.json";
/// 状态变化事件名。载荷是整份队列快照（`Vec<DownloadJob>`）。
pub const EVENT_NAME: &str = "download-queue-changed";

pub const MIN_CONCURRENCY: u8 = 1;
pub const MAX_CONCURRENCY: u8 = 3;
pub const DEFAULT_CONCURRENCY: u8 = 1;

/// 队列设置。目前只有并发数，落盘在应用配置目录（不是密钥，但要跨重启保留）。
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DownloadQueueSettings {
    pub concurrency: u8,
}

impl Default for DownloadQueueSettings {
    fn default() -> Self {
        Self { concurrency: DEFAULT_CONCURRENCY }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum DownloadJobStatus {
    Queued,
    Downloading,
    Verifying,
    /// 下载并校验完成，等待用户确认安装 / 更新（第二阶段）。
    Ready,
    Error,
    Canceled,
}

impl DownloadJobStatus {
    fn is_active(self) -> bool {
        matches!(self, Self::Queued | Self::Downloading | Self::Verifying)
    }
}

/// 一条队列任务。`applicationId` 用于重新解析下载策略，`result` 是第二阶段
/// （`create_operation_plan`）需要的全部信息。
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadJob {
    pub job_id: String,
    pub application_id: String,
    pub package_name: String,
    pub display_name: String,
    /// 目标版本。入队时来自调用方（只用于展示），下载开始后用下载计划里的版本覆盖。
    pub version: Option<String>,
    pub status: DownloadJobStatus,
    pub progress: Option<DownloadProgress>,
    pub error: Option<String>,
    pub result: Option<DownloadResult>,
    pub enqueued_at_unix_seconds: u64,
    pub finished_at_unix_seconds: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CancelOutcome {
    /// 排队中，直接出队。
    Dequeued,
    /// 下载 / 校验中，已置取消位，等待 worker 收尾。
    Requested,
    /// 不存在或已经是终态。
    NotActive,
}

// ---------------------------------------------------------------------------
// 可测的队列核心：只维护顺序、状态与并发计数，不碰网络、事件和文件。
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct QueueCore {
    /// 按入队顺序保存全部任务（含终态），前端据此渲染整个「下载」页。
    jobs: Vec<DownloadJob>,
    running: usize,
    concurrency: u8,
    sequence: u64,
}

impl QueueCore {
    fn new(concurrency: u8) -> Self {
        Self { jobs: Vec::new(), running: 0, concurrency, sequence: 0 }
    }

    fn snapshot(&self) -> Vec<DownloadJob> {
        self.jobs.clone()
    }

    fn concurrency(&self) -> u8 {
        self.concurrency
    }

    fn set_concurrency(&mut self, concurrency: u8) {
        self.concurrency = concurrency;
    }

    fn job(&self, job_id: &str) -> Option<&DownloadJob> {
        self.jobs.iter().find(|job| job.job_id == job_id)
    }

    fn index_of(&self, job_id: &str) -> Option<usize> {
        self.jobs.iter().position(|job| job.job_id == job_id)
    }

    /// 入队。同一个应用已有「排队中 / 下载中 / 待安装」的任务时复用那一条（返回 `false`）；
    /// 上一次失败或已取消的任务会被替换掉，因此「重试」就是再入队一次。
    fn enqueue(
        &mut self,
        application_id: &str,
        package_name: &str,
        display_name: &str,
        version: Option<String>,
    ) -> (String, bool) {
        if let Some(existing) = self
            .jobs
            .iter()
            .find(|job| job.application_id == application_id && job.status.is_active())
        {
            return (existing.job_id.clone(), false);
        }
        if let Some(existing) = self
            .jobs
            .iter()
            .find(|job| job.application_id == application_id && job.status == DownloadJobStatus::Ready)
        {
            return (existing.job_id.clone(), false);
        }
        // 只替换同一个应用的终态（失败 / 已取消）任务，其它历史保留给用户查看。
        if let Some(index) = self
            .jobs
            .iter()
            .position(|job| job.application_id == application_id && !job.status.is_active())
        {
            self.jobs.remove(index);
        }
        self.sequence += 1;
        let job_id = format!("download-{}-{}", unix_timestamp_now(), self.sequence);
        self.jobs.push(DownloadJob {
            job_id: job_id.clone(),
            application_id: application_id.to_owned(),
            package_name: package_name.to_owned(),
            display_name: display_name.to_owned(),
            version,
            status: DownloadJobStatus::Queued,
            progress: None,
            error: None,
            result: None,
            enqueued_at_unix_seconds: unix_timestamp_now(),
            finished_at_unix_seconds: None,
        });
        (job_id, true)
    }

    /// 按入队顺序取出还能启动的任务，直到用满并发额度。
    fn startable(&self) -> Vec<String> {
        let capacity = (self.concurrency as usize).saturating_sub(self.running);
        self.jobs
            .iter()
            .filter(|job| job.status == DownloadJobStatus::Queued)
            .take(capacity)
            .map(|job| job.job_id.clone())
            .collect()
    }

    /// 把任务标记为「下载中」并占用一个并发位。只有仍在排队中的任务能启动 ——
    /// 否则「取出可启动任务」与「用户取消」之间的竞态会把已取消的任务重新拉起来。
    fn mark_running(&mut self, job_id: &str) -> bool {
        let Some(index) = self.index_of(job_id) else { return false };
        if self.jobs[index].status != DownloadJobStatus::Queued {
            return false;
        }
        self.jobs[index].status = DownloadJobStatus::Downloading;
        self.running += 1;
        true
    }

    /// 记录一次进度。返回是否命中了任务（未命中时调用方不必推送事件）。
    ///
    /// 注意 `completed` 仍算「进行中」：`download_and_verify` 在返回前会发一次
    /// `completed` 进度，此时任务还占着并发位，真正的终态由 worker 调 `finish` 写入。
    /// 如果这里直接标成 `Ready`，`finish` 就会认为它没占位，并发额度会被永久漏掉。
    fn set_progress(&mut self, job_id: &str, progress: DownloadProgress) -> bool {
        let Some(index) = self.index_of(job_id) else { return false };
        let job = &mut self.jobs[index];
        job.status = match progress.phase {
            "downloading" => DownloadJobStatus::Downloading,
            _ => DownloadJobStatus::Verifying,
        };
        job.progress = Some(progress);
        true
    }

    /// 记录下载计划里的版本（入队时调用方给的版本可能只是展示用的候选版本）。
    fn set_version(&mut self, job_id: &str, version: String) {
        if let Some(index) = self.index_of(job_id) {
            self.jobs[index].version = Some(version);
        }
    }

    /// 任务结束：写入终态并释放并发位。
    fn finish(
        &mut self,
        job_id: &str,
        status: DownloadJobStatus,
        error: Option<String>,
        result: Option<DownloadResult>,
    ) {
        let Some(index) = self.index_of(job_id) else { return };
        let was_running = matches!(
            self.jobs[index].status,
            DownloadJobStatus::Downloading | DownloadJobStatus::Verifying
        );
        if was_running {
            self.running = self.running.saturating_sub(1);
        }
        let job = &mut self.jobs[index];
        job.status = status;
        job.error = error;
        job.result = result;
        job.progress = None;
        job.finished_at_unix_seconds = Some(unix_timestamp_now());
    }

    fn cancel(&mut self, job_id: &str) -> CancelOutcome {
        let Some(index) = self.index_of(job_id) else { return CancelOutcome::NotActive };
        match self.jobs[index].status {
            DownloadJobStatus::Queued => {
                self.jobs[index].status = DownloadJobStatus::Canceled;
                self.jobs[index].finished_at_unix_seconds = Some(unix_timestamp_now());
                CancelOutcome::Dequeued
            }
            DownloadJobStatus::Downloading | DownloadJobStatus::Verifying => CancelOutcome::Requested,
            _ => CancelOutcome::NotActive,
        }
    }

    /// 移除一条终态任务（前端「清除记录」）。进行中的任务必须先取消。
    fn remove(&mut self, job_id: &str) -> bool {
        match self.index_of(job_id) {
            Some(index) if !self.jobs[index].status.is_active() => {
                self.jobs.remove(index);
                true
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// 运行时：核心 + 取消位 + 调度唤醒
// ---------------------------------------------------------------------------

pub struct DownloadQueue {
    core: Mutex<QueueCore>,
    /// 每个运行中的任务一个取消位。worker 启动时取用，取消时置位。
    cancel_flags: Mutex<HashMap<String, Arc<AtomicBool>>>,
    /// 入队 / 结束 / 取消 / 改并发数时唤醒调度器。
    notify: tokio::sync::Notify,
}

impl DownloadQueue {
    fn new(concurrency: u8) -> Self {
        Self {
            core: Mutex::new(QueueCore::new(concurrency)),
            cancel_flags: Mutex::new(HashMap::new()),
            notify: tokio::sync::Notify::new(),
        }
    }

    fn lock_core(&self) -> MutexGuard<'_, QueueCore> {
        self.core.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn snapshot(&self) -> Vec<DownloadJob> {
        self.lock_core().snapshot()
    }

    /// 取（必要时创建）某个任务的取消位。worker 与取消命令共用同一份 `Arc`。
    fn cancel_flag(&self, job_id: &str) -> Arc<AtomicBool> {
        let mut flags = self.cancel_flags.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        flags.entry(job_id.to_owned()).or_default().clone()
    }

    fn request_cancel(&self, job_id: &str) {
        self.cancel_flag(job_id).store(true, Ordering::SeqCst);
    }

    fn forget_cancel_flag(&self, job_id: &str) {
        let mut flags = self.cancel_flags.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        flags.remove(job_id);
    }
}

/// 启动调度器并把队列挂到应用状态上。只在 setup 里调用一次。
pub fn initialize(app: &AppHandle) {
    let settings = load_settings(app);
    app.manage(DownloadQueue::new(settings.concurrency));
    let handle = app.clone();
    tauri::async_runtime::spawn(async move { schedule(handle).await });
}

/// 调度循环：只要还有并发额度就启动队首任务，否则等下一次唤醒。
///
/// `notified()` 必须在检查队列**之前**创建：否则「检查完发现没活干」与「任务结束发通知」
/// 之间会丢掉一次唤醒，队列就永久停在那里了。
async fn schedule(app: AppHandle) {
    loop {
        // `notified()` 借用 `queue.notify`，因此 `queue` 必须活到本次循环结束。
        let queue = app.state::<DownloadQueue>();
        let notified = queue.notify.notified();
        let startable = {
            let mut core = queue.lock_core();
            core.startable()
                .into_iter()
                .filter(|job_id| core.mark_running(job_id))
                .collect::<Vec<_>>()
        };
        for job_id in startable {
            let handle = app.clone();
            tauri::async_runtime::spawn(async move { run_job(handle, job_id).await });
        }
        publish(&app);
        notified.await;
    }
}

/// 单个任务的完整生命周期：解析下载策略 → 下载校验 → 写终态 → 唤醒调度器。
async fn run_job(app: AppHandle, job_id: String) {
    let outcome = execute(&app, &job_id).await;
    {
        let queue = app.state::<DownloadQueue>();
        queue.lock_core().finish(&job_id, outcome.status, outcome.error, outcome.result);
        queue.forget_cancel_flag(&job_id);
        queue.notify.notify_one();
    }
    publish(&app);
}

struct Outcome {
    status: DownloadJobStatus,
    error: Option<String>,
    result: Option<DownloadResult>,
}

impl Outcome {
    fn error(message: String) -> Self {
        Self { status: DownloadJobStatus::Error, error: Some(message), result: None }
    }

    fn canceled() -> Self {
        Self { status: DownloadJobStatus::Canceled, error: None, result: None }
    }

    fn ready(result: DownloadResult) -> Self {
        Self { status: DownloadJobStatus::Ready, error: None, result: Some(result) }
    }
}

async fn execute(app: &AppHandle, job_id: &str) -> Outcome {
    let (application_id, cancel, display_name) = {
        let queue = app.state::<DownloadQueue>();
        let core = queue.lock_core();
        let Some(job) = core.job(job_id) else {
            return Outcome::canceled();
        };
        (job.application_id.clone(), queue.cancel_flag(job_id), job.display_name.clone())
    };
    let cancel_check: CancelCheck = {
        let flag = cancel.clone();
        Arc::new(move || flag.load(Ordering::SeqCst))
    };
    if cancel_check() {
        return Outcome::canceled();
    }

    let catalog = match crate::feed::effective_catalog().await {
        Ok(catalog) => catalog,
        Err(error) => return Outcome::error(error),
    };
    let application = match crate::require_application(&catalog, &application_id) {
        Ok(application) => application,
        Err(error) => return Outcome::error(error),
    };
    let cache_dir = match app.path().app_cache_dir() {
        Ok(dir) => dir,
        Err(error) => return Outcome::error(format!("无法确定 UManager 缓存目录：{error}")),
    };

    let progress: source_engine::ProgressCallback = {
        let app = app.clone();
        let job_id = job_id.to_owned();
        Arc::new(move |payload: DownloadProgress| {
            let queue = app.state::<DownloadQueue>();
            if queue.lock_core().set_progress(&job_id, payload) {
                publish(&app);
            }
        })
    };

    // 版本先用下载计划里的值补齐，让队列页在下载刚开始时就能显示目标版本。
    if let Ok(plan) = source_engine::build_download_plan(&application, &cache_dir).await {
        app.state::<DownloadQueue>().lock_core().set_version(job_id, plan.version);
        publish(app);
    }
    if cancel_check() {
        return Outcome::canceled();
    }

    match source_engine::download_and_verify(&application, cache_dir, progress, cancel_check.clone()).await {
        Ok(result) => {
            // 取消与「刚好下完」同时发生时以用户意图为准；已校验的文件留在缓存里，
            // 下次重新入队会直接命中缓存，不会重复下载。
            if cancel_check() {
                Outcome::canceled()
            } else {
                Outcome::ready(result)
            }
        }
        Err(error) => {
            if cancel_check() || error == source_engine::CANCELED_ERROR {
                Outcome::canceled()
            } else {
                Outcome::error(format!("{}：{error}", display_name))
            }
        }
    }
}

fn publish(app: &AppHandle) {
    let snapshot = app.state::<DownloadQueue>().snapshot();
    let _ = app.emit(EVENT_NAME, snapshot);
}

// ---------------------------------------------------------------------------
// 设置持久化
// ---------------------------------------------------------------------------

fn settings_path(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|dir| dir.join(CONFIG_FILE_NAME))
}

fn load_settings(app: &AppHandle) -> DownloadQueueSettings {
    settings_path(app)
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<DownloadQueueSettings>(&text).ok())
        .map(|settings| sanitize(settings).unwrap_or_default())
        .unwrap_or_default()
}

fn store_settings(path: &Path, settings: &DownloadQueueSettings) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 UManager 配置目录：{error}"))?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|error| format!("无法编码下载设置：{error}"))?;
    std::fs::write(path, json).map_err(|error| format!("无法保存下载设置：{error}"))
}

fn sanitize(settings: DownloadQueueSettings) -> Result<DownloadQueueSettings, String> {
    if !(MIN_CONCURRENCY..=MAX_CONCURRENCY).contains(&settings.concurrency) {
        return Err(format!("下载并发数只能是 {MIN_CONCURRENCY}–{MAX_CONCURRENCY}"));
    }
    Ok(settings)
}

fn unix_timestamp_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// 当前队列快照。前端在挂载时先拉一次，之后靠 `download-queue-changed` 事件更新。
#[tauri::command]
pub fn list_downloads(queue: tauri::State<'_, DownloadQueue>) -> Vec<DownloadJob> {
    queue.snapshot()
}

/// 把一个应用加入下载队列。重复入队会复用已有任务，不会重复下载。
#[tauri::command]
pub async fn enqueue_download(
    app: AppHandle,
    application_id: String,
    version: Option<String>,
) -> Result<Vec<DownloadJob>, String> {
    let catalog = crate::feed::effective_catalog().await?;
    let application = crate::require_application(&catalog, &application_id)?;
    {
        let queue = app.state::<DownloadQueue>();
        queue.lock_core().enqueue(
            &application.application_id,
            &application.package_name,
            &application.display_name,
            version,
        );
        queue.notify.notify_one();
    }
    publish(&app);
    Ok(app.state::<DownloadQueue>().snapshot())
}

/// 取消一条任务：排队中的直接出队，下载中的置取消位（worker 会在一个数据块内收尾）。
#[tauri::command]
pub fn cancel_download(app: AppHandle, job_id: String) -> Vec<DownloadJob> {
    {
        let queue = app.state::<DownloadQueue>();
        if queue.lock_core().cancel(&job_id) == CancelOutcome::Requested {
            queue.request_cancel(&job_id);
        }
        queue.notify.notify_one();
    }
    publish(&app);
    app.state::<DownloadQueue>().snapshot()
}

/// 清除一条已结束（完成 / 失败 / 已取消）的任务记录。进行中的任务要先取消。
#[tauri::command]
pub fn remove_download(app: AppHandle, job_id: String) -> Vec<DownloadJob> {
    {
        let queue = app.state::<DownloadQueue>();
        queue.lock_core().remove(&job_id);
        queue.forget_cancel_flag(&job_id);
    }
    publish(&app);
    app.state::<DownloadQueue>().snapshot()
}

#[tauri::command]
pub fn get_download_settings(queue: tauri::State<'_, DownloadQueue>) -> DownloadQueueSettings {
    DownloadQueueSettings { concurrency: queue.lock_core().concurrency() }
}

#[tauri::command]
pub fn set_download_settings(
    app: AppHandle,
    settings: DownloadQueueSettings,
) -> Result<DownloadQueueSettings, String> {
    let sanitized = sanitize(settings)?;
    if let Some(path) = settings_path(&app) {
        store_settings(&path, &sanitized)?;
    }
    {
        let queue = app.state::<DownloadQueue>();
        queue.lock_core().set_concurrency(sanitized.concurrency);
        // 调大并发后立刻把排队中的任务拉起来。
        queue.notify.notify_one();
    }
    Ok(sanitized)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core(concurrency: u8) -> QueueCore {
        QueueCore::new(concurrency)
    }

    fn progress(phase: &'static str, transferred: u64) -> DownloadProgress {
        DownloadProgress {
            package_name: "demo".to_owned(),
            phase,
            transferred_bytes: transferred,
            total_bytes: 100,
            bytes_per_second: 0,
        }
    }

    #[test]
    fn enqueue_keeps_fifo_order_and_unique_ids() {
        let mut core = core(1);
        let (first, created) = core.enqueue("a", "pkg-a", "A", None);
        let (second, _) = core.enqueue("b", "pkg-b", "B", None);

        assert!(created);
        assert_ne!(first, second);
        assert_eq!(
            core.snapshot().iter().map(|job| job.job_id.clone()).collect::<Vec<_>>(),
            vec![first.clone(), second.clone()]
        );
        assert!(core.snapshot().iter().all(|job| job.status == DownloadJobStatus::Queued));
    }

    #[test]
    fn enqueue_reuses_active_and_ready_jobs_but_replaces_finished_ones() {
        let mut core = core(1);
        let (queued, created) = core.enqueue("a", "pkg-a", "A", None);
        let (again, created_again) = core.enqueue("a", "pkg-a", "A", None);
        assert!(created);
        assert!(!created_again);
        assert_eq!(queued, again);
        assert_eq!(core.snapshot().len(), 1);

        core.mark_running(&queued);
        core.finish(&queued, DownloadJobStatus::Error, Some("boom".to_owned()), None);
        let (retried, created_retry) = core.enqueue("a", "pkg-a", "A", None);
        assert!(created_retry);
        assert_ne!(queued, retried);
        assert_eq!(core.snapshot().len(), 1, "失败记录被重试替换");

        core.mark_running(&retried);
        core.finish(&retried, DownloadJobStatus::Ready, None, None);
        let (ready, created_ready) = core.enqueue("a", "pkg-a", "A", None);
        assert!(!created_ready);
        assert_eq!(ready, retried, "待安装的任务不会被重复下载覆盖");
    }

    #[test]
    fn startable_respects_the_concurrency_limit_in_order() {
        let mut core = core(2);
        let (first, _) = core.enqueue("a", "pkg-a", "A", None);
        let (second, _) = core.enqueue("b", "pkg-b", "B", None);
        let (third, _) = core.enqueue("c", "pkg-c", "C", None);

        assert_eq!(core.startable(), vec![first.clone(), second.clone()]);
        assert!(core.mark_running(&first));
        assert!(core.mark_running(&second));
        assert!(core.startable().is_empty(), "并发额度已用满");

        core.finish(&first, DownloadJobStatus::Ready, None, None);
        assert_eq!(core.startable(), vec![third.clone()], "空出额度后启动下一个");
        assert!(core.mark_running(&third));
        assert!(core.startable().is_empty());
    }

    #[test]
    fn mark_running_refuses_jobs_that_are_no_longer_queued() {
        let mut core = core(1);
        let (job, _) = core.enqueue("a", "pkg-a", "A", None);
        // 模拟「取出可启动任务」与「用户取消」之间的竞态。
        assert_eq!(core.cancel(&job), CancelOutcome::Dequeued);
        assert!(!core.mark_running(&job), "已取消的任务不能被重新拉起来");
        assert_eq!(core.snapshot()[0].status, DownloadJobStatus::Canceled);
    }

    #[test]
    fn canceling_a_running_job_keeps_it_running_until_the_worker_stops() {
        let mut core = core(1);
        let (job, _) = core.enqueue("a", "pkg-a", "A", None);
        core.mark_running(&job);

        assert_eq!(core.cancel(&job), CancelOutcome::Requested);
        assert_eq!(core.snapshot()[0].status, DownloadJobStatus::Downloading);
        assert!(core.startable().is_empty(), "取消中的任务仍占着并发位");

        core.finish(&job, DownloadJobStatus::Canceled, None, None);
        assert_eq!(core.snapshot()[0].status, DownloadJobStatus::Canceled);
        assert!(core.snapshot()[0].finished_at_unix_seconds.is_some());
    }

    #[test]
    fn progress_updates_the_status_and_releases_the_slot_only_once() {
        let mut core = core(1);
        let (job, _) = core.enqueue("a", "pkg-a", "A", None);
        core.mark_running(&job);

        assert!(core.set_progress(&job, progress("downloading", 10)));
        assert_eq!(core.snapshot()[0].status, DownloadJobStatus::Downloading);
        assert!(core.set_progress(&job, progress("verifying", 100)));
        assert_eq!(core.snapshot()[0].status, DownloadJobStatus::Verifying);
        assert!(!core.set_progress("missing", progress("downloading", 0)));

        core.finish(&job, DownloadJobStatus::Ready, None, None);
        // 重复 finish（例如取消后又收到 worker 收尾）不应把并发计数减成负数。
        core.finish(&job, DownloadJobStatus::Canceled, None, None);
        let (next, _) = core.enqueue("b", "pkg-b", "B", None);
        assert_eq!(core.startable(), vec![next], "并发位已正确归还");
    }

    #[test]
    fn the_completed_progress_event_does_not_release_the_concurrency_slot() {
        let mut core = core(1);
        let (first, _) = core.enqueue("a", "pkg-a", "A", None);
        let (second, _) = core.enqueue("b", "pkg-b", "B", None);
        core.mark_running(&first);

        // download_and_verify 在返回前会发一次 `completed`：任务仍占着并发位，
        // 直到 worker 调 finish 才算结束（否则并发额度会被永久漏掉）。
        assert!(core.set_progress(&first, progress("completed", 100)));
        assert_eq!(core.snapshot()[0].status, DownloadJobStatus::Verifying);
        assert!(core.startable().is_empty(), "完成后仍未 finish，不能启动下一个");

        core.finish(&first, DownloadJobStatus::Ready, None, None);
        assert_eq!(core.startable(), vec![second], "finish 之后额度归还");
    }

    #[test]
    fn remove_only_drops_finished_jobs() {
        let mut core = core(1);
        let (job, _) = core.enqueue("a", "pkg-a", "A", None);
        assert!(!core.remove(&job), "排队中的任务不能直接删掉");

        core.mark_running(&job);
        assert!(!core.remove(&job), "下载中的任务不能直接删掉");

        core.finish(&job, DownloadJobStatus::Error, Some("boom".to_owned()), None);
        assert!(core.remove(&job));
        assert!(core.snapshot().is_empty());
    }

    #[test]
    fn concurrency_can_be_raised_and_lowered() {
        let mut core = core(1);
        for (index, id) in ["a", "b", "c"].iter().enumerate() {
            core.enqueue(id, &format!("pkg-{index}"), id, None);
        }
        assert_eq!(core.startable().len(), 1);

        core.set_concurrency(3);
        assert_eq!(core.startable().len(), 3);

        core.set_concurrency(1);
        assert_eq!(core.startable().len(), 1);
    }

    #[test]
    fn sanitize_rejects_out_of_range_concurrency() {
        assert!(sanitize(DownloadQueueSettings { concurrency: 1 }).is_ok());
        assert!(sanitize(DownloadQueueSettings { concurrency: 3 }).is_ok());
        assert!(sanitize(DownloadQueueSettings { concurrency: 0 }).is_err());
        assert!(sanitize(DownloadQueueSettings { concurrency: 4 }).is_err());
    }

    #[test]
    fn settings_round_trip_through_the_file() {
        let path = std::env::temp_dir().join(format!(
            "umanager-download-settings-{}-{}/downloads.json",
            std::process::id(),
            unix_timestamp_now()
        ));
        let settings = DownloadQueueSettings { concurrency: 2 };
        store_settings(&path, &settings).unwrap();
        let loaded: DownloadQueueSettings =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.concurrency, 2);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn cancel_flags_are_shared_between_worker_and_command() {
        let queue = DownloadQueue::new(1);
        let flag = queue.cancel_flag("job-1");
        assert!(!flag.load(Ordering::SeqCst));
        queue.request_cancel("job-1");
        assert!(flag.load(Ordering::SeqCst), "取消命令置位后 worker 能看到");

        queue.forget_cancel_flag("job-1");
        assert!(!queue.cancel_flag("job-1").load(Ordering::SeqCst), "清理后是新的一位");
    }
}
