#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

mod startup_upgrade;

use std::io;

#[cfg(feature = "native-qa")]
use std::path::PathBuf;

use fylura_tauri::{
    dedupe::DedupeJobManager,
    global_index::{GlobalIndexCoordinator, ManagedAiWorker},
    settings,
    watcher::{
        reload_file_watcher_for_settings, suspend_file_watcher_for_lifecycle, FileWatcherManager,
    },
    AIClassificationCancellationToken, OperationCancellationToken, RuleProposalGenerationManager,
    ScanJobManager,
};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

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
            fylura_tauri::global_index::windows::service_host::run_index_service_process(),
        );
    }

    #[cfg(all(windows, feature = "native-qa"))]
    if launch_args
        .get(1)
        .is_some_and(|argument| argument == "--global-index-recovery-qa")
    {
        let result = fylura_tauri::global_index::windows::recovery_qa::run(&launch_args[2..]);
        if let Err(error) = result {
            eprintln!("Global Index service recovery QA failed: {error}");
            std::process::exit(1);
        }
        std::process::exit(0);
    }

    #[cfg(feature = "native-qa")]
    if launch_args
        .get(1)
        .is_some_and(|arg| arg == "--schema-upgrade-ui-qa")
    {
        if let Err(error) = startup_upgrade::run_native_qa() {
            eprintln!("Native upgrade QA failed: {error}");
            std::process::exit(1);
        }
        return;
    }

    // Fail before single-instance activation, database opening, migrations, or any scan startup.
    // Native-QA launches must never silently use the installed user's app profile.
    #[cfg(feature = "native-qa")]
    if let Err(error) = fylura_tauri::native_qa_profile_root() {
        eprintln!("native_qa startup_refused: {error}");
        std::process::exit(1);
    }

    // Launchers should pass --native-qa: a production build must reject that QA intent.
    #[cfg(not(feature = "native-qa"))]
    if launch_args.iter().any(|argument| argument == "--native-qa") {
        eprintln!("native_qa startup_refused: binary lacks native-qa feature");
        std::process::exit(1);
    }

    let background_launch = fylura_tauri::app_control::is_background_launch_args(&launch_args);

    let startup_aborted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let abort_on_setup = startup_aborted.clone();
    let application = tauri::Builder::default()
        .manage(fylura_tauri::exit_intent::ExitIntentState::default())
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| {
            let classification =
                fylura_tauri::app_control::classify_second_instance(&args);
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
                fylura_tauri::app_control::SecondInstanceAction::ActivateMain => {
                    if app.try_state::<fylura_tauri::db::Database>().is_none() {
                        eprintln!("ui_runtime single_instance_main_activation deferred=database_startup");
                        return;
                    }
                    match fylura_tauri::app_control::show_main_window(app) {
                        Ok(()) => {
                            let generation = app
                                .state::<
                                    fylura_tauri::app_control::MainWindowLifecycleState,
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
                fylura_tauri::app_control::SecondInstanceAction::IgnoreBackground => {}
                fylura_tauri::app_control::SecondInstanceAction::IgnoreUnsupportedArguments => {}
            }
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name("Fylura")
                .args(["--background"])
                .build(),
        )
        .setup(move |app| {
            qualification_startup_checkpoint("tauri_setup_entered");
            let database_path = fylura_tauri::database_path(app.handle()).map_err(io::Error::other)?;
            let db = match startup_upgrade::open(database_path, background_launch) {
                Ok(db)=>db,
                Err(error)=>{
                    // A deferred background upgrade or an acknowledged native
                    // Exit is a normal startup outcome, not a crash/relaunch.
                    abort_on_setup.store(true,std::sync::atomic::Ordering::Release);
                    eprintln!("Database startup deferred or cancelled: {error}");
                    app.handle().exit(0);
                    return Ok(());
                }
            };
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
            fylura_tauri::scanner::recover_scan_state(&db).map_err(io::Error::other)?;
            fylura_tauri::file_ops::reconcile_pending_operation_journal(&db)
                .map_err(io::Error::other)?;
            db.recover_organization_plans().map_err(io::Error::other)?;
            db.recover_rule_proposals().map_err(io::Error::other)?;
            fylura_tauri::storage_analyzer::reconcile_pending_cleanup_journal(&db)
                .map_err(io::Error::other)?;
            app.manage(db.clone());
            app.manage(fylura_tauri::content::ContentRunManager::default());
            #[cfg(feature = "native-qa")]
            let workspace_data_root = std::env::var_os("ZC_NATIVE_QA_PROFILE_ROOT")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "ZC_NATIVE_QA_PROFILE_ROOT must be an absolute path for native-qa",
                    )
                })?;
            #[cfg(not(feature = "native-qa"))]
            let workspace_data_root = app
                .path()
                .app_data_dir()
                .map_err(io::Error::other)?;
            std::fs::create_dir_all(&workspace_data_root).map_err(io::Error::other)?;
            let thumbnail_cache_dir = workspace_data_root.join("quick-look");
            app.manage(
                fylura_tauri::platform::macos::quick_look::MacThumbnailService::new(
                    thumbnail_cache_dir,
                ),
            );
            let workspace_thumbnail_cache_dir =
                workspace_data_root.join("file-workspace-thumbnails");
            let workspace_native_preview_root =
                workspace_data_root.join("file-workspace-native-preview");
            let workspace_runtime = fylura_tauri::file_workspace::integration::FileWorkspaceRuntimeOwner::new(
                db.clone(),
                app.state::<fylura_tauri::platform::macos::quick_look::MacThumbnailService>()
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
            app.manage(fylura_tauri::analysis::AnalysisRunManager::default());
            if let Err(error) = fylura_tauri::scanner::resume_pending_dedupe_dispatches(
                app.handle().clone(),
                db.clone(),
                dedupe_jobs,
            ) {
                eprintln!("Dedupe dispatch recovery failed (non-fatal): {error}");
            }
            app.manage(OperationCancellationToken::default());
            app.manage(AIClassificationCancellationToken::default());
            app.manage(FileWatcherManager::default());
            app.manage(fylura_tauri::storage_analyzer::CleanupRestoreState::default());
            app.manage(fylura_tauri::app_control::GlobalHotkeyStatusState::default());
            app.manage(fylura_tauri::app_control::SearchWindowLifecycleState::default());
            app.manage(fylura_tauri::app_control::MainWindowReadinessState::default());
            app.manage(fylura_tauri::app_control::MainWindowLifecycleState::default());
            app.manage(fylura_tauri::app_control::MainWindowSessionState::default());
            qualification_startup_checkpoint("core_runtime_owners_ready");
            fylura_tauri::app_control::setup_tray(app).map_err(io::Error::other)?;
            qualification_startup_checkpoint("tray_ready");
            let app_settings = settings::get_app_settings(&db).map_err(io::Error::other)?;
            let launch_at_login = app.autolaunch();
            #[cfg(target_os = "windows")]
            let legacy_autostart_result =
                fylura_tauri::identity_migration::migrate_legacy_windows_autostart(
                    || settings::LaunchAtLoginController::enable(&*launch_at_login),
                    || settings::LaunchAtLoginController::disable(&*launch_at_login),
                );
            #[cfg(target_os = "macos")]
            let legacy_autostart_result =
                fylura_tauri::identity_migration::migrate_legacy_macos_autostart(
                    || settings::LaunchAtLoginController::enable(&*launch_at_login),
                    || settings::LaunchAtLoginController::disable(&*launch_at_login),
                );
            #[cfg(not(any(target_os = "macos", target_os = "windows")))]
            let legacy_autostart_result: Result<bool, String> = Ok(false);
            let app_settings = match legacy_autostart_result {
                Ok(true) => {
                    let mut migrated_settings = app_settings;
                    migrated_settings.launch_at_login = true;
                    if let Err(error) = settings::save_app_settings(&db, &migrated_settings) {
                        eprintln!("Legacy login preference persistence failed (non-fatal): {error}");
                    }
                    migrated_settings
                }
                Ok(false) => match settings::sync_launch_at_login_from_system(
                    &db,
                    &app_settings,
                    &*launch_at_login,
                ) {
                    Ok(synced_settings) => synced_settings,
                    Err(error) => {
                        eprintln!("Launch at login sync failed (non-fatal): {error}");
                        app_settings
                    }
                },
                Err(error) => {
                    eprintln!("Legacy login item migration was safely skipped: {error}");
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
            if let Err(error) = fylura_tauri::app_control::setup_global_search_shortcut(
                app,
                &app_settings.search_hotkey,
            ) {
                eprintln!("Global search hotkey setup failed (non-fatal): {error}");
            }
            qualification_startup_checkpoint("hotkey_setup_complete");
            db.sync_file_library_watcher_roots(&app_settings.default_scan_folders)
                .map_err(io::Error::other)?;
            let watcher_manager = app.state::<FileWatcherManager>();
            if fylura_tauri::watcher::backend_watcher_reconciliation_enabled() {
                if let Err(error) = fylura_tauri::watcher::recover_watcher_reconciliation_state(
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
            let automation = fylura_tauri::db::AutomationTriggerCoordinator::start(
                db.clone(),
                move || { let _ = automation_app.emit("automation-updated", ()); },
            ).map_err(io::Error::other)?;
            app.manage(automation);
            let lifecycle_coordinator = global_index_coordinator.clone();
            let lifecycle_app = app.handle().clone();
            let lifecycle_db = db.clone();
            let lifecycle =
                fylura_tauri::platform::macos::lifecycle::MacLifecycleController::start(
                    move |event| {
                        use fylura_tauri::platform::macos::lifecycle::MacLifecycleEvent;

                        let app_handle = lifecycle_app.clone();
                        let db = lifecycle_db.clone();
                        match event {
                            MacLifecycleEvent::ResourcePolicyChanged => {
                                fylura_tauri::scheduler::WorkScheduler::global()
                                    .notify_resource_policy_changed();
                                Ok(())
                            }
                            MacLifecycleEvent::WillSleep | MacLifecycleEvent::WillUnmount => {
                                app_handle
                                    .state::<fylura_tauri::db::AutomationTriggerCoordinator>()
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
                                    .state::<fylura_tauri::analysis::AnalysisRunManager>()
                                    .cancel_all();
                                app_handle
                                    .state::<OperationCancellationToken>()
                                    .cancel_for_lifecycle();
                                app_handle
                                    .state::<AIClassificationCancellationToken>()
                                    .cancel_for_lifecycle();
                                app_handle
                                    .state::<fylura_tauri::storage_analyzer::CleanupRestoreState>()
                                    .cancel_for_lifecycle()?;
                                db.recover_dedupe_runs().map_err(|error| error.to_string())?;
                                db.recover_analysis_runs().map_err(|error| error.to_string())?;
                                db.recover_content_runs().map_err(|error| error.to_string())?;
                                fylura_tauri::scanner::recover_scan_state(&db)
                                    .map_err(|error| error.to_string())?;
                                Ok(())
                            }
                            MacLifecycleEvent::DidWake
                            | MacLifecycleEvent::DidMount
                            | MacLifecycleEvent::DidUnmount
                            | MacLifecycleEvent::VolumeChanged => {
                                app_handle
                                    .state::<fylura_tauri::db::AutomationTriggerCoordinator>()
                                    .pause();
                                db.recover_dedupe_runs().map_err(|error| error.to_string())?;
                                db.recover_analysis_runs().map_err(|error| error.to_string())?;
                                db.recover_content_runs().map_err(|error| error.to_string())?;
                                fylura_tauri::scanner::recover_scan_state(&db)
                                    .map_err(|error| error.to_string())?;
                                fylura_tauri::watcher::recover_watcher_reconciliation_state(
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
                                fylura_tauri::scanner::resume_pending_dedupe_dispatches(
                                    app_handle.clone(),
                                    db,
                                    app_handle.state::<DedupeJobManager>().inner().clone(),
                                )
                                .map_err(|error| error.to_string())?;
                                lifecycle_coordinator
                                    .resume()
                                    .map_err(|error| error.to_string())?;
                                app_handle
                                    .state::<fylura_tauri::db::AutomationTriggerCoordinator>()
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
            fylura_tauri::scheduler::WorkScheduler::global()
                .set_native_policy_notifications_available(true);
            app.manage(lifecycle);
            #[cfg(all(target_os = "windows", feature = "native-qa"))]
            fylura_tauri::native_resident_lifecycle_qa::install(app.handle()).map_err(io::Error::other)?;
            if !background_launch {
                fylura_tauri::app_control::show_main_window(app.handle())
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
            fylura_tauri::db::init_db,
            fylura_tauri::db::insert_file,
            fylura_tauri::db::remove_files_by_paths,
            fylura_tauri::db::upsert_files_by_paths,
            fylura_tauri::db::search_files,
            fylura_tauri::global_index::commands::search_global_entries,
            fylura_tauri::global_index::commands::get_global_index_status,
            fylura_tauri::global_index::coverage::command::global_index_coverage,
            fylura_tauri::global_index::commands::list_global_index_sources,
            fylura_tauri::global_index::commands::start_global_index,
            fylura_tauri::global_index::commands::pause_global_index,
            fylura_tauri::global_index::commands::resume_global_index,
            fylura_tauri::global_index::commands::rebuild_global_index_source,
            fylura_tauri::global_index::commands::set_global_index_source_enabled,
            fylura_tauri::global_index::commands::authorize_global_index_source,
            fylura_tauri::global_index::commands::open_global_search_result,
            fylura_tauri::global_index::commands::reveal_global_search_result,
            fylura_tauri::global_index::commands::list_managed_scopes,
            fylura_tauri::global_index::commands::add_managed_scope,
            fylura_tauri::global_index::commands::remove_managed_scope,
            fylura_tauri::global_index::commands::update_managed_scope_policy,
            fylura_tauri::global_index::commands::get_ai_management_status,
            fylura_tauri::db::get_paged_files,
            fylura_tauri::db::query_file_library_v2,
            fylura_tauri::db::resolve_file_library_exact_count_v2,
            fylura_tauri::db::get_file_library_detail,
            fylura_tauri::db::file_library_personal_metadata,
            fylura_tauri::db::get_file_library_selection_summary,
            fylura_tauri::db::reveal_file_library_entry,
            fylura_tauri::db::list_user_tags,
            fylura_tauri::db::create_user_tag,
            fylura_tauri::db::update_user_tag,
            fylura_tauri::db::delete_user_tag,
            fylura_tauri::db::mutate_file_user_tags,
            fylura_tauri::db::list_library_saved_views,
            fylura_tauri::db::create_library_saved_view,
            fylura_tauri::db::update_library_saved_view,
            fylura_tauri::db::delete_library_saved_view,
            fylura_tauri::db::list_automation_intents,
            fylura_tauri::db::get_automation_intent,
            fylura_tauri::db::list_automation_runs,
            fylura_tauri::db::create_automation_intent,
            fylura_tauri::db::update_automation_intent,
            fylura_tauri::db::set_automation_intent_enabled,
            fylura_tauri::db::archive_automation_intent,
            fylura_tauri::db::run_automation_intent_manual,
            fylura_tauri::db::create_organization_plan,
            fylura_tauri::db::list_organization_plans,
            fylura_tauri::db::get_organization_plan,
            fylura_tauri::db::query_organization_plan_items,
            fylura_tauri::db::query_organization_plan_groups,
            fylura_tauri::db::query_organization_plan_group_items,
            fylura_tauri::db::update_organization_plan_decisions,
            fylura_tauri::db::update_organization_plan_group_decision,
            fylura_tauri::db::refresh_organization_plan,
            fylura_tauri::db::cancel_organization_plan,
            fylura_tauri::db::delete_organization_plan,
            fylura_tauri::db::analyze_organization_plan_items,
            fylura_tauri::db::get_organization_plan_dry_run,
            fylura_tauri::db::execute_organization_plan,
            fylura_tauri::db::get_operation_previews_for_scope,
            fylura_tauri::db::get_operation_previews_by_file_ids,
            fylura_tauri::db::get_manual_move_operation_preview,
            fylura_tauri::db::get_manual_rename_operation_preview,
            fylura_tauri::db::get_permanent_delete_operation_preview,
            fylura_tauri::db::get_operation_previews_for_selection,
            fylura_tauri::db::get_stats_summary,
            fylura_tauri::db::get_operation_logs,
            fylura_tauri::db::get_rule_catalog_state,
            fylura_tauri::db::list_user_rules_v2,
            fylura_tauri::db::create_user_rule_v2,
            fylura_tauri::db::update_user_rule_v2,
            fylura_tauri::db::set_user_rule_enabled_v2,
            fylura_tauri::db::delete_user_rule_v2,
            fylura_tauri::db::confirm_classification,
            fylura_tauri::db::correct_classification,
            fylura_tauri::db::execute_authoritative_rules_for_paths,
            fylura_tauri::db::execute_rules_for_scope_v2,
            fylura_tauri::content::get_content_scope_policy,
            fylura_tauri::content::get_content_catalog_revision,
            fylura_tauri::content::set_content_scope_policy,
            fylura_tauri::content::preview_content,
            fylura_tauri::content::start_content_run,
            fylura_tauri::content::get_content_run,
            fylura_tauri::content::list_content_runs,
            fylura_tauri::content::get_active_content_run_for_file,
            fylura_tauri::content::cancel_content_run,
            fylura_tauri::content::query_content_run_items,
            fylura_tauri::content::get_content_artifact,
            fylura_tauri::content::query_content_artifacts,
            fylura_tauri::content::rebuild_content_artifact,
            fylura_tauri::content::delete_content_artifact,
            fylura_tauri::content::purge_content_scope,
            fylura_tauri::content::understand_content_artifacts,
            fylura_tauri::rule_proposals::create_rule_proposal,
            fylura_tauri::rule_proposals::regenerate_rule_proposal,
            fylura_tauri::rule_proposals::get_rule_proposal,
            fylura_tauri::rule_proposals::list_rule_proposals,
            fylura_tauri::rule_proposals::cancel_rule_proposal,
            fylura_tauri::rule_proposals::delete_rule_proposal,
            fylura_tauri::rule_proposals::replace_rule_proposal_candidate,
            fylura_tauri::rule_proposals::preview_rule_proposal,
            fylura_tauri::rule_proposals::resolve_rule_proposal_exact_impact,
            fylura_tauri::rule_proposals::apply_rule_proposal,
            fylura_tauri::settings::get_settings,
            fylura_tauri::settings::save_settings,
            fylura_tauri::ai::settings::get_ai_settings,
            fylura_tauri::ai::readiness::get_ai_feature_readiness,
            fylura_tauri::ai::settings::save_ai_settings,
            fylura_tauri::ai::settings::list_ai_provider_presets,
            fylura_tauri::ai::settings::list_ai_models,
            fylura_tauri::ai::settings::test_ai_provider_connection,
            fylura_tauri::ai::trace::list_ai_request_traces,
            fylura_tauri::ai::trace::clear_ai_request_traces,
            fylura_tauri::ai::trace::export_ai_request_traces,
            fylura_tauri::ai::classification::classify_files_with_ai,
            fylura_tauri::ai::classification::classify_selected_files_with_ai,
            fylura_tauri::ai::classification::cancel_ai_classification,
            fylura_tauri::ai::debug::debug_ai_classification_once,
            fylura_tauri::runtime_capabilities::get_runtime_capabilities,
            fylura_tauri::ai::cleanup::analyze_cleanup_candidates_with_ai,
            fylura_tauri::app_control::quit_app,
            fylura_tauri::app_control::enter_background,
            fylura_tauri::app_control::activate_search_result,
            fylura_tauri::app_control::resize_search_window,
            fylura_tauri::app_control::get_search_window_state,
            fylura_tauri::app_control::search_window_ready,
            fylura_tauri::app_control::hide_search_window_command,
            fylura_tauri::app_control::mark_main_window_ready,
            fylura_tauri::app_control::acknowledge_main_window_ready,
            fylura_tauri::app_control::acknowledge_search_navigation,
            fylura_tauri::app_control::get_global_hotkey_status,
            fylura_tauri::app_control::register_global_search_hotkey,
            fylura_tauri::scanner::start_managed_scan,
            fylura_tauri::scanner::cancel_scan_run,
            fylura_tauri::scanner::get_managed_scan_snapshot,
            fylura_tauri::scanner::get_scan_run,
            fylura_tauri::scanner::list_scan_runs,
            fylura_tauri::scanner::list_scan_roots,
            fylura_tauri::scanner::get_scan_root_health,
            fylura_tauri::scanner::retry_interrupted_scan,
            fylura_tauri::scanner::scan_directory,
            fylura_tauri::scanner::create_scan_job_id,
            fylura_tauri::scanner::cancel_scan,
            fylura_tauri::dedupe::cancel_dedupe,
            fylura_tauri::dedupe::start_dedupe_run,
            fylura_tauri::dedupe::retry_dedupe_run,
            fylura_tauri::dedupe::cancel_dedupe_run,
            fylura_tauri::dedupe::get_dedupe_run,
            fylura_tauri::dedupe::list_dedupe_runs,
            fylura_tauri::dedupe::get_active_dedupe_run,
            fylura_tauri::dedupe::list_duplicate_groups,
            fylura_tauri::dedupe::get_duplicate_group,
            fylura_tauri::dedupe::list_duplicate_group_members,
            fylura_tauri::dedupe::get_file_duplicate_membership,
            fylura_tauri::analysis::list_analysis_detectors,
            fylura_tauri::analysis::start_analysis_run,
            fylura_tauri::analysis::cancel_analysis_run,
            fylura_tauri::analysis::retry_analysis_run,
            fylura_tauri::analysis::get_analysis_run,
            fylura_tauri::analysis::get_active_analysis_run,
            fylura_tauri::analysis::list_analysis_runs,
            fylura_tauri::analysis::list_analysis_run_detectors,
            fylura_tauri::analysis::list_analysis_findings,
            fylura_tauri::analysis::get_analysis_finding,
            fylura_tauri::analysis::list_analysis_finding_evidence,
            fylura_tauri::analysis::get_dedupe_authority,
            fylura_tauri::analysis::set_analysis_finding_decision,
            fylura_tauri::analysis::revalidate_analysis_finding,
            fylura_tauri::file_ops::reveal_in_folder,
            fylura_tauri::file_ops::request_macos_thumbnail,
            fylura_tauri::file_ops::cancel_macos_thumbnail,
            fylura_tauri::file_workspace::integration::commands::file_workspace_transfer_capture,
            fylura_tauri::file_workspace::integration::commands::file_workspace_transfer_preview,
            fylura_tauri::file_workspace::integration::commands::file_workspace_transfer_execute,
            fylura_tauri::file_workspace::integration::commands::file_workspace_places,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_open,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_reveal,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_restore,
            fylura_tauri::file_workspace::integration::commands::file_workspace_location_browse,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_start_enumeration,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_next_page,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_cancel_enumeration,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_release_page,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_release_path,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_retain_path,
            fylura_tauri::file_workspace::integration::commands::file_workspace_browse_dispose,
            fylura_tauri::file_workspace::integration::commands::file_workspace_location_list,
            fylura_tauri::file_workspace::integration::commands::file_workspace_change_start,
            fylura_tauri::file_workspace::integration::commands::file_workspace_change_pending,
            fylura_tauri::file_workspace::integration::commands::file_workspace_change_refresh,
            fylura_tauri::file_workspace::integration::commands::file_workspace_change_dispose,
            fylura_tauri::file_workspace::integration::commands::file_workspace_read_eligibility,
            fylura_tauri::file_workspace::integration::commands::file_workspace_thumbnail_request,
            fylura_tauri::file_workspace::integration::commands::file_workspace_thumbnail_cancel,
            fylura_tauri::file_workspace::integration::commands::file_workspace_preview_create,
            fylura_tauri::file_workspace::integration::commands::file_workspace_preview_snapshot,
            fylura_tauri::file_workspace::integration::commands::file_workspace_preview_start,
            fylura_tauri::file_workspace::integration::commands::file_workspace_preview_cancel,
            fylura_tauri::file_workspace::integration::commands::file_workspace_preview_dispose,
            fylura_tauri::file_workspace::integration::commands::file_workspace_preview_switch_source,
            fylura_tauri::file_workspace::integration::commands::file_workspace_preview_asset_request,
            fylura_tauri::file_ops::execute_moves,
            fylura_tauri::file_ops::materialize_provider_preview,
            fylura_tauri::file_ops::restore_moves,
            fylura_tauri::file_ops::resolve_operation_recovery,
            fylura_tauri::file_ops::cancel_operations,
            fylura_tauri::storage_analyzer::start_storage_cleanup_scan,
            fylura_tauri::storage_analyzer::get_storage_cleanup_scan_status,
            fylura_tauri::storage_analyzer::get_storage_cleanup_candidate_page,
            fylura_tauri::storage_analyzer::cancel_storage_cleanup_scan,
            fylura_tauri::storage_analyzer::reveal_storage_candidate,
            fylura_tauri::storage_analyzer::preview_cleanup_candidates,
            fylura_tauri::storage_analyzer::preview_cleanup_operations,
            fylura_tauri::storage_analyzer::move_cleanup_candidates_to_safe_trash,
            fylura_tauri::storage_analyzer::list_cleanup_trash_batches,
            fylura_tauri::storage_analyzer::preview_restore_cleanup_trash,
            fylura_tauri::storage_analyzer::restore_cleanup_trash_items,
            fylura_tauri::storage_analyzer::cancel_cleanup_restore
        ])
        .build(tauri::generate_context!());
    let application = match application {
        Ok(application) => application,
        Err(error) => {
            if startup_aborted.load(std::sync::atomic::Ordering::Acquire) {
                eprintln!("Database startup deferred or cancelled: {error}");
                return;
            }
            panic!("failed to build Fylura: {error}");
        }
    };
    application.run(move |app, event| {
        if startup_aborted.load(std::sync::atomic::Ordering::Acquire) {
            // Setup can run on Ready in the native runtime. Keep this
            // acknowledged exit out of normal resident/activation handlers.
            if matches!(event, tauri::RunEvent::Ready) {
                app.exit(0);
            }
            return;
        }
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
                    fylura_tauri::app_control::trace_resident_lifecycle(
                        app,
                        "exit_requested",
                        &format!("request_seq={sequence} code={code:?}"),
                    );
                }
                let exit_intent = app.state::<fylura_tauri::exit_intent::ExitIntentState>();
                #[cfg(feature = "native-qa")]
                let shutdown_invoked = std::cell::Cell::new(false);
                let action = exit_intent.dispatch_exit_requested(
                    code,
                    || {
                        api.prevent_exit();
                        #[cfg(feature = "native-qa")]
                        fylura_tauri::app_control::trace_resident_lifecycle(
                            app,
                            "exit_action",
                            "action=StayResident prevent_exit=true shutdown=false",
                        );
                    },
                    || {
                        #[cfg(feature = "native-qa")]
                        {
                            shutdown_invoked.set(true);
                            fylura_tauri::app_control::trace_resident_lifecycle(
                                app,
                                "exit_action",
                                "action=Exit prevent_exit=false shutdown=true",
                            );
                        }
                        if let Some(automation) =
                            app.try_state::<fylura_tauri::db::AutomationTriggerCoordinator>()
                        {
                            automation.shutdown();
                            #[cfg(feature = "native-qa")]
                            fylura_tauri::app_control::trace_resident_lifecycle(
                                app,
                                "automation_shutdown",
                                "",
                            );
                        }
                        if let Some(coordinator) = app.try_state::<GlobalIndexCoordinator>() {
                            if let Err(error) = coordinator.shutdown() {
                                eprintln!("Global index shutdown failed (non-fatal): {error}");
                            }
                        }
                        #[cfg(feature = "native-qa")]
                        fylura_tauri::app_control::trace_resident_lifecycle(
                            app,
                            "global_index_shutdown",
                            "",
                        );
                        if let Some(worker) = app.try_state::<ManagedAiWorker>() {
                            worker.shutdown();
                            #[cfg(feature = "native-qa")]
                            fylura_tauri::app_control::trace_resident_lifecycle(
                                app,
                                "managed_ai_shutdown",
                                "",
                            );
                        }
                        if let Some(lifecycle) = app.try_state::<
                                fylura_tauri::platform::macos::lifecycle::MacLifecycleController,
                            >() {
                                lifecycle.stop();
                            }
                    },
                );
                #[cfg(feature = "native-qa")]
                fylura_tauri::app_control::trace_resident_lifecycle(
                    app,
                    if action == fylura_tauri::exit_intent::ExitRequestedAction::StayResident {
                        "exit_prevented"
                    } else {
                        "exit_selected"
                    },
                    &format!(
                        "action={action:?} prevent_exit={} shutdown={}",
                        action == fylura_tauri::exit_intent::ExitRequestedAction::StayResident,
                        shutdown_invoked.get()
                    ),
                );
                let _ = action;
            }
            tauri::RunEvent::Exit => {
                #[cfg(feature = "native-qa")]
                fylura_tauri::app_control::trace_resident_lifecycle(app, "run_event_exit", "");
            }
            _ => {}
        }
    });
}
