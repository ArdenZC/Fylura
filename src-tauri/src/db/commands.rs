use super::*;
use crate::file_ops::OperationLogDto;
use crate::window_auth::require_main_window;
use tauri::{AppHandle, Emitter, Runtime, State, WebviewWindow};

const FS_WATCHER_WARNING_EVENT: &str = "fs-watcher-warning";

#[tauri::command]
pub fn init_db<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
) -> Result<(), String> {
    require_main_window(&window)?;
    db.init().map_err(command_error)
}

#[tauri::command]
pub fn insert_file<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    file: InsertFileRequest,
) -> Result<(), String> {
    require_main_window(&window)?;
    db.insert_file(file).map_err(command_error)
}

#[tauri::command]
pub fn remove_files_by_paths<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    paths: Vec<String>,
) -> Result<usize, String> {
    require_main_window(&window)?;
    db.remove_files_by_paths(&paths).map_err(command_error)
}

#[tauri::command]
pub fn upsert_files_by_paths<R: Runtime>(
    app: AppHandle<R>,
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    paths: Vec<String>,
) -> Result<usize, String> {
    require_main_window(&window)?;
    let db = db.inner();
    let result = upsert_files_by_paths_for_db_with_warnings(db, &paths).map_err(command_error)?;
    for warning in &result.warnings {
        emit_fs_watcher_warning(&app, warning);
    }
    let upserted = result.upserted;
    if let Some(report) = optimize_search_index_after_bulk_upsert(db, upserted) {
        emit_search_index_optimized(&app, &report);
    }
    Ok(upserted)
}

#[tauri::command]
pub fn search_files(
    db: State<'_, Database>,
    query: String,
    limit: Option<u32>,
    scope: Option<LibraryScope>,
) -> Result<Vec<FileRecordDto>, String> {
    match scope.as_ref() {
        Some(scope) => db
            .search_files_in_scope(&query, limit, scope)
            .map_err(command_error),
        None => db.search_files(&query, limit).map_err(command_error),
    }
}

#[tauri::command]
pub fn get_paged_files(
    db: State<'_, Database>,
    limit: Option<u32>,
    offset: Option<u32>,
    query: Option<String>,
    scope: Option<LibraryScope>,
    filter: Option<FileLibraryFilter>,
) -> Result<PagedFilesResult, String> {
    let scope = scope.unwrap_or(LibraryScope::All);
    db.get_paged_files_in_scope_with_filter(
        limit,
        offset,
        query.as_deref(),
        &scope,
        filter.as_ref(),
    )
    .map_err(command_error)
}

#[tauri::command]
pub fn query_file_library_v2<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: FileQueryRequestV2,
) -> Result<FileQueryResponseV2, String> {
    require_main_window(&window)?;
    db.query_file_library_v2(request).map_err(command_error)
}

#[tauri::command]
pub fn resolve_file_library_exact_count_v2<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: ResolveFileLibraryExactCountRequestV2,
) -> Result<ResolveFileLibraryExactCountResponseV2, String> {
    require_main_window(&window)?;
    db.resolve_file_library_exact_count_v2(request)
        .map_err(command_error)
}

#[tauri::command]
pub fn get_file_library_detail<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    file_id: String,
) -> Result<FileLibraryDetailDto, String> {
    require_main_window(&window)?;
    db.get_file_library_detail(&file_id).map_err(command_error)
}

#[tauri::command]
pub fn get_file_library_selection_summary<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    selection: LibrarySelectionV1,
) -> Result<FileLibrarySelectionSummaryDto, String> {
    require_main_window(&window)?;
    db.get_file_library_selection_summary(selection)
        .map_err(command_error)
}

#[tauri::command]
pub fn reveal_file_library_entry<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    file_id: String,
) -> Result<(), String> {
    require_main_window(&window)?;
    let path = db
        .resolve_file_library_path(&file_id)
        .map_err(command_error)?;
    crate::file_ops::reveal_in_folder(path)
}

#[tauri::command]
pub fn list_user_tags<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
) -> Result<Vec<UserTagDto>, String> {
    require_main_window(&window)?;
    db.list_user_tags().map_err(command_error)
}

