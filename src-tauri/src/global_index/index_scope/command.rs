use super::*;
use crate::{
    db::Database,
    global_index::{GlobalEntryInput, GlobalIndexCoordinator},
    window_auth::require_main_window,
};
use std::path::Path;
use tauri::{Runtime, State, WebviewWindow};
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleRequest {
    pub source_id: String,
    pub path: String,
    pub include: bool,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageRequest {
    pub action: String,
    pub expected_revision: Option<u64>,
    pub mode: Option<CoverageMode>,
    #[serde(default)]
    pub rules: Vec<RuleRequest>,
}
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoverageResponse {
    pub snapshot: CoverageSnapshot,
    pub preview: Option<CoveragePreview>,
}
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CoveragePreview {
    pub sampled: u64,
    pub included: u64,
    pub excluded: u64,
    pub undetermined: u64,
    pub sample_limit: u64,
    pub is_complete: bool,
}
#[tauri::command]
pub fn global_index_coverage<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    coordinator: State<'_, GlobalIndexCoordinator>,
    request: CoverageRequest,
) -> Result<CoverageResponse, String> {
    require_main_window(&window)?;
    execute(db.inner(), coordinator.inner(), request)
}
fn execute(
    db: &Database,
    coordinator: &GlobalIndexCoordinator,
    request: CoverageRequest,
) -> Result<CoverageResponse, String> {
    let snapshot = db.global_index_coverage().map_err(|e| e.to_string())?;
    if request.action == "read" {
        return Ok(CoverageResponse {
            snapshot,
            preview: None,
        });
    }
    if !matches!(request.action.as_str(), "preview" | "save") || request.rules.len() > 128 {
        return Err("coverage_request_invalid".into());
    }
    let revision = request
        .expected_revision
        .ok_or("coverage_revision_required")?;
    if revision != snapshot.policy.revision {
        return Err("coverage_revision_conflict".into());
    }
    let mode = request.mode.ok_or("coverage_mode_required")?;
    let mut rules = if mode == CoverageMode::Custom {
        Vec::new()
    } else {
        snapshot.policy.rules.clone()
    };
    for r in request
        .rules
        .into_iter()
        .filter(|_| mode == CoverageMode::Custom)
    {
        let source = db
            .get_global_volume(&r.source_id)
            .map_err(|e| e.to_string())?
            .ok_or("coverage_source_missing")?;
        rules.push(bind_rule(&source, &r.path, r.include).map_err(|e| e.to_string())?);
    }
    let policy = CoveragePolicy {
        version: POLICY_VERSION,
        revision,
        mode,
        rules,
    };
    if request.action == "save" {
        db.save_global_index_coverage(revision, mode, policy.rules)
            .map_err(|e| e.to_string())?;
        coordinator
            .coverage_policy_changed()
            .map_err(|e| e.to_string())?;
        return Ok(CoverageResponse {
            snapshot: db.global_index_coverage().map_err(|e| e.to_string())?,
            preview: None,
        });
    }
    // Preview is explicitly a bounded sample of existing metadata, not an exact
    // impact count or permission to enumerate new filesystem roots.
    let mut preview = CoveragePreview {
        sample_limit: 1000,
        ..CoveragePreview::default()
    };
    for source in db.list_global_volumes().map_err(|e| e.to_string())? {
        if !source.enabled
            || (source.platform == "windows" && source.scope_authorization_state != "authorized")
        {
            continue;
        }
        let remaining = 1000 - preview.sampled;
        if remaining == 0 {
            break;
        }
        let entries = {
            let conn = db.conn().map_err(|e| e.to_string())?;
            let mut stmt=conn.prepare("SELECT path,platform_file_id,parent_platform_file_id,is_directory,file_attributes FROM global_entries WHERE volume_id=?1 ORDER BY id LIMIT ?2").map_err(|e|e.to_string())?;
            let rows = stmt
                .query_map(rusqlite::params![source.id, remaining as i64], |r| {
                    let path: String = r.get(0)?;
                    // Construct from persisted metadata; never stat before authorization.
                    Ok(GlobalEntryInput {
                        volume_id: source.id.clone(),
                        platform_file_id: r.get(1)?,
                        parent_platform_file_id: r.get(2)?,
                        name: Path::new(&path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        extension: String::new(),
                        path,
                        is_directory: r.get(3)?,
                        size: 0,
                        created_at_fs: None,
                        modified_at_fs: None,
                        file_attributes: r.get(4)?,
                        is_hidden: false,
                        is_system: false,
                        source_provider: source.provider.clone(),
                        last_seen_at: 0,
                    })
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            rows
        };
        for decision in observe_batch(&policy, &source, &entries) {
            preview.sampled += 1;
            match decision {
                Decision::Include => preview.included += 1,
                Decision::Undetermined(_) => preview.undetermined += 1,
                _ => preview.excluded += 1,
            }
        }
    }
    // Omitted files/new roots are not represented in this sample, even below limit.
    Ok(CoverageResponse {
        snapshot,
        preview: Some(preview),
    })
}
