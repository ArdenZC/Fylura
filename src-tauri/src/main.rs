#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::io;

use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use zen_canvas_tauri::{
    dedupe::DedupeJobManager,
    global_index::{GlobalIndexCoordinator, ManagedAiWorker},
    open_database, settings,
    watcher::{
        reload_file_watcher_for_settings, suspend_file_watcher_for_lifecycle, FileWatcherManager,
    },
    AIClassificationCancellationToken, OperationCancellationToken, RuleProposalGenerationManager,
    ScanJobManager,
};

#[inline]
fn qualification_startup_checkpoint(name: &str) {
    #[cfg(feature = "native-qa")]
    eprintln!("native_qa startup_checkpoint={name}");

    #[cfg(not(feature = "native-qa"))]
    let _ = name;
}

fn main() {
    qualification_startup_checkpoint("process_main_entered");
    let launch_args = std::env::args().collect::<Vec<_>>();
    #[cfg(windows)]
    if launch_args
        .iter()
        .any(|argument| argument == "--index-service")
    {
        std::process::exit(
            zen_canvas_tauri::global_index::windows::service_host::run_index_service_process(),
        );
    }

    #[cfg(all(windows, feature = "native-qa"))]
    if launch_args
        .get(1)
        .is_some_and(|argument| argument == "--global-index-recovery-qa")
    {
        let result = zen_canvas_tauri::global_index::windows::recovery_qa::run(&launch_args[2..]);
        if let Err(error) = result {
            eprintln!("Global Index service recovery QA failed: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    let background_launch = zen_canvas_tauri::app_control::is_background_launch_args(&launch_args);

    tauri::Builder::default()
        .manage(zen_canvas_tauri::exit_intent::ExitIntentState::default())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let classification =
                zen_canvas_tauri::app_control::classify_second_instance(&args);
            let diagnostic = classification.diagnostic;
            eprintln!(
                "ui_runtime single_instance_received first_pid={} arg_count={} action={:?} background_flag_present={} unsupported_arguments={}",
                std::process::id(),
                diagnostic.arg_count,
                diagnostic.action,
                diagnostic.background_flag_present,
                diagnostic.unsupported_arguments
            );
            match classification.action {
                zen_canvas_tauri::app_control::SecondInstanceAction::ActivateMain => {
                    match zen_canvas_tauri::app_control::show_main_window(app) {
                        Ok(()) => {
                            let generation = app
                                .state::<
                                    zen_canvas_tauri::app_control::MainWindowLifecycleState,
                                >()
                                .latest_generation()
                                .map_or_else(
                                    |_| "unavailable".to_string(),
                                    |generation| generation.to_string(),
                                );
                            eprintln!(
                                "ui_runtime single_instance_main_activation result=ok generation={generation} webview_count={}",
                                app.webview_windows().len()
                            );
                        }
                        Err(_) => eprintln!(
                            "ui_runtime single_instance_main_activation result=error error=main_window_activation_failed"
                        ),
                    }
                }
                zen_canvas_tauri::app_control::SecondInstanceAction::IgnoreBackground => {}
                zen_canvas_tauri::app_control::SecondInstanceAction::IgnoreUnsupportedArguments => {}
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--background"]),
        ))
        .setup(move |app| {
            qualification_startup_checkpoint("tauri_setup_entered");
            let db = open_database(app.handle()).map_err(io::Error::other)?;
            qualification_startup_checkpoint("database_ready");
            db.recover_dedupe_runs().map_err(io::Error::other)?;
            db.recover_analysis_runs().map_err(io::Error::other)?;
            db.recover_content_runs().map_err(io::Error::other)?;
            if let Err(error) = db.prune_content_artifacts() {
                eprintln!("Content retention prune skipped: {error}");
            }
            if let Err(error) = db.prune_analysis_artifacts() {
                eprintln!("Analysis retention prune skipped: {error}");
            }
            if let Err(error) = db.prune_dedupe_artifacts() {
                eprintln!("Dedupe retention prune skipped: {error}");
            }
            zen_canvas_tauri::scanner::recover_scan_state(&db).map_err(io::Error::other)?;
            zen_canvas_tauri::file_ops::reconcile_pending_operation_journal(&db)
                .map_err(io::Error::other)?;
            db.recover_organization_plans().map_err(io::Error::other)?;
            db.recover_rule_proposals().map_err(io::Error::other)?;
            zen_canvas_tauri::storage_analyzer::reconcile_pending_cleanup_journal(&db)
                .map_err(io::Error::other)?;
            app.manage(db.clone());
            app.manage(zen_canvas_tauri::content::ContentRunManager::default());
            let thumbnail_cache_dir = app
                .path()
                .app_data_dir()
                .map_err(io::Error::other)?
                .join("quick-look");
            app.manage(
                zen_canvas_tauri::platform::macos::quick_look::MacThumbnailService::new(
                    thumbnail_cache_dir,
                ),
            );
            let workspace_thumbnail_cache_dir = app
                .path()
                .app_data_dir()
                .map_err(io::Error::other)?
                .join("file-workspace-thumbnails");
            let workspace_native_preview_root = app
                .path()
                .app_data_dir()
                .map_err(io::Error::other)?
                .join("file-workspace-native-preview");
            let workspace_runtime = zen_canvas_tauri::file_workspace::integration::FileWorkspaceRuntimeOwner::new(
                db.clone(),
                app.state::<zen_canvas_tauri::platform::macos::quick_look::MacThumbnailService>()
                    .inner()
                    .clone(),
                workspace_thumbnail_cache_dir,
                workspace_native_preview_root,
            );
            app.manage(workspace_runtime);
            let global_index_coordinator = GlobalIndexCoordinator::new(db.clone());
            app.manage(global_index_coordinator.clone());
            if let Err(error) = global_index_coordinator.start() {
                eprintln!("Global index startup failed (non-fatal): {error}");
            }
            let managed_ai_worker = ManagedAiWorker::start(db.clone());
            app.manage(managed_ai_worker);
            app.manage(ScanJobManager::default());
            let dedupe_jobs = DedupeJobManager::default();
            app.manage(dedupe_jobs.clone());
            app.manage(zen_canvas_tauri::analysis::AnalysisRunManager::default());
            if let Err(error) = zen_canvas_tauri::scanner::resume_pending_dedupe_dispatches(
                app.handle().clone(),
                db.clone(),
                dedupe_jobs,
            ) {
                eprintln!("Dedupe dispatch recovery failed (non-fatal): {error}");
            }
            app.manage(OperationCancellationToken::default());
            app.manage(AIClassificationCancellationToken::default());
            app.manage(FileWatcherManager::default());
            app.manage(zen_canvas_tauri::storage_analyzer::CleanupRestoreState::default());
            app.manage(zen_canvas_tauri::app_control::GlobalHotkeyStatusState::default());
            app.manage(zen_canvas_tauri::app_control::SearchWindowLifecycleState::default());
            app.manage(zen_canvas_tauri::app_control::MainWindowReadinessState::default());
            app.manage(zen_canvas_tauri::app_control::MainWindowLifecycleState::default());
            app.manage(zen_canvas_tauri::app_control::MainWindowSessionState::default());
            qualification_startup_checkpoint("core_runtime_owners_ready");
            zen_canvas_tauri::app_control::setup_tray(app).map_err(io::Error::other)?;
            qualification_startup_checkpoint("tray_ready");
            let app_settings = settings::get_app_settings(&db).map_err(io::Error::other)?;
            let launch_at_login = app.autolaunch();
            let app_settings = match settings::sync_launch_at_login_from_system(
                &db,
                &app_settings,
                &*launch_at_login,
            ) {
                Ok(synced_settings) => synced_settings,
                Err(error) => {
                    eprintln!("Launch at login sync failed (non-fatal): {error}");
                    app_settings
                }
            };
            qualification_startup_checkpoint("autostart_sync_complete");
            db.prune_operation_logs(app_settings.restore_retention_days)
                .map_err(io::Error::other)?;
            if let Err(error) = db.prune_organization_plans() {
                eprintln!("Organization plan retention prune skipped: {error}");
            }
            if let Err(error) = db.prune_rule_proposals() {
                eprintln!("Rule proposal retention prune skipped: {error}");
            }
            app.manage(RuleProposalGenerationManager::default());
            if let Err(error) = zen_canvas_tauri::app_control::setup_global_search_shortcut(
                app,
                &app_settings.search_hotkey,
            ) {
                eprintln!("Global search hotkey setup failed (non-fatal): {error}");
            }
            qualification_startup_checkpoint("hotkey_setup_complete");
            db.sync_file_library_watcher_roots(&app_settings.default_scan_folders)
                .map_err(io::Error::other)?;
            let watcher_manager = app.state::<FileWatcherManager>();
            if zen_canvas_tauri::watcher::backend_watcher_reconciliation_enabled() {
                if let Err(error) = zen_canvas_tauri::watcher::recover_watcher_reconciliation_state(
                    app.handle().clone(),
                    db.clone(),
                ) {
                    eprintln!("Watcher reconciliation recovery failed (non-fatal): {error}");
                }
            }
            if let Err(error) = reload_file_watcher_for_settings(
                app.handle().clone(),
                &watcher_manager,
                &db,
                app.state::<ScanJobManager>().inner(),
                app.state::<DedupeJobManager>().inner(),
                &app_settings,
            ) {
                eprintln!("File watcher init failed (non-fatal): {error}");
            }
            qualification_startup_checkpoint("watcher_setup_complete");

            let automation_app = app.handle().clone();
            let automation = zen_canvas_tauri::db::AutomationTriggerCoordinator::start(
                db.clone(),
                move || { let _ = automation_app.emit("automation-updated", ()); },
            ).map_err(io::Error::other)?;
            app.manage(automation);
            let lifecycle_coordinator = global_index_coordinator.clone();
            let lifecycle_app = app.handle().clone();
            let lifecycle_db = db.clone();
            let lifecycle =
                zen_canvas_tauri::platform::macos::lifecycle::MacLifecycleController::start(
                    move |event| {
                        use zen_canvas_tauri::platform::macos::lifecycle::MacLifecycleEvent;

                        let app_handle = lifecycle_app.clone();
                        let db = lifecycle_db.clone();
                        match event {
                            MacLifecycleEvent::ResourcePolicyChanged => {
                                zen_canvas_tauri::scheduler::WorkScheduler::global()
                                    .notify_resource_policy_changed();
                                Ok(())
                            }
                            MacLifecycleEvent::WillSleep | MacLifecycleEvent::WillUnmount => {
                                app_handle
                                    .state::<zen_canvas_tauri::db::AutomationTriggerCoordinator>()
                                    .pause();
                                lifecycle_coordinator
                                    .pause()
                                    .map_err(|error| error.to_string())?;
                                let watcher = app_handle.state::<FileWatcherManager>();
                                suspend_file_watcher_for_lifecycle(
                                    app_handle.clone(),
                                    watcher.inner(),
                                    &db,
                                    app_handle.state::<ScanJobManager>().inner(),
                                    app_handle.state::<DedupeJobManager>().inner(),
                                )?;
                                app_handle.state::<ScanJobManager>().cancel_all();
                                app_handle.state::<DedupeJobManager>().cancel_all();
                                app_handle
                                    .state::<zen_canvas_tauri::analysis::AnalysisRunManager>()
                                    .cancel_all();
                                app_handle
                                    .state::<OperationCancellationToken>()
                                    .cancel_for_lifecycle();
                                app_handle
                                    .state::<AIClassificationCancellationToken>()
                                    .cancel_for_lifecycle();
                                app_handle
                                    .state::<zen_canvas_tauri::storage_analyzer::CleanupRestoreState>()
                                    .cancel_for_lifecycle()?;
                                db.recover_dedupe_runs().map_err(|error| error.to_string())?;
                                db.recover_analysis_runs().map_err(|error| error.to_string())?;
                                db.recover_content_runs().map_err(|error| error.to_string())?;
                                zen_canvas_tauri::scanner::recover_scan_state(&db)
                                    .map_err(|error| error.to_string())?;
                                Ok(())
                            }
                            MacLifecycleEvent::DidWake
                            | MacLifecycleEvent::DidMount
                            | MacLifecycleEvent::DidUnmount
                            | MacLifecycleEvent::VolumeChanged => {
                                app_handle
                                    .state::<zen_canvas_tauri::db::AutomationTriggerCoordinator>()
                                    .pause();
                                db.recover_dedupe_runs().map_err(|error| error.to_string())?;
                                db.recover_analysis_runs().map_err(|error| error.to_string())?;
                                db.recover_content_runs().map_err(|error| error.to_string())?;
                                zen_canvas_tauri::scanner::recover_scan_state(&db)
                                    .map_err(|error| error.to_string())?;
                                zen_canvas_tauri::watcher::recover_watcher_reconciliation_state(
                                    app_handle.clone(),
                                    db.clone(),
                                )?;
                                let settings = settings::get_app_settings(&db)
                                    .map_err(|error| error.to_string())?;
                                reload_file_watcher_for_settings(
                                    app_handle.clone(),
                                    app_handle.state::<FileWatcherManager>().inner(),
                                    &db,
                                    app_handle.state::<ScanJobManager>().inner(),
                                    app_handle.state::<DedupeJobManager>().inner(),
                                    &settings,
                                )?;
                                zen_canvas_tauri::scanner::resume_pending_dedupe_dispatches(
                                    app_handle.clone(),
                                    db,
                                    app_handle.state::<DedupeJobManager>().inner().clone(),
                                )
                                .map_err(|error| error.to_string())?;
                                lifecycle_coordinator
                                    .resume()
                                    .map_err(|error| error.to_string())?;
                                app_handle
                                    .state::<zen_canvas_tauri::db::AutomationTriggerCoordinator>()
                                    .resume();
                                Ok(())
                            }
                        }
                    },
                )
                .map_err(|error| error.to_string())?;
            #[cfg(target_os = "macos")]
            qualification_startup_checkpoint("mac_lifecycle_ready");
            #[cfg(target_os = "macos")]
            zen_canvas_tauri::scheduler::WorkScheduler::global()
                .set_native_policy_notifications_available(true);
            app.manage(lifecycle);
            #[cfg(all(target_os = "windows", feature = "native-qa"))]
            zen_canvas_tauri::native_resident_lifecycle_qa::install(app.handle()).map_err(io::Error::other)?;
            if !background_launch {
                zen_canvas_tauri::app_control::show_main_window(app.handle())
                    .map_err(io::Error::other)?;
            }
            eprintln!(
                "ui_runtime startup_mode={} webview_count={} labels={}",
                if background_launch { "background" } else { "manual" },
                app.webview_windows().len(),
                app.webview_windows()
                    .keys()
                    .map(String::as_str)
                    .collect::<Vec<_>>()
                    .join(",")
            );
            qualification_startup_checkpoint("setup_complete");
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            zen_canvas_tauri::db::init_db,
            zen_canvas_tauri::db::insert_file,
            zen_canvas_tauri::db::remove_files_by_paths,
            zen_canvas_tauri::db::upsert_files_by_paths,
            zen_canvas_tauri::db::search_files,
            zen_canvas_tauri::global_index::commands::search_global_entries,
            zen_canvas_tauri::global_index::commands::get_global_index_status,
            zen_canvas_tauri::global_index::commands::list_global_index_sources,
            zen_canvas_tauri::global_index::commands::start_global_index,
            zen_canvas_tauri::global_index::commands::pause_global_index,
            zen_canvas_tauri::global_index::commands::resume_global_index,
            zen_canvas_tauri::global_index::commands::rebuild_global_index_source,
            zen_canvas_tauri::global_index::commands::set_global_index_source_enabled,
            zen_canvas_tauri::global_index::commands::open_global_search_result,
            zen_canvas_tauri::global_index::commands::reveal_global_search_result,
            zen_canvas_tauri::global_index::commands::list_managed_scopes,
            zen_canvas_tauri::global_index::commands::add_managed_scope,
            zen_canvas_tauri::global_index::commands::remove_managed_scope,
            zen_canvas_tauri::global_index::commands::update_managed_scope_policy,
            zen_canvas_tauri::global_index::commands::get_ai_management_status,
            zen_canvas_tauri::db::get_paged_files,
            zen_canvas_tauri::db::query_file_library_v2,
            zen_canvas_tauri::db::resolve_file_library_exact_count_v2,
            zen_canvas_tauri::db::get_file_library_detail,
            zen_canvas_tauri::db::get_file_library_selection_summary,
            zen_canvas_tauri::db::reveal_file_library_entry,
            zen_canvas_tauri::db::list_user_tags,
            zen_canvas_tauri::db::create_user_tag,
            zen_canvas_tauri::db::update_user_tag,
            zen_canvas_tauri::db::delete_user_tag,
            zen_canvas_tauri::db::mutate_file_user_tags,
            zen_canvas_tauri::db::list_library_saved_views,
            zen_canvas_tauri::db::create_library_saved_view,
            zen_canvas_tauri::db::update_library_saved_view,
            zen_canvas_tauri::db::delete_library_saved_view,
            zen_canvas_tauri::db::list_automation_intents,
            zen_canvas_tauri::db::get_automation_intent,
            zen_canvas_tauri::db::list_automation_runs,
            zen_canvas_tauri::db::create_automation_intent,
            zen_canvas_tauri::db::update_automation_intent,
            zen_canvas_tauri::db::set_automation_intent_enabled,
            zen_canvas_tauri::db::archive_automation_intent,
            zen_canvas_tauri::db::run_automation_intent_manual,
            zen_canvas_tauri::db::create_organization_plan,
            zen_canvas_tauri::db::list_organization_plans,
            zen_canvas_tauri::db::get_organization_plan,
            zen_canvas_tauri::db::query_organization_plan_items,
            zen_canvas_tauri::db::query_organization_plan_groups,
            zen_canvas_tauri::db::query_organization_plan_group_items,
            zen_canvas_tauri::db::update_organization_plan_decisions,
            zen_canvas_tauri::db::update_organization_plan_group_decision,
            zen_canvas_tauri::db::refresh_organization_plan,
            zen_canvas_tauri::db::cancel_organization_plan,
            zen_canvas_tauri::db::delete_organization_plan,
            zen_canvas_tauri::db::analyze_organization_plan_items,
            zen_canvas_tauri::db::get_organization_plan_dry_run,
            zen_canvas_tauri::db::execute_organization_plan,
            zen_canvas_tauri::db::get_operation_previews_for_scope,
            zen_canvas_tauri::db::get_operation_previews_by_file_ids,
            zen_canvas_tauri::db::get_permanent_delete_operation_preview,
            zen_canvas_tauri::db::get_operation_previews_for_selection,
            zen_canvas_tauri::db::get_stats_summary,
            zen_canvas_tauri::db::get_operation_logs,
            zen_canvas_tauri::db::get_rule_catalog_state,
            zen_canvas_tauri::db::list_user_rules_v2,
            zen_canvas_tauri::db::create_user_rule_v2,
            zen_canvas_tauri::db::update_user_rule_v2,
            zen_canvas_tauri::db::set_user_rule_enabled_v2,
            zen_canvas_tauri::db::delete_user_rule_v2,
            zen_canvas_tauri::db::confirm_classification,
            zen_canvas_tauri::db::correct_classification,
            zen_canvas_tauri::db::execute_authoritative_rules_for_paths,
            zen_canvas_tauri::db::execute_rules_for_scope_v2,
            zen_canvas_tauri::content::get_content_scope_policy,
            zen_canvas_tauri::content::get_content_catalog_revision,
            zen_canvas_tauri::content::set_content_scope_policy,
            zen_canvas_tauri::content::preview_content,
            zen_canvas_tauri::content::start_content_run,
            zen_canvas_tauri::content::get_content_run,
            zen_canvas_tauri::content::list_content_runs,
            zen_canvas_tauri::content::get_active_content_run_for_file,
            zen_canvas_tauri::content::cancel_content_run,
            zen_canvas_tauri::content::query_content_run_items,
            zen_canvas_tauri::content::get_content_artifact,
            zen_canvas_tauri::content::query_content_artifacts,
            zen_canvas_tauri::content::rebuild_content_artifact,
            zen_canvas_tauri::content::delete_content_artifact,
            zen_canvas_tauri::content::purge_content_scope,
            zen_canvas_tauri::content::understand_content_artifacts,
            zen_canvas_tauri::rule_proposals::create_rule_proposal,
            zen_canvas_tauri::rule_proposals::regenerate_rule_proposal,
            zen_canvas_tauri::rule_proposals::get_rule_proposal,
            zen_canvas_tauri::rule_proposals::list_rule_proposals,
            zen_canvas_tauri::rule_proposals::cancel_rule_proposal,
            zen_canvas_tauri::rule_proposals::delete_rule_proposal,
            zen_canvas_tauri::rule_proposals::replace_rule_proposal_candidate,
            zen_canvas_tauri::rule_proposals::preview_rule_proposal,
            zen_canvas_tauri::rule_proposals::resolve_rule_proposal_exact_impact,
            zen_canvas_tauri::rule_proposals::apply_rule_proposal,
            zen_canvas_tauri::settings::get_settings,
            zen_canvas_tauri::settings::save_settings,
            zen_canvas_tauri::ai::settings::get_ai_settings,
            zen_canvas_tauri::ai::readiness::get_ai_feature_readiness,
            zen_canvas_tauri::ai::settings::save_ai_settings,
            zen_canvas_tauri::ai::settings::list_ai_provider_presets,
            zen_canvas_tauri::ai::settings::list_ai_models,
            zen_canvas_tauri::ai::settings::test_ai_provider_connection,
            zen_canvas_tauri::ai::trace::list_ai_request_traces,
            zen_canvas_tauri::ai::trace::clear_ai_request_traces,
            zen_canvas_tauri::ai::trace::export_ai_request_traces,
            zen_canvas_tauri::ai::classification::classify_files_with_ai,
            zen_canvas_tauri::ai::classification::classify_selected_files_with_ai,
            zen_canvas_tauri::ai::classification::cancel_ai_classification,
            zen_canvas_tauri::ai::debug::debug_ai_classification_once,
            zen_canvas_tauri::runtime_capabilities::get_runtime_capabilities,
            zen_canvas_tauri::ai::cleanup::analyze_cleanup_candidates_with_ai,
            zen_canvas_tauri::app_control::quit_app,
            zen_canvas_tauri::app_control::enter_background,
            zen_canvas_tauri::app_control::activate_search_result,
            zen_canvas_tauri::app_control::resize_search_window,
            zen_canvas_tauri::app_control::get_search_window_state,
            zen_canvas_tauri::app_control::search_window_ready,
            zen_canvas_tauri::app_control::hide_search_window_command,
            zen_canvas_tauri::app_control::mark_main_window_ready,
            zen_canvas_tauri::app_control::acknowledge_main_window_ready,
            zen_canvas_tauri::app_control::acknowledge_search_navigation,
            zen_canvas_tauri::app_control::get_global_hotkey_status,
            zen_canvas_tauri::app_control::register_global_search_hotkey,
            zen_canvas_tauri::scanner::start_managed_scan,
            zen_canvas_tauri::scanner::cancel_scan_run,
            zen_canvas_tauri::scanner::get_managed_scan_snapshot,
            zen_canvas_tauri::scanner::get_scan_run,
            zen_canvas_tauri::scanner::list_scan_runs,
            zen_canvas_tauri::scanner::list_scan_roots,
            zen_canvas_tauri::scanner::get_scan_root_health,
            zen_canvas_tauri::scanner::retry_interrupted_scan,
            zen_canvas_tauri::scanner::scan_directory,
            zen_canvas_tauri::scanner::create_scan_job_id,
            zen_canvas_tauri::scanner::cancel_scan,
            zen_canvas_tauri::dedupe::cancel_dedupe,
            zen_canvas_tauri::dedupe::start_dedupe_run,
            zen_canvas_tauri::dedupe::retry_dedupe_run,
            zen_canvas_tauri::dedupe::cancel_dedupe_run,
            zen_canvas_tauri::dedupe::get_dedupe_run,
            zen_canvas_tauri::dedupe::list_dedupe_runs,
            zen_canvas_tauri::dedupe::get_active_dedupe_run,
            zen_canvas_tauri::dedupe::list_duplicate_groups,
            zen_canvas_tauri::dedupe::get_duplicate_group,
            zen_canvas_tauri::dedupe::list_duplicate_group_members,
            zen_canvas_tauri::dedupe::get_file_duplicate_membership,
            zen_canvas_tauri::analysis::list_analysis_detectors,
            zen_canvas_tauri::analysis::start_analysis_run,
            zen_canvas_tauri::analysis::cancel_analysis_run,
            zen_canvas_tauri::analysis::retry_analysis_run,
            zen_canvas_tauri::analysis::get_analysis_run,
            zen_canvas_tauri::analysis::get_active_analysis_run,
            zen_canvas_tauri::analysis::list_analysis_runs,
            zen_canvas_tauri::analysis::list_analysis_run_detectors,
            zen_canvas_tauri::analysis::list_analysis_findings,
            zen_canvas_tauri::analysis::get_analysis_finding,
            zen_canvas_tauri::analysis::list_analysis_finding_evidence,
            zen_canvas_tauri::analysis::get_dedupe_authority,
            zen_canvas_tauri::analysis::set_analysis_finding_decision,
            zen_canvas_tauri::analysis::revalidate_analysis_finding,
            zen_canvas_tauri::file_ops::reveal_in_folder,
            zen_canvas_tauri::file_ops::request_macos_thumbnail,
            zen_canvas_tauri::file_ops::cancel_macos_thumbnail,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_open,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_restore,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_location_browse,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_start_enumeration,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_next_page,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_cancel_enumeration,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_release_page,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_release_path,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_retain_path,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_browse_dispose,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_location_list,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_change_start,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_change_pending,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_change_refresh,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_change_dispose,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_read_eligibility,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_thumbnail_request,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_thumbnail_cancel,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_preview_create,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_preview_snapshot,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_preview_start,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_preview_cancel,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_preview_dispose,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_preview_switch_source,
            zen_canvas_tauri::file_workspace::integration::commands::file_workspace_preview_asset_request,
            zen_canvas_tauri::file_ops::execute_moves,
            zen_canvas_tauri::file_ops::materialize_provider_preview,
            zen_canvas_tauri::file_ops::restore_moves,
            zen_canvas_tauri::file_ops::resolve_operation_recovery,
            zen_canvas_tauri::file_ops::cancel_operations,
            zen_canvas_tauri::storage_analyzer::start_storage_cleanup_scan,
            zen_canvas_tauri::storage_analyzer::get_storage_cleanup_scan_status,
            zen_canvas_tauri::storage_analyzer::get_storage_cleanup_candidate_page,
            zen_canvas_tauri::storage_analyzer::cancel_storage_cleanup_scan,
            zen_canvas_tauri::storage_analyzer::reveal_storage_candidate,
            zen_canvas_tauri::storage_analyzer::preview_cleanup_candidates,
            zen_canvas_tauri::storage_analyzer::preview_cleanup_operations,
            zen_canvas_tauri::storage_analyzer::move_cleanup_candidates_to_safe_trash,
            zen_canvas_tauri::storage_analyzer::list_cleanup_trash_batches,
            zen_canvas_tauri::storage_analyzer::preview_restore_cleanup_trash,
            zen_canvas_tauri::storage_analyzer::restore_cleanup_trash_items,
            zen_canvas_tauri::storage_analyzer::cancel_cleanup_restore
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Zen Canvas")
        .run(|app, event| {
            match event {
                tauri::RunEvent::Resumed => {
                    if let Some(coordinator) = app.try_state::<GlobalIndexCoordinator>() {
                        coordinator.notify_runtime_resume();
                    }
                }
                tauri::RunEvent::ExitRequested { code, api, .. } => {
                    #[cfg(feature = "native-qa")]
                    {
                        use std::sync::atomic::{AtomicUsize, Ordering};
                        static REQUESTS: AtomicUsize = AtomicUsize::new(0);
                        let sequence = REQUESTS.fetch_add(1, Ordering::Relaxed) + 1;
                        zen_canvas_tauri::app_control::trace_resident_lifecycle(app, "exit_requested", &format!("request_seq={sequence} code={code:?}"));
                    }
                    let exit_intent = app.state::<zen_canvas_tauri::exit_intent::ExitIntentState>();
                    #[cfg(feature = "native-qa")]
                    let shutdown_invoked = std::cell::Cell::new(false);
                    let action = exit_intent.dispatch_exit_requested(
                        code,
                        || {
                            api.prevent_exit();
                            #[cfg(feature = "native-qa")]
                            zen_canvas_tauri::app_control::trace_resident_lifecycle(app, "exit_action", "action=StayResident prevent_exit=true shutdown=false");
                        },
                        || {
                            #[cfg(feature = "native-qa")]
                            {
                                shutdown_invoked.set(true);
                                zen_canvas_tauri::app_control::trace_resident_lifecycle(app, "exit_action", "action=Exit prevent_exit=false shutdown=true");
                            }
                            if let Some(automation) = app.try_state::<
                                zen_canvas_tauri::db::AutomationTriggerCoordinator,
                            >() {
                                automation.shutdown();
                                #[cfg(feature = "native-qa")]
                                zen_canvas_tauri::app_control::trace_resident_lifecycle(app, "automation_shutdown", "");
                            }
                            if let Some(coordinator) = app.try_state::<GlobalIndexCoordinator>() {
                                if let Err(error) = coordinator.shutdown() {
                                    eprintln!("Global index shutdown failed (non-fatal): {error}");
                                }
                            }
                            #[cfg(feature = "native-qa")]
                            zen_canvas_tauri::app_control::trace_resident_lifecycle(app, "global_index_shutdown", "");
                            if let Some(worker) = app.try_state::<ManagedAiWorker>() {
                                worker.shutdown();
                                #[cfg(feature = "native-qa")]
                                zen_canvas_tauri::app_control::trace_resident_lifecycle(app, "managed_ai_shutdown", "");
                            }
                            if let Some(lifecycle) = app.try_state::<
                                zen_canvas_tauri::platform::macos::lifecycle::MacLifecycleController,
                            >() {
                                lifecycle.stop();
                            }
                        },
                    );
                    #[cfg(feature = "native-qa")]
                    zen_canvas_tauri::app_control::trace_resident_lifecycle(app, if action == zen_canvas_tauri::exit_intent::ExitRequestedAction::StayResident { "exit_prevented" } else { "exit_selected" }, &format!("action={action:?} prevent_exit={} shutdown={}", action == zen_canvas_tauri::exit_intent::ExitRequestedAction::StayResident, shutdown_invoked.get()));
                    let _ = action;
                }
                tauri::RunEvent::Exit => {
                    #[cfg(feature = "native-qa")]
                    zen_canvas_tauri::app_control::trace_resident_lifecycle(app, "run_event_exit", "");
                }
                _ => {}
            }
        });
}