#[tauri::command]
pub fn create_user_tag<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: CreateUserTagRequest,
) -> Result<UserTagDto, String> {
    require_main_window(&window)?;
    db.create_user_tag(request).map_err(command_error)
}

#[tauri::command]
pub fn update_user_tag<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: UpdateUserTagRequest,
) -> Result<UserTagDto, String> {
    require_main_window(&window)?;
    db.update_user_tag(request).map_err(command_error)
}

#[tauri::command]
pub fn delete_user_tag<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: DeleteUserTagRequest,
) -> Result<bool, String> {
    require_main_window(&window)?;
    db.delete_user_tag(request).map_err(command_error)
}

#[tauri::command]
pub fn mutate_file_user_tags<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: MutateFileUserTagsRequest,
) -> Result<MutateFileUserTagsResultDto, String> {
    require_main_window(&window)?;
    db.mutate_file_user_tags(request).map_err(command_error)
}

#[tauri::command]
pub fn list_library_saved_views<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
) -> Result<Vec<LibrarySavedViewDto>, String> {
    require_main_window(&window)?;
    db.list_library_saved_views().map_err(command_error)
}

#[tauri::command]
pub fn create_library_saved_view<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: CreateLibrarySavedViewRequest,
) -> Result<LibrarySavedViewDto, String> {
    require_main_window(&window)?;
    db.create_library_saved_view(request).map_err(command_error)
}

#[tauri::command]
pub fn update_library_saved_view<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: UpdateLibrarySavedViewRequest,
) -> Result<LibrarySavedViewDto, String> {
    require_main_window(&window)?;
    db.update_library_saved_view(request).map_err(command_error)
}

#[tauri::command]
pub fn delete_library_saved_view<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: DeleteLibrarySavedViewRequest,
) -> Result<bool, String> {
    require_main_window(&window)?;
    db.delete_library_saved_view(request).map_err(command_error)
}

#[tauri::command]
pub fn create_organization_plan<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: CreateOrganizationPlanRequestV1,
) -> Result<OrganizationPlanDto, String> {
    require_main_window(&window)?;
    db.create_organization_plan(request).map_err(command_error)
}

#[tauri::command]
pub fn list_organization_plans<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
) -> Result<Vec<OrganizationPlanDto>, String> {
    require_main_window(&window)?;
    db.list_organization_plans().map_err(command_error)
}

#[tauri::command]
pub fn get_organization_plan<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    plan_id: String,
) -> Result<OrganizationPlanDto, String> {
    require_main_window(&window)?;
    db.get_organization_plan(&plan_id).map_err(command_error)
}

#[tauri::command]
pub fn query_organization_plan_items<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: QueryOrganizationPlanItemsRequest,
) -> Result<OrganizationPlanItemPageDto, String> {
    require_main_window(&window)?;
    db.query_organization_plan_items(request)
        .map_err(command_error)
}

#[tauri::command]
pub fn query_organization_plan_groups<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: QueryOrganizationPlanGroupsRequest,
) -> Result<OrganizationPlanGroupPageDto, String> {
    require_main_window(&window)?;
    db.query_organization_plan_groups(request)
        .map_err(command_error)
}

#[tauri::command]
pub fn query_organization_plan_group_items<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: QueryOrganizationPlanGroupItemsRequest,
) -> Result<OrganizationPlanGroupItemPageDto, String> {
    require_main_window(&window)?;
    db.query_organization_plan_group_items(request)
        .map_err(command_error)
}

#[tauri::command]
pub fn update_organization_plan_decisions<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: UpdateOrganizationPlanDecisionsRequest,
) -> Result<OrganizationPlanDto, String> {
    require_main_window(&window)?;
    db.update_organization_plan_decisions(request)
        .map_err(command_error)
}

#[tauri::command]
pub fn update_organization_plan_group_decision<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: UpdateOrganizationPlanGroupDecisionRequest,
) -> Result<UpdateOrganizationPlanGroupDecisionResultDto, String> {
    require_main_window(&window)?;
    db.update_organization_plan_group_decision(request)
        .map_err(command_error)
}

#[tauri::command]
pub fn refresh_organization_plan<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: OrganizationPlanRevisionRequest,
) -> Result<OrganizationPlanDto, String> {
    require_main_window(&window)?;
    db.refresh_organization_plan(request).map_err(command_error)
}

