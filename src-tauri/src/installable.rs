use crate::scanner::SourceKind;
use crate::source_engine::{self, DownloadPlan};
use serde::Serialize;
use std::path::Path;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallableApplication {
    pub application_id: String,
    pub package_name: String,
    pub display_name: String,
    pub vendor: String,
    pub description: Option<String>,
    pub architecture: String,
    /// `"cli"` for system-level command-line tools, absent for desktop apps.
    pub category: Option<String>,
    pub source_kind: SourceKind,
    pub installed_version: Option<String>,
    pub candidate_version: Option<String>,
    pub install_available: bool,
    pub unavailable_reason: Option<String>,
    pub download_plan: Option<DownloadPlan>,
}

pub async fn load_applications(cache_dir: &Path) -> Result<Vec<InstallableApplication>, String> {
    let applications = crate::feed::effective_applications().await?;
    let mut offers = Vec::new();
    for application in applications.iter().filter(|item| item.is_auto_installable()) {
        offers.push(offer_for(cache_dir, application).await?);
    }
    offers.sort_by(|left, right| left.display_name.cmp(&right.display_name));
    Ok(offers)
}

async fn offer_for(
    cache_dir: &Path,
    application: &umanager_catalog::Application,
) -> Result<InstallableApplication, String> {
    let installable = source_engine::load_installable(application, cache_dir).await?;
    let install_available = installable.installed_version.is_none() && installable.download_plan.is_some();
    let source_kind = if application.is_website_download() {
        SourceKind::OfficialWebsite
    } else {
        SourceKind::OfficialRepository
    };
    let unavailable_reason = if installable.installed_version.is_some() {
        Some("已在本机安装，请在“软件”页管理更新或卸载。".to_owned())
    } else if !install_available {
        Some(format!(
            "未发现 {} 的官方安装来源或候选版本，请确认官方仓库已配置或稍后重试。",
            application.display_name
        ))
    } else {
        None
    };

    Ok(InstallableApplication {
        application_id: application.application_id.clone(),
        package_name: application.package_name.clone(),
        display_name: application.display_name.clone(),
        vendor: application.vendor.clone(),
        description: application.description.clone(),
        architecture: application.architecture.clone(),
        category: application.category.clone(),
        source_kind,
        installed_version: installable.installed_version,
        candidate_version: installable.candidate_version,
        install_available,
        unavailable_reason,
        download_plan: installable.download_plan,
    })
}