#[tauri::command]
pub fn cancel_organization_plan<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: OrganizationPlanRevisionRequest,
) -> Result<OrganizationPlanDto, String> {
    require_main_window(&window)?;
    db.cancel_organization_plan(request).map_err(command_error)
}

#[tauri::command]
pub fn delete_organization_plan<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: DeleteOrganizationPlanRequest,
) -> Result<bool, String> {
    require_main_window(&window)?;
    db.delete_organization_plan(request).map_err(command_error)
}

#[tauri::command]
pub fn analyze_organization_plan_items<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: AnalyzeOrganizationPlanItemsRequest,
) -> Result<AnalyzeOrganizationPlanItemsResult, String> {
    require_main_window(&window)?;
    db.analyze_organization_plan_items(request)
        .map_err(command_error)
}

#[tauri::command]
pub fn get_organization_plan_dry_run<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: OrganizationPlanSelectionRequest,
) -> Result<OrganizationPlanDryRunDto, String> {
    require_main_window(&window)?;
    db.get_organization_plan_dry_run(request)
        .map_err(command_error)
}

#[tauri::command]
pub async fn execute_organization_plan<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    db: State<'_, Database>,
    cancel: State<'_, crate::file_ops::OperationCancellationToken>,
    request: ExecuteOrganizationPlanRequest,
) -> Result<ExecuteOrganizationPlanResultDto, String> {
    require_main_window(&window)?;
    let database = db.inner().clone();
    let dispatch = database
        .begin_organization_plan_execution(&request)
        .map_err(command_error)?;
    let move_request = crate::file_ops::ExecuteMovesRequest {
        operations: dispatch.selections.clone(),
    };
    let execution = crate::file_ops::execute_canonical_operations(
        app,
        database.clone(),
        cancel.inner().clone(),
        move_request,
        dispatch.operation_batch_id.clone(),
    )
    .await;
    match execution {
        Ok(result) => database
            .finalize_organization_plan_execution(&request.plan_id, &dispatch, &result.logs)
            .map_err(command_error),
        Err(error) => {
            let _ = database.fail_unjournaled_organization_execution(
                &request.plan_id,
                &dispatch,
                &error,
            );
            Err(error)
        }
    }
}

#[tauri::command]
pub fn get_operation_previews_for_scope(
    db: State<'_, Database>,
    scope: LibraryScope,
    filter: Option<FileLibraryFilter>,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<OperationPreviewScopeResult, String> {
    db.get_operation_previews_for_scope(&scope, filter.as_ref(), limit, offset)
        .map_err(command_error)
}

#[tauri::command]
pub fn get_operation_previews_by_file_ids(
    db: State<'_, Database>,
    file_ids: Vec<String>,
) -> Result<Vec<OperationPreviewDto>, String> {
    db.get_operation_previews_by_file_ids(&file_ids)
        .map_err(command_error)
}

#[tauri::command]
pub fn get_manual_rename_operation_preview(
    db: State<'_, Database>,
    file_id: String,
    new_name: String,
) -> Result<OperationPreviewDto, String> {
    db.get_manual_rename_operation_preview(&file_id, &new_name)
        .map_err(command_error)
}

#[tauri::command]
pub fn get_permanent_delete_operation_preview(
    db: State<'_, Database>,
    file_id: String,
) -> Result<OperationPreviewDto, String> {
    db.get_permanent_delete_operation_preview(&file_id)
        .map_err(command_error)
}

#[tauri::command]
pub fn get_operation_previews_for_selection(
    db: State<'_, Database>,
    selection: LibrarySelectionV1,
    limit: Option<u32>,
    offset: Option<u32>,
) -> Result<OperationPreviewScopeResult, String> {
    db.get_operation_previews_for_selection(&selection, limit, offset)
        .map_err(command_error)
}

#[tauri::command]
pub fn get_stats_summary(
    db: State<'_, Database>,
    scope: Option<LibraryScope>,
) -> Result<StatsSummary, String> {
    match scope.as_ref() {
        Some(scope) => db.get_stats_summary_in_scope(scope).map_err(command_error),
        None => db.get_stats_summary().map_err(command_error),
    }
}

#[tauri::command]
pub fn get_operation_logs(
    db: State<'_, Database>,
    limit: Option<u32>,
) -> Result<Vec<OperationLogDto>, String> {
    db.get_operation_logs(limit).map_err(command_error)
}

#[tauri::command]
pub fn get_rule_catalog_state(db: State<'_, Database>) -> Result<RuleCatalogStateDto, String> {
    db.get_rule_catalog_state().map_err(command_error)
}

#[tauri::command]
pub fn list_user_rules_v2(db: State<'_, Database>) -> Result<Vec<UserRuleV2>, String> {
    db.list_user_rules_v2().map_err(command_error)
}

#[tauri::command]
pub fn create_user_rule_v2<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: CreateUserRuleV2Request,
) -> Result<RuleMutationResultV2, String> {
    require_main_window(&window)?;
    db.create_user_rule_v2(request).map_err(command_error)
}

#[tauri::command]
pub fn update_user_rule_v2<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: UpdateUserRuleV2Request,
) -> Result<RuleMutationResultV2, String> {
    require_main_window(&window)?;
    db.update_user_rule_v2(request).map_err(command_error)
}

#[tauri::command]
pub fn set_user_rule_enabled_v2<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: SetUserRuleEnabledV2Request,
) -> Result<RuleMutationResultV2, String> {
    require_main_window(&window)?;
    db.set_user_rule_enabled_v2(request).map_err(command_error)
}

#[tauri::command]
pub fn delete_user_rule_v2<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: DeleteUserRuleV2Request,
) -> Result<RuleCatalogStateDto, String> {
    require_main_window(&window)?;
    db.delete_user_rule_v2(request).map_err(command_error)
}

#[tauri::command]
pub async fn execute_rules_on_inbox<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    rules: Vec<Rule>,
) -> Result<RuleExecutionSummary, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || db.execute_rules_on_inbox(rules))
        .await
        .map_err(|error| error.to_string())?
        .map_err(command_error)
}

#[tauri::command]
pub async fn execute_rules_for_paths<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    paths: Vec<String>,
    rules: Vec<Rule>,
) -> Result<RuleExecutionSummary, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || db.execute_rules_for_paths(&paths, rules))
        .await
        .map_err(|error| error.to_string())?
        .map_err(command_error)
}

#[tauri::command]
pub async fn execute_rules_for_scope<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    scope: LibraryScope,
    rules: Vec<Rule>,
    mode: Option<RuleExecutionMode>,
) -> Result<RuleExecutionSummary, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    let mode = mode.unwrap_or_default();
    tauri::async_runtime::spawn_blocking(move || {
        db.execute_rules_for_scope_with_mode(&scope, rules, mode)
    })
    .await
    .map_err(|error| error.to_string())?
    .map_err(command_error)
}

#[tauri::command]
pub async fn execute_rules_for_scope_v2<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    request: ExecuteRulesForScopeV2Request,
) -> Result<RuleExecutionResultV2, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || db.execute_rules_for_scope_v2(request))
        .await
        .map_err(|error| error.to_string())?
        .map_err(command_error)
}

/// Runs the same backend-owned rule classifier used by native watcher
/// reconciliation. The legacy renderer watcher may request paths, but it
/// never supplies rules or classification decisions.
#[tauri::command]
pub async fn execute_authoritative_rules_for_paths<R: Runtime>(
    window: WebviewWindow<R>,
    db: State<'_, Database>,
    paths: Vec<String>,
) -> Result<RuleExecutionSummary, String> {
    require_main_window(&window)?;
    let db = db.inner().clone();
    tauri::async_runtime::spawn_blocking(move || db.execute_authoritative_rules_for_paths(&paths))
        .await
        .map_err(|error| error.to_string())?
        .map_err(command_error)
}

fn command_error(error: DbError) -> String {
    error.to_string()
}

fn emit_fs_watcher_warning<R: Runtime>(app: &AppHandle<R>, warning: &WatcherUpsertWarning) {
    if let Err(error) = app.emit(FS_WATCHER_WARNING_EVENT, warning) {
        eprintln!("Failed to emit {FS_WATCHER_WARNING_EVENT}: {error}");
    }
}
