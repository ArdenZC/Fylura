use serde::{Deserialize, Serialize};
use std::{
    sync::{Condvar, Mutex},
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager, Runtime, State, WebviewWindow};

use crate::{
    exit_intent::ExitIntentState, settings::DEFAULT_SEARCH_HOTKEY, window_auth::require_main_window,
};

#[cfg(feature = "desktop-runtime")]
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, Emitter, LogicalSize, Size, WebviewUrl, WebviewWindowBuilder, WindowEvent,
};
#[cfg(feature = "desktop-runtime")]
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[cfg(feature = "desktop-runtime")]
const TRAY_SHOW_MAIN_WINDOW_ID: &str = "show-main-window";
#[cfg(feature = "desktop-runtime")]
const TRAY_QUIT_APP_ID: &str = "quit-app";
#[cfg(feature = "desktop-runtime")]
const TRAY_ICON_BYTES: &[u8] = include_bytes!("../../build/icon.png");
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_LABEL: &str = "main";
const SEARCH_WINDOW_LABEL: &str = "search";
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_CLOSE_REQUESTED_EVENT: &str = "main-window-close-requested";
const SEARCH_WINDOW_URL: &str = "index.html?mode=search";
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_WIDTH: f64 = 1280.0;
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_HEIGHT: f64 = 860.0;
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_MIN_WIDTH: f64 = 980.0;
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_MIN_HEIGHT: f64 = 680.0;
#[cfg(feature = "desktop-runtime")]
const SEARCH_WINDOW_WIDTH: f64 = 820.0;
#[cfg(feature = "desktop-runtime")]
const SEARCH_WINDOW_COLLAPSED_HEIGHT: f64 = 160.0;
#[cfg(feature = "desktop-runtime")]
const SEARCH_WINDOW_EXPANDED_HEIGHT: f64 = 660.0;
#[cfg(feature = "desktop-runtime")]
const SEARCH_NAVIGATE_EVENT: &str = "search-navigate";
#[cfg(feature = "desktop-runtime")]
const GLOBAL_HOTKEY_REGISTRATION_FAILED_EVENT: &str = "global-hotkey-registration-failed";
#[cfg(feature = "desktop-runtime")]
const SEARCH_WINDOW_STATE_EVENT: &str = "search-window-state";
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_READY_REQUEST_EVENT: &str = "search-main-ready-request";
#[cfg(feature = "desktop-runtime")]
const MAIN_WINDOW_READY_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchNavigatePayload {
    pub view: SearchView,
    pub file_id: Option<String>,
    pub nonce: u64,
    pub generation: u64,
    pub session_id: Option<u64>,
    pub revision: Option<u64>,
    pub settings_target: Option<SearchSettingsTarget>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SearchView {
    #[default]
    Scanner,
    Cleanup,
    Organize,
    Library,
    Preview,
    #[serde(alias = "rules")]
    Automation,
    Restore,
    Settings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecondInstanceAction {
    ActivateMain,
    IgnoreBackground,
    IgnoreUnsupportedArguments,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondInstanceDiagnostic {
    pub arg_count: usize,
    pub action: SecondInstanceAction,
    pub background_flag_present: bool,
    pub unsupported_arguments: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecondInstanceClassification {
    pub action: SecondInstanceAction,
    pub diagnostic: SecondInstanceDiagnostic,
}

/// The official single-instance plugin forwards the second process argv, including argv[0].
/// Only the ordinary no-argument launch activates Main; the explicit autostart flag stays
/// resident, and unrecognized command-line payloads never gain a new routing capability.
pub fn second_instance_action(args: &[String]) -> SecondInstanceAction {
    match args.get(1..).unwrap_or_default() {
        [] => SecondInstanceAction::ActivateMain,
        [argument] if argument == "--background" => SecondInstanceAction::IgnoreBackground,
        _ => SecondInstanceAction::IgnoreUnsupportedArguments,
    }
}

/// Classify a second-instance payload once and retain only safe diagnostic
/// facts. Argument values and the process working directory are never copied
/// into the diagnostic record.
pub fn classify_second_instance(args: &[String]) -> SecondInstanceClassification {
    let action = second_instance_action(args);
    let forwarded_args = args.get(1..).unwrap_or_default();
    let diagnostic = SecondInstanceDiagnostic {
        arg_count: forwarded_args.len(),
        action,
        background_flag_present: forwarded_args
            .iter()
            .any(|argument| argument == "--background"),
        unsupported_arguments: action == SecondInstanceAction::IgnoreUnsupportedArguments,
    };
    SecondInstanceClassification { action, diagnostic }
}

/// Background startup is enabled only by the exact supported CLI shape used by autostart.
pub fn is_background_launch_args(args: &[String]) -> bool {
    matches!(args.get(1..).unwrap_or_default(), [argument] if argument == "--background")
}

impl SearchView {
    #[cfg(feature = "desktop-runtime")]
    fn as_query_value(self) -> &'static str {
        match self {
            Self::Scanner => "scanner",
            Self::Cleanup => "cleanup",
            Self::Organize => "organize",
            Self::Library => "library",
            Self::Preview => "preview",
            Self::Automation => "automation",
            Self::Restore => "restore",
            Self::Settings => "settings",
        }
    }
}

#[cfg(feature = "desktop-runtime")]
fn main_window_restore_url(generation: u64, last_view: SearchView) -> String {
    format!(
        "index.html?mainGeneration={generation}&view={}",
        last_view.as_query_value()
    )
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SearchSettingsTarget {
    SearchScope,
    GlobalIndex,
    Appearance,
    Ai,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalHotkeyErrorPayload {
    pub message: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GlobalHotkeyStatus {
    pub requested_accelerator: String,
    pub effective_accelerator: Option<String>,
    pub registered: bool,
    pub error: Option<String>,
    pub revision: u64,
}

#[cfg(any(feature = "desktop-runtime", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct HotkeyRollbackResult {
    returned_status: GlobalHotkeyStatus,
    state_status: GlobalHotkeyStatus,
}

#[derive(Debug, Default)]
pub struct GlobalHotkeyStatusState {
    status: Mutex<Option<GlobalHotkeyStatus>>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SearchWindowPhase {
    Hidden,
    Showing,
    VisibleCollapsed,
    VisibleExpanded,
    Hiding,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SearchWindowSnapshot {
    pub session_id: u64,
    pub revision: u64,
    pub phase: SearchWindowPhase,
}

#[derive(Debug)]
pub struct SearchWindowLifecycleState {
    snapshot: Mutex<SearchWindowSnapshot>,
    // Every native window side effect must be performed while holding this
    // owner. A CAS check followed by a later native call is not sufficient:
    // an older resize could otherwise pass the check and mutate the window
    // after a newer lifecycle revision has committed.
    operation_owner: Mutex<()>,
    #[cfg_attr(not(feature = "desktop-runtime"), allow(dead_code))]
    creation_started: Mutex<Option<(u64, Instant)>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWindowMutationRequest {
    pub session_id: u64,
    pub expected_revision: u64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchWindowResizeRequest {
    pub session_id: u64,
    pub expected_revision: u64,
    pub expanded: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateSearchResultRequest {
    pub session_id: Option<u64>,
    pub expected_revision: Option<u64>,
    pub view: SearchView,
    pub file_id: Option<String>,
    pub settings_target: Option<SearchSettingsTarget>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MainWindowReadyRequest {
    pub nonce: u64,
    pub generation: u64,
    pub session_id: Option<u64>,
    pub revision: Option<u64>,
}

/// Transient confirmation of this exact navigation; no view/path authority.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SearchNavigationAcknowledgement {
    pub generation: u64,
    pub nonce: u64,
    pub session_id: Option<u64>,
    pub revision: Option<u64>,
    pub applied: bool,
}

#[derive(Debug, Default)]
struct MainWindowReadiness {
    generation: u64,
    ready: bool,
    next_nonce: u64,
    acknowledged_nonce: u64,
    navigation: Option<SearchNavigationAcknowledgement>,
    navigation_outcome: Option<bool>,
}

#[derive(Debug, Default)]
pub struct MainWindowReadinessState {
    readiness: Mutex<MainWindowReadiness>,
    changed: Condvar,
}

#[derive(Debug, Default)]
pub struct MainWindowLifecycleState {
    operation_owner: Mutex<()>,
    #[cfg_attr(not(feature = "desktop-runtime"), allow(dead_code))]
    next_generation: Mutex<u64>,
    #[cfg_attr(not(feature = "desktop-runtime"), allow(dead_code))]
    creation_started: Mutex<Option<(u64, Instant)>>,
}

#[derive(Debug, Default)]
pub struct MainWindowSessionState {
    last_view: Mutex<SearchView>,
}

impl SearchNavigatePayload {
    pub fn new(view: SearchView, file_id: Option<String>) -> Self {
        Self {
            view,
            file_id,
            nonce: 0,
            generation: 0,
            session_id: None,
            revision: None,
            settings_target: None,
        }
    }

    pub fn with_window_context(
        mut self,
        session_id: Option<u64>,
        revision: Option<u64>,
        settings_target: Option<SearchSettingsTarget>,
    ) -> Self {
        self.session_id = session_id;
        self.revision = revision;
        self.settings_target = settings_target;
        self
    }
}

impl GlobalHotkeyStatusState {
    pub fn set(&self, status: GlobalHotkeyStatus) {
        if let Ok(mut guard) = self.status.lock() {
            *guard = Some(status);
        }
    }

    pub fn get(&self) -> Option<GlobalHotkeyStatus> {
        self.status.lock().ok().and_then(|guard| guard.clone())
    }
}

impl Default for SearchWindowLifecycleState {
    fn default() -> Self {
        Self {
            snapshot: Mutex::new(SearchWindowSnapshot {
                session_id: 0,
                revision: 1,
                phase: SearchWindowPhase::Hidden,
            }),
            operation_owner: Mutex::new(()),
            creation_started: Mutex::new(None),
        }
    }
}

impl SearchWindowLifecycleState {
    #[cfg(feature = "desktop-runtime")]
    fn record_creation_start(&self, session_id: u64) {
        if let Ok(mut started) = self.creation_started.lock() {
            *started = Some((session_id, Instant::now()));
        }
    }

    #[cfg(feature = "desktop-runtime")]
    fn take_creation_latency(&self, session_id: u64) -> Option<Duration> {
        let mut started = self.creation_started.lock().ok()?;
        let (started_session, instant) = started.take()?;
        (started_session == session_id).then(|| instant.elapsed())
    }

    pub fn get(&self) -> SearchWindowSnapshot {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or(SearchWindowSnapshot {
                session_id: 0,
                revision: 1,
                phase: SearchWindowPhase::Hidden,
            })
    }

    #[cfg(test)]
    fn begin_show(&self) -> Result<SearchWindowSnapshot, String> {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        self.begin_show_locked()
    }

    fn begin_show_locked(&self) -> Result<SearchWindowSnapshot, String> {
        let mut state = self
            .snapshot
            .lock()
            .map_err(|_| "search_window_state_unavailable".to_string())?;
        if state.phase != SearchWindowPhase::Hidden {
            return Err("search_window_already_visible".to_string());
        }
        state.session_id = state.session_id.saturating_add(1);
        state.revision = state.revision.saturating_add(1);
        state.phase = SearchWindowPhase::Showing;
        Ok(state.clone())
    }

    fn complete_show(
        &self,
        session_id: u64,
        expected_revision: u64,
    ) -> Result<SearchWindowSnapshot, String> {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        self.transition_locked(
            session_id,
            expected_revision,
            &[SearchWindowPhase::Showing],
            SearchWindowPhase::VisibleCollapsed,
        )
    }

    #[cfg(test)]
    fn resize(&self, request: &SearchWindowResizeRequest) -> Result<SearchWindowSnapshot, String> {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        self.resize_locked(request)
    }

    fn resize_locked(
        &self,
        request: &SearchWindowResizeRequest,
    ) -> Result<SearchWindowSnapshot, String> {
        self.transition_locked(
            request.session_id,
            request.expected_revision,
            &[
                SearchWindowPhase::VisibleCollapsed,
                SearchWindowPhase::VisibleExpanded,
            ],
            if request.expanded {
                SearchWindowPhase::VisibleExpanded
            } else {
                SearchWindowPhase::VisibleCollapsed
            },
        )
    }

    #[cfg(test)]
    fn begin_hide(
        &self,
        request: Option<&SearchWindowMutationRequest>,
    ) -> Result<SearchWindowSnapshot, String> {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        self.begin_hide_locked(request)
    }

    fn begin_hide_locked(
        &self,
        request: Option<&SearchWindowMutationRequest>,
    ) -> Result<SearchWindowSnapshot, String> {
        let current = self.snapshot_locked()?;
        if let Some(request) = request {
            validate_search_window_cas(&current, request.session_id, request.expected_revision)?;
        }
        if current.phase == SearchWindowPhase::Hidden {
            return Ok(current);
        }
        if current.phase == SearchWindowPhase::Hiding {
            return Ok(current);
        }
        self.transition_locked(
            current.session_id,
            current.revision,
            &[
                SearchWindowPhase::Showing,
                SearchWindowPhase::VisibleCollapsed,
                SearchWindowPhase::VisibleExpanded,
            ],
            SearchWindowPhase::Hiding,
        )
    }

    #[cfg(test)]
    fn complete_hide(
        &self,
        session_id: u64,
        expected_revision: u64,
    ) -> Result<SearchWindowSnapshot, String> {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        self.transition_locked(
            session_id,
            expected_revision,
            &[SearchWindowPhase::Hiding],
            SearchWindowPhase::Hidden,
        )
    }

    fn snapshot_locked(&self) -> Result<SearchWindowSnapshot, String> {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .map_err(|_| "search_window_state_unavailable".to_string())
    }

    fn transition_locked(
        &self,
        session_id: u64,
        expected_revision: u64,
        allowed: &[SearchWindowPhase],
        next: SearchWindowPhase,
    ) -> Result<SearchWindowSnapshot, String> {
        let mut state = self
            .snapshot
            .lock()
            .map_err(|_| "search_window_state_unavailable".to_string())?;
        validate_search_window_cas(&state, session_id, expected_revision)?;
        if !allowed.contains(&state.phase) {
            return Err("search_window_transition_invalid".to_string());
        }
        state.revision = state.revision.saturating_add(1);
        state.phase = next;
        Ok(state.clone())
    }

    /// Validates the CAS, performs the injected native resize while the
    /// operation owner is held, and commits the new revision only after the
    /// native call succeeds. Tests use the injected closure as the native
    /// adapter; production passes the real Tauri resize operation.
    fn resize_with_native<F>(
        &self,
        request: &SearchWindowResizeRequest,
        native: F,
    ) -> Result<SearchWindowSnapshot, String>
    where
        F: FnOnce() -> Result<(), String>,
    {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        let current = self.snapshot_locked()?;
        validate_search_window_cas(&current, request.session_id, request.expected_revision)?;
        if !matches!(
            current.phase,
            SearchWindowPhase::VisibleCollapsed | SearchWindowPhase::VisibleExpanded
        ) {
            return Err("search_window_transition_invalid".to_string());
        }
        native()?;
        self.resize_locked(request)
    }

    /// Owns the entire show sequence. Native failure rolls the durable phase
    /// back to Hidden so a failed start is retryable and never leaves a stuck
    /// Showing phase behind.
    fn show_with_native<F, E>(&self, native: F, mut emit: E) -> Result<SearchWindowSnapshot, String>
    where
        F: FnOnce() -> Result<(), String>,
        E: FnMut(&SearchWindowSnapshot),
    {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        let previous = self.snapshot_locked()?;
        let showing = self.begin_show_locked()?;
        if let Err(error) = native() {
            self.restore_snapshot_locked(&previous)?;
            return Err(error);
        }
        emit(&showing);
        Ok(showing)
    }

    /// Owns the dismiss transition, native window destruction, and
    /// finalization. On native failure the prior visible snapshot is
    /// restored and emitted, so a renderer can retry.
    fn hide_with_native<F, E>(
        &self,
        request: Option<&SearchWindowMutationRequest>,
        native: F,
        mut emit: E,
    ) -> Result<SearchWindowSnapshot, String>
    where
        F: FnOnce() -> Result<(), String>,
        E: FnMut(&SearchWindowSnapshot),
    {
        let _owner = self
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        let previous = self.snapshot_locked()?;
        let hiding = self.begin_hide_locked(request)?;
        if hiding.phase == SearchWindowPhase::Hidden {
            return Ok(hiding);
        }
        emit(&hiding);
        if let Err(error) = native() {
            self.restore_snapshot_locked(&previous)?;
            emit(&previous);
            return Err(error);
        }
        let hidden = self.transition_locked(
            hiding.session_id,
            hiding.revision,
            &[SearchWindowPhase::Hiding],
            SearchWindowPhase::Hidden,
        )?;
        emit(&hidden);
        Ok(hidden)
    }

    fn restore_snapshot_locked(&self, previous: &SearchWindowSnapshot) -> Result<(), String> {
        let mut state = self
            .snapshot
            .lock()
            .map_err(|_| "search_window_state_unavailable".to_string())?;
        *state = previous.clone();
        Ok(())
    }
}

fn validate_search_window_cas(
    current: &SearchWindowSnapshot,
    session_id: u64,
    expected_revision: u64,
) -> Result<(), String> {
    if current.session_id != session_id {
        return Err("search_window_session_stale".to_string());
    }
    if current.revision != expected_revision {
        return Err("search_window_revision_stale".to_string());
    }
    Ok(())
}

impl MainWindowReadinessState {
    fn begin_generation(&self, generation: u64) -> Result<(), String> {
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        state.generation = generation;
        state.ready = false;
        state.next_nonce = 0;
        state.acknowledged_nonce = 0;
        state.navigation = None;
        state.navigation_outcome = None;
        self.changed.notify_all();
        Ok(())
    }

    #[cfg(feature = "desktop-runtime")]
    fn invalidate_generation(&self, generation: u64) {
        if let Ok(mut state) = self.readiness.lock() {
            if state.generation == generation {
                state.ready = false;
                state.next_nonce = 0;
                state.acknowledged_nonce = 0;
                state.navigation = None;
                state.navigation_outcome = None;
            }
            self.changed.notify_all();
        }
    }

    fn set_ready(&self, generation: u64, ready: bool) -> Result<(), String> {
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        if generation == 0 || state.generation != generation {
            return Err("main_window_generation_stale".to_string());
        }
        state.ready = ready;
        if !ready {
            state.navigation = None;
            state.navigation_outcome = None;
        }
        self.changed.notify_all();
        Ok(())
    }

    fn begin_request(&self, timeout: Duration) -> Result<(u64, u64), String> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        while !state.ready {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("main_window_not_ready".to_string());
            }
            let (next, wait) = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| "main_window_readiness_unavailable".to_string())?;
            state = next;
            if wait.timed_out() && !state.ready {
                return Err("main_window_not_ready".to_string());
            }
        }
        state.navigation = None;
        state.navigation_outcome = None;
        state.next_nonce = state.next_nonce.saturating_add(1);
        Ok((state.generation, state.next_nonce))
    }

    fn acknowledge(&self, generation: u64, nonce: u64) -> Result<(), String> {
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        if generation == 0 || state.generation != generation {
            return Err("main_window_generation_stale".to_string());
        }
        if nonce == 0 || nonce != state.next_nonce {
            return Err("main_window_ready_nonce_stale".to_string());
        }
        state.acknowledged_nonce = nonce;
        self.changed.notify_all();
        Ok(())
    }

    fn wait_for_ack(&self, generation: u64, nonce: u64, timeout: Duration) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        while state.acknowledged_nonce < nonce {
            if state.generation != generation {
                return Err("main_window_generation_stale".to_string());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err("main_window_ready_ack_timeout".to_string());
            }
            let (next, wait) = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| "main_window_readiness_unavailable".to_string())?;
            state = next;
            if wait.timed_out() && state.acknowledged_nonce < nonce {
                return Err("main_window_ready_ack_timeout".to_string());
            }
        }
        if state.generation != generation {
            return Err("main_window_generation_stale".to_string());
        }
        Ok(())
    }
}

fn validate_active_search(
    snapshot: &SearchWindowSnapshot,
    request: &SearchWindowMutationRequest,
) -> Result<(), String> {
    validate_search_window_cas(snapshot, request.session_id, request.expected_revision)?;
    if !matches!(
        snapshot.phase,
        SearchWindowPhase::VisibleCollapsed | SearchWindowPhase::VisibleExpanded
    ) {
        return Err("search_window_phase_stale".to_string());
    }
    Ok(())
}

impl MainWindowReadinessState {
    #[cfg(any(feature = "desktop-runtime", test))]
    fn begin_navigation(&self, binding: SearchNavigationAcknowledgement) -> Result<(), String> {
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        if !state.ready || state.generation != binding.generation || binding.generation == 0 {
            return Err("main_window_generation_stale".to_string());
        }
        if binding.nonce == 0
            || state.next_nonce != binding.nonce
            || state.acknowledged_nonce != binding.nonce
        {
            return Err("main_window_ready_nonce_stale".to_string());
        }
        state.navigation = Some(binding);
        state.navigation_outcome = None;
        Ok(())
    }

    fn acknowledge_navigation(
        &self,
        binding: SearchNavigationAcknowledgement,
    ) -> Result<(), String> {
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        let mut expected = binding.clone();
        expected.applied = false;
        if !state.ready
            || state.generation != binding.generation
            || state.navigation.as_ref() != Some(&expected)
            || state.navigation_outcome.is_some()
        {
            return Err("search_navigation_ack_stale".to_string());
        }
        state.navigation_outcome = Some(binding.applied);
        self.changed.notify_all();
        Ok(())
    }

    #[cfg(any(feature = "desktop-runtime", test))]
    fn cancel_navigation(&self, binding: &SearchNavigationAcknowledgement) {
        if let Ok(mut state) = self.readiness.lock() {
            if state.navigation.as_ref() == Some(binding) {
                state.navigation = None;
                state.navigation_outcome = None;
            }
            self.changed.notify_all();
        }
    }

    #[cfg(any(feature = "desktop-runtime", test))]
    fn wait_for_navigation(
        &self,
        binding: &SearchNavigationAcknowledgement,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = Instant::now() + timeout;
        let mut state = self
            .readiness
            .lock()
            .map_err(|_| "main_window_readiness_unavailable".to_string())?;
        let result = loop {
            if !state.ready
                || state.generation != binding.generation
                || state.navigation.as_ref() != Some(binding)
            {
                break Err("search_navigation_main_stale".to_string());
            }
            if let Some(applied) = state.navigation_outcome {
                break if applied {
                    Ok(())
                } else {
                    Err("search_navigation_rejected".to_string())
                };
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err("search_navigation_ack_timeout".to_string());
            }
            state = self
                .changed
                .wait_timeout(state, remaining)
                .map_err(|_| "main_window_readiness_unavailable".to_string())?
                .0;
        };
        if state.navigation.as_ref() == Some(binding) {
            state.navigation = None;
            state.navigation_outcome = None;
        }
        result
    }
}

/// Shared by native transport and tests. Emission alone never permits hide.
#[cfg(any(feature = "desktop-runtime", test))]
fn handoff_search_navigation(
    lifecycle: &SearchWindowLifecycleState,
    readiness: &MainWindowReadinessState,
    mut payload: SearchNavigatePayload,
    timeout: Duration,
    emit_ready: impl FnOnce(MainWindowReadyRequest) -> Result<(), String>,
    emit_navigation: impl FnOnce(&SearchNavigatePayload) -> Result<(), String>,
    hide_scoped: impl FnOnce(&SearchWindowMutationRequest) -> Result<(), String>,
) -> Result<(), String> {
    let original = match (payload.session_id, payload.revision) {
        (Some(session_id), Some(expected_revision)) => Some(SearchWindowMutationRequest {
            session_id,
            expected_revision,
        }),
        (None, None) => None,
        _ => return Err("search_window_session_required".to_string()),
    };
    if let Some(request) = &original {
        validate_active_search(&lifecycle.get(), request)?;
    }
    let (generation, nonce) = readiness.begin_request(timeout)?;
    payload.generation = generation;
    payload.nonce = nonce;
    emit_ready(MainWindowReadyRequest {
        generation,
        nonce,
        session_id: payload.session_id,
        revision: payload.revision,
    })?;
    readiness.wait_for_ack(generation, nonce, timeout)?;
    let binding = SearchNavigationAcknowledgement {
        generation,
        nonce,
        session_id: payload.session_id,
        revision: payload.revision,
        applied: false,
    };
    {
        let _owner = lifecycle
            .operation_owner
            .lock()
            .map_err(|_| "search_window_operation_unavailable".to_string())?;
        if let Some(request) = &original {
            validate_active_search(&lifecycle.snapshot_locked()?, request)?;
        }
        readiness.begin_navigation(binding.clone())?;
        if let Err(error) = emit_navigation(&payload) {
            readiness.cancel_navigation(&binding);
            return Err(error);
        }
    }
    readiness.wait_for_navigation(&binding, timeout)?;
    if let Some(request) = &original {
        validate_active_search(&lifecycle.get(), request)?;
        // Scoped hide repeats CAS under the Search operation lock.
        hide_scoped(request)?;
    }
    Ok(())
}

impl MainWindowLifecycleState {
    #[cfg(feature = "desktop-runtime")]
    fn next_generation(&self) -> Result<u64, String> {
        let mut next = self
            .next_generation
            .lock()
            .map_err(|_| "main_window_lifecycle_unavailable".to_string())?;
        *next = next
            .checked_add(1)
            .ok_or_else(|| "main_window_generation_exhausted".to_string())?;
        Ok(*next)
    }

    /// Reports the latest allocated Main generation after a lifecycle operation succeeds.
    pub fn latest_generation(&self) -> Result<u64, String> {
        self.next_generation
            .lock()
            .map(|generation| *generation)
            .map_err(|_| "main_window_lifecycle_unavailable".to_string())
    }

    #[cfg(feature = "desktop-runtime")]
    fn record_creation_start(&self, generation: u64) {
        if let Ok(mut started) = self.creation_started.lock() {
            *started = Some((generation, Instant::now()));
        }
    }

    #[cfg(feature = "desktop-runtime")]
    fn take_creation_latency(&self, generation: u64) -> Option<Duration> {
        let mut started = self.creation_started.lock().ok()?;
        let (started_generation, instant) = started.take()?;
        (started_generation == generation).then(|| instant.elapsed())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, ()>, String> {
        self.operation_owner
            .lock()
            .map_err(|_| "main_window_lifecycle_unavailable".to_string())
    }
}

impl MainWindowSessionState {
    fn set_last_view(&self, view: SearchView) -> Result<(), String> {
        *self
            .last_view
            .lock()
            .map_err(|_| "main_window_session_unavailable".to_string())? = view;
        Ok(())
    }

    #[cfg(feature = "desktop-runtime")]
    fn last_view(&self) -> Result<SearchView, String> {
        self.last_view
            .lock()
            .map(|view| *view)
            .map_err(|_| "main_window_session_unavailable".to_string())
    }
}

pub fn search_window_url() -> &'static str {
    SEARCH_WINDOW_URL
}

#[cfg(feature = "native-qa")]
pub fn trace_resident_lifecycle<R: Runtime>(app: &AppHandle<R>, stage: &str, detail: &str) {
    #[cfg(all(target_os = "windows", feature = "desktop-runtime"))]
    crate::native_resident_lifecycle_qa::observe(app, stage);
    let windows = app.webview_windows();
    let mut labels = windows.keys().map(String::as_str).collect::<Vec<_>>();
    labels.sort_unstable();
    let generation = app
        .try_state::<MainWindowLifecycleState>()
        .and_then(|s| s.latest_generation().ok())
        .unwrap_or(0);
    app.state::<ExitIntentState>().trace(
        stage,
        &format!(
            "generation={generation} webviews={} labels={} {detail}",
            windows.len(),
            labels.join(",")
        ),
    );
}

pub fn exit_app<R: Runtime>(app: &AppHandle<R>) {
    app.state::<ExitIntentState>().record_explicit_exit();
    #[cfg(feature = "native-qa")]
    trace_resident_lifecycle(app, "explicit_quit", "");
    app.exit(0);
}

#[tauri::command]
pub fn quit_app<R: Runtime>(window: WebviewWindow<R>, app: AppHandle<R>) -> Result<(), String> {
    require_main_window(&window)?;
    exit_app(&app);
    Ok(())
}

#[cfg(feature = "desktop-runtime")]
pub fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let lifecycle = app.state::<MainWindowLifecycleState>();
    let _owner = lifecycle.lock()?;
    ensure_main_window_locked(
        app,
        &lifecycle,
        &app.state::<MainWindowReadinessState>(),
        &app.state::<crate::file_workspace::integration::FileWorkspaceRuntimeOwner>(),
        &app.state::<MainWindowSessionState>(),
    )
}

#[cfg(feature = "desktop-runtime")]
fn ensure_main_window_locked<R: Runtime>(
    app: &AppHandle<R>,
    lifecycle: &MainWindowLifecycleState,
    readiness: &MainWindowReadinessState,
    workspace: &crate::file_workspace::integration::FileWorkspaceRuntimeOwner,
    session: &MainWindowSessionState,
) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) {
        window.unminimize().map_err(|error| error.to_string())?;
        window.show().map_err(|error| error.to_string())?;
        window.set_focus().map_err(|error| error.to_string())?;
        return Ok(());
    }

    let generation = lifecycle.next_generation()?;
    workspace.activate_generation(generation)?;
    if let Err(error) = readiness.begin_generation(generation) {
        let _ = workspace.abort_generation(generation);
        return Err(error);
    }
    let last_view = match session.last_view() {
        Ok(view) => view,
        Err(error) => {
            let _ = workspace.abort_generation(generation);
            readiness.invalidate_generation(generation);
            return Err(error);
        }
    };
    let url = main_window_restore_url(generation, last_view);
    lifecycle.record_creation_start(generation);
    let window =
        match WebviewWindowBuilder::new(app, MAIN_WINDOW_LABEL, WebviewUrl::App(url.into()))
            .title("Zen Canvas")
            .inner_size(MAIN_WINDOW_WIDTH, MAIN_WINDOW_HEIGHT)
            .min_inner_size(MAIN_WINDOW_MIN_WIDTH, MAIN_WINDOW_MIN_HEIGHT)
            .transparent(true)
            .decorations(false)
            .visible(false)
            .build()
        {
            Ok(window) => window,
            Err(error) => {
                let _ = workspace.abort_generation(generation);
                readiness.invalidate_generation(generation);
                return Err(error.to_string());
            }
        };

    app.state::<ExitIntentState>().record_window_created();
    let app_handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = app_handle.emit_to(MAIN_WINDOW_LABEL, MAIN_WINDOW_CLOSE_REQUESTED_EVENT, ());
        }
    });
    if let Err(error) = window.show().and_then(|()| window.set_focus()) {
        match destroy_failed_main_window(
            &app.state::<ExitIntentState>(),
            app.webview_windows().len(),
            || window.destroy(),
        ) {
            Ok(()) => {
                let _ = workspace.abort_generation(generation);
                readiness.invalidate_generation(generation);
            }
            Err(cleanup_error) => {
                return Err(format!(
                    "main_window_show_failed:{error};main_window_cleanup_failed:{cleanup_error}"
                ));
            }
        }
        return Err(error.to_string());
    }
    eprintln!(
        "ui_runtime main_window_created generation={generation} webview_count={}",
        app.webview_windows().len()
    );
    Ok(())
}

#[cfg(any(feature = "desktop-runtime", test))]
fn destroy_failed_main_window<E>(
    exit_intent: &ExitIntentState,
    webview_count: usize,
    destroy: impl FnOnce() -> Result<(), E>,
) -> Result<(), E> {
    let teardown = exit_intent.begin_internal_window_teardown(webview_count);
    // An error drops the guard, withdrawing the prediction rather than
    // leaving a stale suppression for a later native/system exit.
    destroy()?;
    teardown.complete();
    Ok(())
}

#[cfg(test)]
#[path = "app_control_main_failure_tests.rs"]
mod main_failure_tests;

#[tauri::command]
pub fn enter_background<R: Runtime>(
    window: WebviewWindow<R>,
    lifecycle: State<'_, MainWindowLifecycleState>,
    readiness: State<'_, MainWindowReadinessState>,
    workspace: State<'_, crate::file_workspace::integration::FileWorkspaceRuntimeOwner>,
    session: State<'_, MainWindowSessionState>,
    app: AppHandle<R>,
    last_view: SearchView,
) -> Result<(), String> {
    require_main_window(&window)?;
    let generation = crate::file_workspace::integration::main_generation_from_window(&window)?;
    #[cfg(feature = "native-qa")]
    trace_resident_lifecycle(&app, "enter_background", "");
    let _owner = lifecycle.lock()?;
    session.set_last_view(last_view)?;
    readiness.set_ready(generation, false)?;
    if let Err(error) = workspace.dispose_generation(generation) {
        return Err(restore_readiness_after_workspace_dispose_failure(
            &readiness, generation, error,
        ));
    }
    let exit_intent = app.state::<ExitIntentState>();
    let teardown = exit_intent.begin_internal_window_teardown(app.webview_windows().len());
    #[cfg(feature = "native-qa")]
    trace_resident_lifecycle(&app, "teardown_begin", "");
    #[cfg(feature = "native-qa")]
    trace_resident_lifecycle(&app, "destroy_start", "");
    if let Err(error) = window.destroy() {
        #[cfg(feature = "native-qa")]
        trace_resident_lifecycle(&app, "destroy_result", "result=error");
        drop(teardown);
        let _ = workspace.resume_after_window_destroy_failure(generation);
        let _ = readiness.set_ready(generation, true);
        return Err(format!("main_window_destroy_failed:{error}"));
    }
    #[cfg(feature = "native-qa")]
    trace_resident_lifecycle(&app, "destroy_result", "result=ok");
    teardown.complete();
    #[cfg(feature = "native-qa")]
    trace_resident_lifecycle(&app, "teardown_complete", "");
    #[cfg(feature = "desktop-runtime")]
    eprintln!(
        "ui_runtime main_window_destroyed generation={generation} webview_count={}",
        app.webview_windows().len()
    );
    #[cfg(not(feature = "desktop-runtime"))]
    let _ = app;
    Ok(())
}

fn restore_readiness_after_workspace_dispose_failure(
    readiness: &MainWindowReadinessState,
    generation: u64,
    dispose_error: String,
) -> String {
    match readiness.set_ready(generation, true) {
        Ok(()) => dispose_error,
        Err(restore_error) => {
            format!("{dispose_error};main_window_readiness_restore_failed:{restore_error}")
        }
    }
}

#[tauri::command]
pub async fn activate_search_result<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    lifecycle: State<'_, SearchWindowLifecycleState>,
    request: ActivateSearchResultRequest,
) -> Result<(), String> {
    if window.label() == SEARCH_WINDOW_LABEL {
        let (Some(session_id), Some(expected_revision)) =
            (request.session_id, request.expected_revision)
        else {
            return Err("search_window_session_required".to_string());
        };
        validate_active_search(
            &lifecycle.get(),
            &SearchWindowMutationRequest {
                session_id,
                expected_revision,
            },
        )?;
    } else {
        require_main_window(&window)?;
    }
    let payload = SearchNavigatePayload::new(request.view, request.file_id).with_window_context(
        request.session_id,
        request.expected_revision,
        request.settings_target,
    );
    // Renderer ACKs must be deliverable while this bounded handshake waits.
    tauri::async_runtime::spawn_blocking(move || {
        activate_search_result_payload(
            &app,
            &app.state::<SearchWindowLifecycleState>(),
            &app.state::<MainWindowReadinessState>(),
            payload,
        )
    })
    .await
    .map_err(|_| "search_navigation_worker_failed".to_string())?
}

#[tauri::command]
pub fn get_global_hotkey_status(
    status_state: State<'_, GlobalHotkeyStatusState>,
) -> Option<GlobalHotkeyStatus> {
    status_state.get()
}

#[tauri::command]
pub fn register_global_search_hotkey<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    status_state: State<'_, GlobalHotkeyStatusState>,
    accelerator: String,
) -> GlobalHotkeyStatus {
    if require_main_window(&window).is_err() {
        let previous = status_state.get();
        return GlobalHotkeyStatus {
            requested_accelerator: global_search_accelerator(&accelerator).to_string(),
            effective_accelerator: previous
                .as_ref()
                .and_then(|status| status.effective_accelerator.clone()),
            registered: previous.as_ref().is_some_and(|status| status.registered),
            error: Some("main_window_required".to_string()),
            revision: previous.map_or(1, |status| status.revision.saturating_add(1)),
        };
    }
    register_global_search_shortcut(&app, &status_state, &accelerator)
}

#[tauri::command]
pub fn get_search_window_state(
    lifecycle: State<'_, SearchWindowLifecycleState>,
) -> SearchWindowSnapshot {
    lifecycle.get()
}

#[tauri::command]
pub fn search_window_ready<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    lifecycle: State<'_, SearchWindowLifecycleState>,
    request: SearchWindowMutationRequest,
) -> Result<SearchWindowSnapshot, String> {
    require_search_window(&window)?;
    let snapshot = lifecycle.complete_show(request.session_id, request.expected_revision)?;
    #[cfg(feature = "desktop-runtime")]
    if let Some(latency) = lifecycle.take_creation_latency(request.session_id) {
        eprintln!(
            "ui_runtime search_window_ready session={} create_to_ready_ms={} webview_count={}",
            request.session_id,
            latency.as_secs_f64() * 1000.0,
            app.webview_windows().len()
        );
    }
    emit_search_window_state(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub fn resize_search_window<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    lifecycle: State<'_, SearchWindowLifecycleState>,
    request: SearchWindowResizeRequest,
) -> Result<SearchWindowSnapshot, String> {
    require_search_window(&window)?;
    let snapshot = lifecycle.resize_with_native(&request, || {
        resize_search_window_for_state(&app, request.expanded).map_err(|error| error.to_string())
    })?;
    emit_search_window_state(&app, &snapshot);
    Ok(snapshot)
}

#[tauri::command]
pub fn hide_search_window_command<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    lifecycle: State<'_, SearchWindowLifecycleState>,
    request: SearchWindowMutationRequest,
) -> Result<SearchWindowSnapshot, String> {
    require_search_window(&window)?;
    hide_search_window_with_state(&app, &lifecycle, Some(&request))
}

#[tauri::command]
pub fn mark_main_window_ready<R: Runtime>(
    window: WebviewWindow<R>,
    app: AppHandle<R>,
    lifecycle: State<'_, MainWindowLifecycleState>,
    readiness: State<'_, MainWindowReadinessState>,
    ready: bool,
) -> Result<(), String> {
    require_main_window(&window)?;
    let generation = crate::file_workspace::integration::main_generation_from_window(&window)?;
    readiness.set_ready(generation, ready)?;
    #[cfg(feature = "desktop-runtime")]
    if ready {
        if let Some(latency) = lifecycle.take_creation_latency(generation) {
            eprintln!(
                "ui_runtime main_window_ready generation={generation} create_to_ready_ms={} webview_count={}",
                latency.as_secs_f64() * 1000.0,
                app.webview_windows().len()
            );
        }
    }
    #[cfg(feature = "native-qa")]
    if ready {
        trace_resident_lifecycle(&app, "main_ready", "");
    }
    #[cfg(not(feature = "desktop-runtime"))]
    let _ = (&app, &lifecycle);
    Ok(())
}

#[tauri::command]
pub fn acknowledge_main_window_ready<R: Runtime>(
    window: WebviewWindow<R>,
    readiness: State<'_, MainWindowReadinessState>,
    nonce: u64,
) -> Result<(), String> {
    require_main_window(&window)?;
    let generation = crate::file_workspace::integration::main_generation_from_window(&window)?;
    readiness.acknowledge(generation, nonce)
}

#[tauri::command]
pub fn acknowledge_search_navigation<R: Runtime>(
    window: WebviewWindow<R>,
    readiness: State<'_, MainWindowReadinessState>,
    request: SearchNavigationAcknowledgement,
) -> Result<(), String> {
    require_main_window(&window)?;
    let generation = crate::file_workspace::integration::main_generation_from_window(&window)?;
    if generation != request.generation {
        return Err("main_window_generation_stale".to_string());
    }
    readiness.acknowledge_navigation(request)
}

#[cfg(feature = "desktop-runtime")]
fn activate_search_result_payload<R: Runtime>(
    app: &AppHandle<R>,
    lifecycle: &SearchWindowLifecycleState,
    readiness: &MainWindowReadinessState,
    payload: SearchNavigatePayload,
) -> Result<(), String> {
    eprintln!(
        "ui_runtime search_activation_received view={:?} session_id={:?} revision={:?}",
        payload.view, payload.session_id, payload.revision
    );
    let stage_error = |stage: &str, error: String| {
        eprintln!("ui_runtime search_activation_failed stage={stage} error={error}");
        error
    };
    let main_lifecycle = app.state::<MainWindowLifecycleState>();
    let _owner = main_lifecycle
        .lock()
        .map_err(|error| stage_error("main_lifecycle_lock", error))?;
    eprintln!("ui_runtime main_ensure_started");
    ensure_main_window_locked(
        app,
        &main_lifecycle,
        readiness,
        &app.state::<crate::file_workspace::integration::FileWorkspaceRuntimeOwner>(),
        &app.state::<MainWindowSessionState>(),
    )
    .map_err(|error| stage_error("main_ensure_started", error))?;
    let main_generation = app
        .state::<MainWindowLifecycleState>()
        .latest_generation()
        .map_or_else(
            |_| "unavailable".to_string(),
            |generation| generation.to_string(),
        );
    eprintln!(
        "ui_runtime main_ensure_started result=ok generation={main_generation} webview_count={}",
        app.webview_windows().len()
    );

    handoff_search_navigation(
        lifecycle,
        readiness,
        payload,
        MAIN_WINDOW_READY_TIMEOUT,
        |request| {
            eprintln!(
                "ui_runtime main_ready_request generation={} nonce={}",
                request.generation, request.nonce
            );
            app.emit_to(MAIN_WINDOW_LABEL, MAIN_WINDOW_READY_REQUEST_EVENT, request)
                .map_err(|error| error.to_string())
        },
        |payload| {
            eprintln!(
                "ui_runtime main_ready_ack generation={} nonce={}",
                payload.generation, payload.nonce
            );
            app.emit_to(MAIN_WINDOW_LABEL, SEARCH_NAVIGATE_EVENT, payload)
                .map_err(|error| error.to_string())?;
            eprintln!(
                "ui_runtime search_navigation_emitted generation={} nonce={}",
                payload.generation, payload.nonce
            );
            Ok(())
        },
        |request| {
            eprintln!(
                "ui_runtime navigation_applied_ack session_id={} revision={}",
                request.session_id, request.expected_revision
            );
            hide_search_window_with_state(app, lifecycle, Some(request))?;
            eprintln!(
                "ui_runtime search_scoped_hide session_id={} revision={}",
                request.session_id, request.expected_revision
            );
            Ok(())
        },
    )
    .map_err(|error| stage_error("navigation_commit", error))
}

#[cfg(not(feature = "desktop-runtime"))]
fn activate_search_result_payload<R: Runtime>(
    _app: &AppHandle<R>,
    _lifecycle: &SearchWindowLifecycleState,
    _readiness: &MainWindowReadinessState,
    _payload: SearchNavigatePayload,
) -> Result<(), String> {
    Ok(())
}

#[cfg(feature = "desktop-runtime")]
fn create_search_window<R: Runtime>(app: &AppHandle<R>) -> Result<WebviewWindow<R>, String> {
    let search_window = WebviewWindowBuilder::new(
        app,
        SEARCH_WINDOW_LABEL,
        WebviewUrl::App(search_window_url().into()),
    )
    .title("Zen Canvas Search")
    .inner_size(SEARCH_WINDOW_WIDTH, SEARCH_WINDOW_COLLAPSED_HEIGHT)
    .min_inner_size(SEARCH_WINDOW_WIDTH, SEARCH_WINDOW_COLLAPSED_HEIGHT)
    .max_inner_size(SEARCH_WINDOW_WIDTH, SEARCH_WINDOW_EXPANDED_HEIGHT)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(false)
    .skip_taskbar(true)
    .always_on_top(true)
    .visible(false)
    .center()
    .build()
    .map_err(|error| error.to_string())?;
    app.state::<ExitIntentState>().record_window_created();
    let app_handle = app.clone();
    search_window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let lifecycle = app_handle.state::<SearchWindowLifecycleState>();
            if let Err(error) = hide_search_window_with_state(&app_handle, &lifecycle, None) {
                eprintln!("Destroy search window after close request failed: {error}");
            }
        }
    });
    Ok(search_window)
}

#[cfg(feature = "desktop-runtime")]
pub fn setup_global_search_shortcut(app: &mut App, accelerator: &str) -> Result<(), String> {
    app.handle()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        if let Err(error) = toggle_search_window(app) {
                            eprintln!("Toggle search window from global shortcut failed: {error}");
                        }
                    }
                })
                .build(),
        )
        .map_err(|error| error.to_string())?;
    let status_state = app.state::<GlobalHotkeyStatusState>();
    let status = register_global_search_shortcut(app.handle(), &status_state, accelerator);
    if status.registered {
        Ok(())
    } else {
        Err(status
            .error
            .unwrap_or_else(|| "Global search hotkey registration failed".to_string()))
    }
}

#[cfg(feature = "desktop-runtime")]
fn register_global_search_shortcut<R: Runtime>(
    app: &AppHandle<R>,
    status_state: &GlobalHotkeyStatusState,
    accelerator: &str,
) -> GlobalHotkeyStatus {
    let requested = global_search_accelerator(accelerator).to_string();
    let previous_status = status_state.get();
    if let Some(previous) = previous_status
        .as_ref()
        .filter(|status| hotkey_registration_is_idempotent(status, &requested))
    {
        return previous.clone();
    }
    let next_revision = previous_status
        .as_ref()
        .map_or(1, |status| status.revision.saturating_add(1));
    let shortcut = match global_search_shortcut(&requested) {
        Ok(shortcut) => shortcut,
        Err(error) => {
            let message =
                format!("Global search hotkey registration failed for {requested}: {error}");
            eprintln!("{message}");
            emit_global_hotkey_error(app, message.clone());
            let status = GlobalHotkeyStatus {
                requested_accelerator: requested,
                effective_accelerator: previous_status
                    .as_ref()
                    .and_then(|status| status.effective_accelerator.clone()),
                registered: previous_status
                    .as_ref()
                    .is_some_and(|status| status.registered),
                error: Some(message),
                revision: next_revision,
            };
            status_state.set(status.clone());
            return status;
        }
    };

    let previous_effective = previous_status.as_ref().and_then(|status| {
        status
            .registered
            .then(|| status.effective_accelerator.clone())
            .flatten()
    });
    if let Some(previous_accelerator) = previous_effective.as_deref() {
        let previous_shortcut = match global_search_shortcut(previous_accelerator) {
            Ok(shortcut) => shortcut,
            Err(error) => {
                let message = format!("Previous global search hotkey could not be parsed: {error}");
                let status = GlobalHotkeyStatus {
                    requested_accelerator: requested,
                    effective_accelerator: previous_effective,
                    registered: true,
                    error: Some(message.clone()),
                    revision: next_revision,
                };
                emit_global_hotkey_error(app, message);
                status_state.set(status.clone());
                return status;
            }
        };
        if let Err(error) = app.global_shortcut().unregister(previous_shortcut) {
            let message = format!("Global search hotkey reset failed: {error}");
            let status = GlobalHotkeyStatus {
                requested_accelerator: requested,
                effective_accelerator: previous_effective,
                registered: true,
                error: Some(message.clone()),
                revision: next_revision,
            };
            emit_global_hotkey_error(app, message);
            status_state.set(status.clone());
            return status;
        }
    }

    let status = match app.global_shortcut().register(shortcut) {
        Ok(()) => GlobalHotkeyStatus {
            requested_accelerator: requested,
            effective_accelerator: Some(global_search_accelerator(accelerator).to_string()),
            registered: true,
            error: None,
            revision: next_revision,
        },
        Err(error) => {
            let restore_error =
                restore_previous_global_hotkey(app, previous_effective.as_deref()).err();
            let rollback = hotkey_registration_failure_with_rollback(
                requested,
                error.to_string(),
                previous_effective,
                restore_error,
                next_revision,
            );
            if let Some(message) = rollback.returned_status.error.clone() {
                eprintln!("{message}");
                emit_global_hotkey_error(app, message);
            }
            status_state.set(rollback.state_status.clone());
            return rollback.returned_status;
        }
    };
    status_state.set(status.clone());
    status
}

fn hotkey_registration_is_idempotent(previous: &GlobalHotkeyStatus, requested: &str) -> bool {
    previous.registered
        && previous.error.is_none()
        && previous.effective_accelerator.as_deref() == Some(requested)
}

#[cfg(feature = "desktop-runtime")]
fn restore_previous_global_hotkey<R: Runtime>(
    app: &AppHandle<R>,
    previous_accelerator: Option<&str>,
) -> Result<(), String> {
    let Some(previous_accelerator) = previous_accelerator else {
        return Ok(());
    };
    let previous_shortcut = global_search_shortcut(previous_accelerator)?;
    app.global_shortcut()
        .register(previous_shortcut)
        .map_err(|error| error.to_string())
}

#[cfg(any(feature = "desktop-runtime", test))]
fn hotkey_registration_failure_with_rollback(
    requested_accelerator: String,
    registration_error: String,
    previous_effective: Option<String>,
    restore_error: Option<String>,
    revision: u64,
) -> HotkeyRollbackResult {
    let base_message = format!(
        "Global search hotkey registration failed for {requested_accelerator}: {registration_error}"
    );

    match (previous_effective, restore_error) {
        (Some(previous), None) => {
            let returned_status = GlobalHotkeyStatus {
                requested_accelerator,
                effective_accelerator: Some(previous.clone()),
                registered: true,
                error: Some(format!(
                    "{base_message}; restored previous hotkey {}",
                    previous
                )),
                revision,
            };
            HotkeyRollbackResult {
                returned_status: returned_status.clone(),
                state_status: returned_status,
            }
        }
        (Some(previous), Some(restore_error)) => {
            let returned_status = GlobalHotkeyStatus {
                requested_accelerator,
                effective_accelerator: None,
                registered: false,
                error: Some(format!(
                    "{base_message}; restore previous hotkey failed for {}: {restore_error}",
                    previous
                )),
                revision,
            };
            HotkeyRollbackResult {
                returned_status: returned_status.clone(),
                state_status: returned_status,
            }
        }
        _ => {
            let returned_status = GlobalHotkeyStatus {
                requested_accelerator,
                effective_accelerator: None,
                registered: false,
                error: Some(base_message),
                revision,
            };
            HotkeyRollbackResult {
                returned_status: returned_status.clone(),
                state_status: returned_status,
            }
        }
    }
}

#[cfg(not(feature = "desktop-runtime"))]
fn register_global_search_shortcut<R: Runtime>(
    _app: &AppHandle<R>,
    status_state: &GlobalHotkeyStatusState,
    accelerator: &str,
) -> GlobalHotkeyStatus {
    let status = GlobalHotkeyStatus {
        requested_accelerator: global_search_accelerator(accelerator).to_string(),
        effective_accelerator: None,
        registered: false,
        error: Some("Global hotkeys require the desktop runtime.".to_string()),
        revision: status_state
            .get()
            .map_or(1, |status| status.revision.saturating_add(1)),
    };
    status_state.set(status.clone());
    status
}

#[cfg(feature = "desktop-runtime")]
fn global_search_shortcut(accelerator: &str) -> Result<Shortcut, String> {
    global_search_accelerator(accelerator)
        .parse::<Shortcut>()
        .map_err(|error| error.to_string())
}

pub fn global_search_accelerator(accelerator: &str) -> &str {
    let trimmed = accelerator.trim();
    if trimmed.is_empty() {
        DEFAULT_SEARCH_HOTKEY
    } else {
        trimmed
    }
}

#[cfg(feature = "desktop-runtime")]
fn emit_global_hotkey_error<R: Runtime>(app: &AppHandle<R>, message: String) {
    let _ = app.emit(
        GLOBAL_HOTKEY_REGISTRATION_FAILED_EVENT,
        GlobalHotkeyErrorPayload { message },
    );
}

#[cfg(feature = "desktop-runtime")]
fn resize_search_window_for_state<R: Runtime>(
    app: &AppHandle<R>,
    expanded: bool,
) -> Result<(), String> {
    let window = app
        .get_webview_window(SEARCH_WINDOW_LABEL)
        .ok_or_else(|| "search_window_missing".to_string())?;
    window
        .set_size(Size::Logical(LogicalSize {
            width: SEARCH_WINDOW_WIDTH,
            height: if expanded {
                SEARCH_WINDOW_EXPANDED_HEIGHT
            } else {
                SEARCH_WINDOW_COLLAPSED_HEIGHT
            },
        }))
        .map_err(|error| error.to_string())?;
    window.center().map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(not(feature = "desktop-runtime"))]
fn resize_search_window_for_state<R: Runtime>(
    _app: &AppHandle<R>,
    _expanded: bool,
) -> tauri::Result<()> {
    Ok(())
}

#[cfg(feature = "desktop-runtime")]
pub fn toggle_search_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let lifecycle = app.state::<SearchWindowLifecycleState>();
    match search_window_action(lifecycle.get().phase) {
        SearchWindowAction::ShowStandalone => show_search_window(app, &lifecycle)?,
        SearchWindowAction::HideStandalone => {
            hide_search_window_with_state(app, &lifecycle, None)?;
        }
    }
    Ok(())
}

#[cfg(any(feature = "desktop-runtime", test))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SearchWindowAction {
    ShowStandalone,
    HideStandalone,
}

#[cfg(any(feature = "desktop-runtime", test))]
fn search_window_action(phase: SearchWindowPhase) -> SearchWindowAction {
    match phase {
        SearchWindowPhase::Hidden => SearchWindowAction::ShowStandalone,
        SearchWindowPhase::Showing
        | SearchWindowPhase::VisibleCollapsed
        | SearchWindowPhase::VisibleExpanded
        | SearchWindowPhase::Hiding => SearchWindowAction::HideStandalone,
    }
}

#[cfg(feature = "desktop-runtime")]
fn show_search_window<R: Runtime>(
    app: &AppHandle<R>,
    lifecycle: &SearchWindowLifecycleState,
) -> Result<(), String> {
    lifecycle
        .show_with_native(
            || {
                let window = match app.get_webview_window(SEARCH_WINDOW_LABEL) {
                    Some(window) => window,
                    None => {
                        let snapshot = lifecycle.get();
                        lifecycle.record_creation_start(snapshot.session_id);
                        create_search_window(app)?
                    }
                };
                resize_search_window_for_state(app, false).map_err(|error| error.to_string())?;
                window.show().map_err(|error| error.to_string())?;
                window.set_focus().map_err(|error| error.to_string())?;
                Ok(())
            },
            |snapshot| emit_search_window_state(app, snapshot),
        )
        .map(|_| ())
}

#[cfg(feature = "desktop-runtime")]
fn hide_search_window_with_state<R: Runtime>(
    app: &AppHandle<R>,
    lifecycle: &SearchWindowLifecycleState,
    request: Option<&SearchWindowMutationRequest>,
) -> Result<SearchWindowSnapshot, String> {
    lifecycle.hide_with_native(
        request,
        || {
            let window = app
                .get_webview_window(SEARCH_WINDOW_LABEL)
                .ok_or_else(|| "search_window_missing".to_string())?;
            let exit_intent = app.state::<ExitIntentState>();
            let teardown = exit_intent.begin_internal_window_teardown(app.webview_windows().len());
            if let Err(error) = window.destroy() {
                drop(teardown);
                return Err(error.to_string());
            }
            teardown.complete();
            eprintln!(
                "ui_runtime search_window_destroyed webview_count={}",
                app.webview_windows().len()
            );
            Ok(())
        },
        |snapshot| emit_search_window_state(app, snapshot),
    )
}

#[cfg(not(feature = "desktop-runtime"))]
fn hide_search_window_with_state<R: Runtime>(
    app: &AppHandle<R>,
    lifecycle: &SearchWindowLifecycleState,
    request: Option<&SearchWindowMutationRequest>,
) -> Result<SearchWindowSnapshot, String> {
    lifecycle.hide_with_native(
        request,
        || Ok(()),
        |snapshot| emit_search_window_state(app, snapshot),
    )
}

#[cfg(feature = "desktop-runtime")]
fn emit_search_window_state<R: Runtime>(app: &AppHandle<R>, snapshot: &SearchWindowSnapshot) {
    let _ = app.emit_to(
        SEARCH_WINDOW_LABEL,
        SEARCH_WINDOW_STATE_EVENT,
        snapshot.clone(),
    );
}

#[cfg(not(feature = "desktop-runtime"))]
fn emit_search_window_state<R: Runtime>(_app: &AppHandle<R>, _snapshot: &SearchWindowSnapshot) {}

fn require_search_window<R: Runtime>(window: &WebviewWindow<R>) -> Result<(), String> {
    if window.label() == SEARCH_WINDOW_LABEL {
        Ok(())
    } else {
        Err("search_window_required".to_string())
    }
}

#[cfg(feature = "desktop-runtime")]
pub fn setup_tray(app: &mut App) -> tauri::Result<()> {
    let show_item = MenuItem::with_id(
        app,
        TRAY_SHOW_MAIN_WINDOW_ID,
        "显示主窗口",
        true,
        None::<&str>,
    )?;
    let quit_item = MenuItem::with_id(app, TRAY_QUIT_APP_ID, "退出", true, None::<&str>)?;
    #[cfg(feature = "native-qa")]
    eprintln!("native_qa startup_checkpoint=tray_menu_items_ready");

    let menu = Menu::with_items(app, &[&show_item, &quit_item])?;
    #[cfg(feature = "native-qa")]
    eprintln!("native_qa startup_checkpoint=tray_menu_ready");

    let icon = Image::from_bytes(TRAY_ICON_BYTES)?;
    #[cfg(feature = "native-qa")]
    eprintln!("native_qa startup_checkpoint=tray_icon_decoded");

    TrayIconBuilder::new()
        .icon(icon)
        .tooltip("Zen Canvas")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_SHOW_MAIN_WINDOW_ID => {
                if let Err(error) = show_main_window(app) {
                    eprintln!("Show main window from tray failed: {error}");
                }
            }
            TRAY_QUIT_APP_ID => exit_app(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                if let Err(error) = show_main_window(tray.app_handle()) {
                    eprintln!("Show main window from tray click failed: {error}");
                }
            }
        })
        .build(app)?;
    #[cfg(feature = "native-qa")]
    eprintln!("native_qa startup_checkpoint=tray_icon_built");

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc,
    };

    #[cfg(feature = "desktop-runtime")]
    #[test]
    fn latest_main_generation_reports_the_allocated_generation() {
        let lifecycle = MainWindowLifecycleState::default();

        assert_eq!(
            lifecycle.latest_generation().expect("initial generation"),
            0
        );
        assert_eq!(lifecycle.next_generation().expect("allocate generation"), 1);
        assert_eq!(lifecycle.latest_generation().expect("latest generation"), 1);
    }

    #[test]
    fn global_search_shortcut_matches_documented_accelerator() {
        assert_eq!(DEFAULT_SEARCH_HOTKEY, "CmdOrCtrl+K");
        assert_eq!(global_search_accelerator("Alt+Space"), "Alt+Space");
        assert_eq!(global_search_accelerator(""), DEFAULT_SEARCH_HOTKEY);
    }

    #[test]
    fn concurrent_search_show_creates_once_and_reopen_uses_a_fresh_session() {
        let lifecycle = Arc::new(SearchWindowLifecycleState::default());
        let create_count = Arc::new(AtomicUsize::new(0));
        let (first_started_tx, first_started_rx) = mpsc::channel();
        let (release_first_tx, release_first_rx) = mpsc::channel();
        let first_lifecycle = Arc::clone(&lifecycle);
        let first_count = Arc::clone(&create_count);
        let first_show = std::thread::spawn(move || {
            first_lifecycle.show_with_native(
                || {
                    first_count.fetch_add(1, Ordering::SeqCst);
                    first_started_tx.send(()).expect("signal first show");
                    release_first_rx.recv().expect("release first show");
                    Ok(())
                },
                |_| {},
            )
        });
        first_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("first show owns the native create");

        let second_lifecycle = Arc::clone(&lifecycle);
        let second_count = Arc::clone(&create_count);
        let second_show = std::thread::spawn(move || {
            second_lifecycle.show_with_native(
                || {
                    second_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                |_| {},
            )
        });
        release_first_tx.send(()).expect("release first show");
        let first = first_show
            .join()
            .expect("first show thread")
            .expect("first show");
        assert_eq!(
            second_show.join().expect("second show thread"),
            Err("search_window_already_visible".to_string())
        );
        assert_eq!(create_count.load(Ordering::SeqCst), 1);
        assert_eq!(first.session_id, 1);

        let destroy_count = AtomicUsize::new(0);
        let hidden = lifecycle
            .hide_with_native(
                None,
                || {
                    destroy_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                |_| {},
            )
            .expect("destroy Search");
        assert_eq!(hidden.phase, SearchWindowPhase::Hidden);
        assert_eq!(destroy_count.load(Ordering::SeqCst), 1);

        let reopened = lifecycle
            .show_with_native(
                || {
                    create_count.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                },
                |_| {},
            )
            .expect("create a fresh Search instance");
        assert_eq!(reopened.session_id, 2);
        assert_eq!(create_count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn manual_second_instance_activates_main_but_background_and_unknown_args_do_not() {
        let executable = "zen-canvas.exe".to_string();
        assert_eq!(
            second_instance_action(std::slice::from_ref(&executable)),
            SecondInstanceAction::ActivateMain
        );
        assert_eq!(
            second_instance_action(&[executable.clone(), "--background".to_string()]),
            SecondInstanceAction::IgnoreBackground
        );
        assert_eq!(
            second_instance_action(&[
                executable,
                "--open".to_string(),
                "arbitrary-path".to_string()
            ]),
            SecondInstanceAction::IgnoreUnsupportedArguments
        );
    }

    #[test]
    fn single_instance_diagnostic_omits_path_values_and_working_directory() {
        let path_argument = r"C:\Users\Private\Documents\report.txt";
        let working_directory = r"C:\Users\Private\Documents";
        let classification =
            classify_second_instance(&["zen-canvas.exe".to_string(), path_argument.to_string()]);
        let diagnostic = format!("{:?}", classification.diagnostic);

        assert_eq!(
            classification.action,
            SecondInstanceAction::IgnoreUnsupportedArguments
        );
        assert_eq!(classification.diagnostic.arg_count, 1);
        assert!(classification.diagnostic.unsupported_arguments);
        assert!(!diagnostic.contains(path_argument));
        assert!(!diagnostic.contains(working_directory));
    }

    #[test]
    fn second_instance_diagnostic_preserves_supported_classifications() {
        let manual = classify_second_instance(&["zen-canvas.exe".to_string()]);
        assert_eq!(manual.action, SecondInstanceAction::ActivateMain);
        assert_eq!(manual.diagnostic.arg_count, 0);
        assert!(!manual.diagnostic.background_flag_present);
        assert!(!manual.diagnostic.unsupported_arguments);

        let background =
            classify_second_instance(&["zen-canvas.exe".to_string(), "--background".to_string()]);
        assert_eq!(background.action, SecondInstanceAction::IgnoreBackground);
        assert_eq!(background.diagnostic.arg_count, 1);
        assert!(background.diagnostic.background_flag_present);
        assert!(!background.diagnostic.unsupported_arguments);
    }

    #[test]
    fn unsupported_second_instance_arguments_remain_fail_closed() {
        let classification = classify_second_instance(&[
            "zen-canvas.exe".to_string(),
            "--background".to_string(),
            "--open".to_string(),
            r"C:\Users\Private\report.txt".to_string(),
        ]);
        assert_eq!(
            classification.action,
            SecondInstanceAction::IgnoreUnsupportedArguments
        );
        assert!(classification.diagnostic.background_flag_present);
        assert!(classification.diagnostic.unsupported_arguments);
    }

    #[test]
    fn background_launch_requires_the_exact_supported_argument_shape() {
        assert!(is_background_launch_args(&[
            "zen-canvas.exe".to_string(),
            "--background".to_string()
        ]));
        assert!(!is_background_launch_args(&["zen-canvas.exe".to_string()]));
        assert!(!is_background_launch_args(&[
            "zen-canvas.exe".to_string(),
            "--background".to_string(),
            "--open".to_string()
        ]));
    }

    #[cfg(feature = "desktop-runtime")]
    #[test]
    fn main_reopen_advances_generation_and_restores_small_view_state() {
        let lifecycle = MainWindowLifecycleState::default();
        let session = MainWindowSessionState::default();
        assert_eq!(lifecycle.next_generation().expect("first Main"), 1);
        session
            .set_last_view(SearchView::Cleanup)
            .expect("store last view");
        assert_eq!(
            session.last_view().expect("restore last view"),
            SearchView::Cleanup
        );
        assert_eq!(lifecycle.next_generation().expect("reopened Main"), 2);
    }

    #[cfg(feature = "desktop-runtime")]
    #[test]
    fn main_readiness_is_restored_when_workspace_teardown_fails() {
        let readiness = MainWindowReadinessState::default();
        readiness
            .begin_generation(3)
            .expect("begin Main generation");
        readiness.set_ready(3, true).expect("mark Main ready");
        readiness.set_ready(3, false).expect("begin Main teardown");

        let error = restore_readiness_after_workspace_dispose_failure(
            &readiness,
            3,
            "native_preview_host_dispose_failed".to_string(),
        );

        assert_eq!(error, "native_preview_host_dispose_failed");
        assert_eq!(
            readiness.begin_request(Duration::from_millis(1)),
            Ok((3, 1))
        );
    }

    #[cfg(feature = "desktop-runtime")]
    #[test]
    fn global_search_shortcut_parses_for_registration() {
        assert!(global_search_shortcut(DEFAULT_SEARCH_HOTKEY).is_ok());
    }

    #[test]
    fn search_window_url_targets_standalone_search_mode() {
        assert_eq!(search_window_url(), "index.html?mode=search");
    }

    #[test]
    fn search_window_action_is_owned_by_the_durable_phase() {
        assert_eq!(
            search_window_action(SearchWindowPhase::Hidden),
            SearchWindowAction::ShowStandalone
        );
        assert_eq!(
            search_window_action(SearchWindowPhase::Showing),
            SearchWindowAction::HideStandalone
        );
        assert_eq!(
            search_window_action(SearchWindowPhase::VisibleCollapsed),
            SearchWindowAction::HideStandalone
        );
    }

    #[test]
    fn search_window_rejects_old_session_and_revision_mutations() {
        let lifecycle = SearchWindowLifecycleState::default();
        let showing = lifecycle.begin_show().expect("begin show");
        assert_eq!(showing.phase, SearchWindowPhase::Showing);
        assert_eq!(
            lifecycle
                .complete_show(showing.session_id.saturating_sub(1), showing.revision)
                .expect_err("old session rejected"),
            "search_window_session_stale"
        );
        let visible = lifecycle
            .complete_show(showing.session_id, showing.revision)
            .expect("complete show");
        let stale_resize = SearchWindowResizeRequest {
            session_id: visible.session_id,
            expected_revision: visible.revision.saturating_sub(1),
            expanded: true,
        };
        assert_eq!(
            lifecycle
                .resize(&stale_resize)
                .expect_err("old revision rejected"),
            "search_window_revision_stale"
        );
    }

    #[test]
    fn search_window_lifecycle_hides_and_reopens_with_a_new_session() {
        let lifecycle = SearchWindowLifecycleState::default();
        let showing = lifecycle.begin_show().expect("begin show");
        let visible = lifecycle
            .complete_show(showing.session_id, showing.revision)
            .expect("complete show");
        let expanded = lifecycle
            .resize(&SearchWindowResizeRequest {
                session_id: visible.session_id,
                expected_revision: visible.revision,
                expanded: true,
            })
            .expect("expand");
        let hiding = lifecycle.begin_hide(None).expect("begin hide");
        assert_eq!(hiding.phase, SearchWindowPhase::Hiding);
        assert_eq!(
            lifecycle.begin_hide(None).expect("retry begin hide"),
            hiding
        );
        let hidden = lifecycle
            .complete_hide(hiding.session_id, hiding.revision)
            .expect("complete hide");
        assert_eq!(hidden.phase, SearchWindowPhase::Hidden);
        assert!(hidden.revision > expanded.revision);

        let reopened = lifecycle.begin_show().expect("reopen");
        assert!(reopened.session_id > hidden.session_id);
        assert_eq!(reopened.phase, SearchWindowPhase::Showing);
    }

    #[test]
    fn stale_resize_is_rejected_before_the_native_adapter_runs() {
        let lifecycle = SearchWindowLifecycleState::default();
        let showing = lifecycle.begin_show().expect("begin show");
        let visible = lifecycle
            .complete_show(showing.session_id, showing.revision)
            .expect("complete show");
        let calls = AtomicUsize::new(0);
        let stale = SearchWindowResizeRequest {
            session_id: visible.session_id,
            expected_revision: visible.revision.saturating_sub(1),
            expanded: true,
        };

        assert_eq!(
            lifecycle
                .resize_with_native(&stale, || {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                })
                .expect_err("stale resize"),
            "search_window_revision_stale"
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn lifecycle_owner_serializes_native_resize_and_rejects_old_race() {
        let lifecycle = Arc::new(SearchWindowLifecycleState::default());
        let showing = lifecycle.begin_show().expect("begin show");
        let visible = lifecycle
            .complete_show(showing.session_id, showing.revision)
            .expect("complete show");
        let old_request = SearchWindowResizeRequest {
            session_id: visible.session_id,
            expected_revision: visible.revision,
            expanded: true,
        };
        let new_request = old_request.clone();
        let native_calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let new_lifecycle = Arc::clone(&lifecycle);
        let new_calls = Arc::clone(&native_calls);
        let newer = std::thread::spawn(move || {
            new_lifecycle.resize_with_native(&new_request, || {
                new_calls.fetch_add(1, Ordering::SeqCst);
                entered_tx.send(()).expect("notify native entry");
                release_rx.recv().expect("release native adapter");
                Ok(())
            })
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("new request owns native adapter");

        let old_lifecycle = Arc::clone(&lifecycle);
        let old_calls = Arc::clone(&native_calls);
        let older = std::thread::spawn(move || {
            old_lifecycle.resize_with_native(&old_request, || {
                old_calls.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        });
        release_tx.send(()).expect("release new native adapter");

        newer
            .join()
            .expect("newer request thread")
            .expect("newer resize succeeds");
        assert_eq!(
            older.join().expect("older request thread"),
            Err("search_window_revision_stale".to_string())
        );
        assert_eq!(native_calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn native_failure_restores_retryable_phase_without_stuck_transition() {
        let lifecycle = SearchWindowLifecycleState::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let first_events = Arc::clone(&events);
        assert_eq!(
            lifecycle
                .show_with_native(
                    || Err("native_show_failed".to_string()),
                    |snapshot| first_events.lock().unwrap().push(snapshot.phase),
                )
                .expect_err("show failure"),
            "native_show_failed"
        );
        assert_eq!(lifecycle.get().phase, SearchWindowPhase::Hidden);

        let showing = lifecycle
            .show_with_native(|| Ok(()), |_| {})
            .expect("retry show");
        let visible = lifecycle
            .complete_show(showing.session_id, showing.revision)
            .expect("complete retry show");
        let second_events = Arc::clone(&events);
        assert_eq!(
            lifecycle
                .hide_with_native(
                    None,
                    || Err("native_hide_failed".to_string()),
                    |snapshot| second_events.lock().unwrap().push(snapshot.phase),
                )
                .expect_err("hide failure"),
            "native_hide_failed"
        );
        assert_eq!(lifecycle.get(), visible);
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[
                SearchWindowPhase::Hiding,
                SearchWindowPhase::VisibleCollapsed
            ]
        );
    }

    #[test]
    fn main_window_readiness_rejects_stale_nonce_and_accepts_current_ack() {
        let readiness = MainWindowReadinessState::default();
        readiness
            .begin_generation(3)
            .expect("begin Main generation");
        readiness.set_ready(3, true).expect("mark ready");
        let (generation, nonce) = readiness
            .begin_request(Duration::from_millis(5))
            .expect("ready request");
        assert_eq!(generation, 3);
        assert_eq!(
            readiness.acknowledge(generation, nonce.saturating_add(1)),
            Err("main_window_ready_nonce_stale".to_string())
        );
        readiness
            .acknowledge(generation, nonce)
            .expect("ack current nonce");
        readiness
            .wait_for_ack(generation, nonce, Duration::from_millis(5))
            .expect("observe ack");
        assert_eq!(
            readiness.acknowledge(generation + 1, nonce),
            Err("main_window_generation_stale".to_string())
        );
    }

    #[test]
    fn main_window_readiness_fails_closed_when_renderer_is_not_ready() {
        let readiness = MainWindowReadinessState::default();
        assert_eq!(
            readiness.begin_request(Duration::from_millis(1)),
            Err("main_window_not_ready".to_string())
        );
    }

    #[test]
    fn search_navigation_payload_serializes_camel_case_file_id() {
        let payload = SearchNavigatePayload::new(SearchView::Settings, None).with_window_context(
            Some(7),
            Some(12),
            Some(SearchSettingsTarget::GlobalIndex),
        );
        let value = serde_json::to_value(payload).expect("serialize search navigation payload");

        assert_eq!(value["view"], "settings");
        assert_eq!(value["fileId"], serde_json::Value::Null);
        assert_eq!(value["nonce"], 0);
        assert_eq!(value["sessionId"], 7);
        assert_eq!(value["revision"], 12);
        assert_eq!(value["settingsTarget"], "global-index");
        assert!(
            serde_json::from_value::<SearchSettingsTarget>(serde_json::json!("arbitrary")).is_err()
        );
    }

    #[test]
    fn search_view_uses_automation_canonically_and_accepts_legacy_rules() {
        let canonical = serde_json::from_value::<SearchView>(serde_json::json!("automation"))
            .expect("deserialize canonical Automation route");
        let legacy = serde_json::from_value::<SearchView>(serde_json::json!("rules"))
            .expect("deserialize legacy Rules route");

        assert_eq!(canonical, SearchView::Automation);
        assert_eq!(legacy, SearchView::Automation);
        assert_eq!(
            serde_json::to_value(SearchView::Automation).expect("serialize Automation route"),
            serde_json::json!("automation")
        );
    }

    #[test]
    fn search_view_preserves_existing_route_wire_names() {
        let routes = [
            (SearchView::Scanner, "scanner"),
            (SearchView::Cleanup, "cleanup"),
            (SearchView::Organize, "organize"),
            (SearchView::Library, "library"),
            (SearchView::Preview, "preview"),
            (SearchView::Restore, "restore"),
            (SearchView::Settings, "settings"),
        ];

        for (view, wire_value) in routes {
            assert_eq!(
                serde_json::to_value(view).expect("serialize existing route"),
                serde_json::json!(wire_value)
            );
            assert_eq!(
                serde_json::from_value::<SearchView>(serde_json::json!(wire_value))
                    .expect("deserialize existing route"),
                view
            );
        }
    }

    #[test]
    fn search_navigation_payload_serializes_automation_route_canonically() {
        let payload = SearchNavigatePayload::new(SearchView::Automation, None);
        let value = serde_json::to_value(payload).expect("serialize Automation navigation");

        assert_eq!(value["view"], "automation");
    }

    #[test]
    fn activate_search_result_request_accepts_canonical_and_legacy_automation_routes() {
        for wire_value in ["automation", "rules"] {
            let request =
                serde_json::from_value::<ActivateSearchResultRequest>(serde_json::json!({
                    "sessionId": 7,
                    "expectedRevision": 12,
                    "view": wire_value,
                    "fileId": null,
                    "settingsTarget": null
                }))
                .expect("deserialize search activation request");

            assert_eq!(request.view, SearchView::Automation);
        }
    }

    #[cfg(feature = "desktop-runtime")]
    #[test]
    fn main_window_restore_uses_the_canonical_automation_route() {
        let session = MainWindowSessionState::default();
        session
            .set_last_view(SearchView::Automation)
            .expect("store last Automation view");

        let url = main_window_restore_url(
            7,
            session.last_view().expect("restore last Automation view"),
        );

        assert_eq!(url, "index.html?mainGeneration=7&view=automation");
    }

    #[test]
    fn main_window_ready_request_carries_search_context_for_renderer_parity() {
        let value = serde_json::to_value(MainWindowReadyRequest {
            nonce: 21,
            generation: 4,
            session_id: Some(7),
            revision: Some(12),
        })
        .expect("serialize main window ready request");

        assert_eq!(value["nonce"], 21);
        assert_eq!(value["generation"], 4);
        assert_eq!(value["sessionId"], 7);
        assert_eq!(value["revision"], 12);
    }

    #[test]
    fn hotkey_registration_failure_restores_previous_status_when_fallback_succeeds() {
        let rollback = hotkey_registration_failure_with_rollback(
            "Alt+Space".to_string(),
            "shortcut already registered".to_string(),
            Some("CmdOrCtrl+K".to_string()),
            None,
            2,
        );

        assert!(rollback.returned_status.registered);
        assert_eq!(rollback.returned_status.requested_accelerator, "Alt+Space");
        assert_eq!(
            rollback.returned_status.effective_accelerator.as_deref(),
            Some("CmdOrCtrl+K")
        );
        assert!(rollback
            .returned_status
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("restored previous hotkey CmdOrCtrl+K"));
        assert_eq!(rollback.state_status, rollback.returned_status);
    }

    #[test]
    fn hotkey_registration_failure_keeps_failure_status_when_fallback_fails() {
        let rollback = hotkey_registration_failure_with_rollback(
            "Alt+Space".to_string(),
            "shortcut already registered".to_string(),
            Some("CmdOrCtrl+K".to_string()),
            Some("restore failed".to_string()),
            2,
        );

        assert!(!rollback.returned_status.registered);
        assert_eq!(rollback.returned_status.requested_accelerator, "Alt+Space");
        assert_eq!(rollback.returned_status.effective_accelerator, None);
        assert!(rollback
            .returned_status
            .error
            .as_deref()
            .unwrap_or_default()
            .contains("restore previous hotkey failed"));
        assert_eq!(rollback.state_status, rollback.returned_status);
    }

    #[test]
    fn hotkey_same_effective_value_is_idempotent_only_for_a_healthy_registration() {
        let healthy = GlobalHotkeyStatus {
            requested_accelerator: "CmdOrCtrl+K".to_string(),
            effective_accelerator: Some("CmdOrCtrl+K".to_string()),
            registered: true,
            error: None,
            revision: 4,
        };
        assert!(hotkey_registration_is_idempotent(&healthy, "CmdOrCtrl+K"));
        assert!(!hotkey_registration_is_idempotent(&healthy, "Alt+Space"));

        let failed = GlobalHotkeyStatus {
            error: Some("registration failed".to_string()),
            ..healthy
        };
        assert!(!hotkey_registration_is_idempotent(&failed, "CmdOrCtrl+K"));
    }
    fn handoff_fixture() -> (
        SearchWindowLifecycleState,
        MainWindowReadinessState,
        SearchWindowSnapshot,
        SearchNavigatePayload,
    ) {
        let search = SearchWindowLifecycleState::default();
        let showing = search.begin_show().unwrap();
        let visible = search
            .complete_show(showing.session_id, showing.revision)
            .unwrap();
        let main = MainWindowReadinessState::default();
        main.begin_generation(7).unwrap();
        main.set_ready(7, true).unwrap();
        let payload = SearchNavigatePayload::new(SearchView::Automation, None).with_window_context(
            Some(visible.session_id),
            Some(visible.revision),
            None,
        );
        (search, main, visible, payload)
    }

    fn applied_ack(
        payload: &SearchNavigatePayload,
        applied: bool,
    ) -> SearchNavigationAcknowledgement {
        SearchNavigationAcknowledgement {
            generation: payload.generation,
            nonce: payload.nonce,
            session_id: payload.session_id,
            revision: payload.revision,
            applied,
        }
    }

    #[test]
    fn handoff_success_requires_ready_then_navigation_commit_then_original_scoped_hide() {
        let (search, main, original, payload) = handoff_fixture();
        let stages = std::cell::RefCell::new(Vec::new());
        handoff_search_navigation(
            &search,
            &main,
            payload,
            Duration::from_millis(100),
            |request| {
                stages.borrow_mut().push("ready");
                main.acknowledge(request.generation, request.nonce)
            },
            |payload| {
                assert_eq!(search.get(), original); // no premature native hide
                assert_eq!(serde_json::to_value(payload).unwrap()["view"], "automation");
                stages.borrow_mut().push("navigation");
                main.acknowledge_navigation(applied_ack(payload, true))
            },
            |request| {
                assert_eq!(request.session_id, original.session_id);
                assert_eq!(request.expected_revision, original.revision);
                stages.borrow_mut().push("scoped_hide");
                search
                    .hide_with_native(Some(request), || Ok(()), |_| {})
                    .map(|_| ())
            },
        )
        .unwrap();
        assert_eq!(*stages.borrow(), ["ready", "navigation", "scoped_hide"]);
        assert_eq!(search.get().phase, SearchWindowPhase::Hidden);
    }

    #[test]
    fn handoff_timeout_and_rejection_keep_search_retryable_and_reject_late_ack() {
        for outcome in [None, Some(false)] {
            let (search, main, original, payload) = handoff_fixture();
            let emitted = std::cell::RefCell::new(None);
            let result = handoff_search_navigation(
                &search,
                &main,
                payload,
                Duration::from_millis(5),
                |request| main.acknowledge(request.generation, request.nonce),
                |payload| {
                    assert_eq!(search.get(), original);
                    *emitted.borrow_mut() = Some(payload.clone());
                    if let Some(applied) = outcome {
                        main.acknowledge_navigation(applied_ack(payload, applied))?;
                    }
                    Ok(())
                },
                |_| panic!("failed navigation must never hide"),
            );
            assert_eq!(
                result.unwrap_err(),
                if outcome.is_some() {
                    "search_navigation_rejected"
                } else {
                    "search_navigation_ack_timeout"
                }
            );
            assert_eq!(search.get(), original);
            assert!(main
                .acknowledge_navigation(applied_ack(emitted.borrow().as_ref().unwrap(), true))
                .is_err());
        }
    }

    #[test]
    fn handoff_revalidates_revision_and_phase_after_ready_before_emit() {
        for hide in [false, true] {
            let (search, main, original, payload) = handoff_fixture();
            let result = handoff_search_navigation(
                &search,
                &main,
                payload,
                Duration::from_millis(100),
                |request| {
                    main.acknowledge(request.generation, request.nonce)?;
                    if hide {
                        search.hide_with_native(
                            Some(&SearchWindowMutationRequest {
                                session_id: original.session_id,
                                expected_revision: original.revision,
                            }),
                            || Ok(()),
                            |_| {},
                        )?;
                    } else {
                        search.resize(&SearchWindowResizeRequest {
                            session_id: original.session_id,
                            expected_revision: original.revision,
                            expanded: true,
                        })?;
                    }
                    Ok(())
                },
                |_| panic!("stale post-ready Search must not emit navigation"),
                |_| panic!("must not hide"),
            );
            assert!(result.is_err());
        }
        let (search, main, original, payload) = handoff_fixture();
        search.snapshot.lock().unwrap().phase = SearchWindowPhase::Showing;
        assert_eq!(
            handoff_search_navigation(
                &search,
                &main,
                payload,
                Duration::from_millis(5),
                |_| panic!("invalid phase"),
                |_| panic!("invalid phase"),
                |_| panic!("invalid phase")
            )
            .unwrap_err(),
            "search_window_phase_stale"
        );
        assert_eq!(search.get().revision, original.revision);
    }

    #[test]
    fn handoff_reopened_or_resized_search_during_commit_wait_is_never_hidden() {
        for reopen in [false, true] {
            let (search, main, original, payload) = handoff_fixture();
            std::thread::scope(|scope| {
                let (tx, rx) = mpsc::channel::<SearchNavigatePayload>();
                let search = &search;
                let main = &main;
                let original = &original;
                let worker = scope.spawn(move || {
                    let emitted = rx.recv_timeout(Duration::from_secs(1)).unwrap();
                    if reopen {
                        search
                            .hide_with_native(
                                Some(&SearchWindowMutationRequest {
                                    session_id: original.session_id,
                                    expected_revision: original.revision,
                                }),
                                || Ok(()),
                                |_| {},
                            )
                            .unwrap();
                        let showing = search.begin_show().unwrap();
                        search
                            .complete_show(showing.session_id, showing.revision)
                            .unwrap();
                    } else {
                        search
                            .resize(&SearchWindowResizeRequest {
                                session_id: original.session_id,
                                expected_revision: original.revision,
                                expanded: true,
                            })
                            .unwrap();
                    }
                    main.acknowledge_navigation(applied_ack(&emitted, true))
                        .unwrap();
                });
                let result = handoff_search_navigation(
                    search,
                    main,
                    payload,
                    Duration::from_secs(1),
                    |request| main.acknowledge(request.generation, request.nonce),
                    |payload| {
                        assert_eq!(search.get(), *original);
                        tx.send(payload.clone()).unwrap();
                        Ok(())
                    },
                    |_| panic!("old handoff cannot hide changed Search"),
                );
                assert!(result.is_err());
                worker.join().unwrap();
            });
            assert_ne!(search.get().phase, SearchWindowPhase::Hidden);
            if reopen {
                assert!(search.get().session_id > original.session_id);
            }
        }
    }

    #[test]
    fn handoff_main_disappearance_or_recreation_and_mismatched_ack_fail_closed() {
        for recreate in [false, true] {
            let (search, main, original, payload) = handoff_fixture();
            let result = handoff_search_navigation(
                &search,
                &main,
                payload,
                Duration::from_millis(100),
                |request| main.acknowledge(request.generation, request.nonce),
                |payload| {
                    let good = applied_ack(payload, true);
                    for bad in [
                        SearchNavigationAcknowledgement {
                            generation: 8,
                            ..good.clone()
                        },
                        SearchNavigationAcknowledgement {
                            nonce: 2,
                            ..good.clone()
                        },
                        SearchNavigationAcknowledgement {
                            session_id: Some(99),
                            ..good.clone()
                        },
                        SearchNavigationAcknowledgement {
                            revision: Some(99),
                            ..good.clone()
                        },
                    ] {
                        assert!(main.acknowledge_navigation(bad).is_err());
                    }
                    if recreate {
                        main.begin_generation(8)?;
                        main.set_ready(8, true)?;
                    } else {
                        main.set_ready(7, false)?;
                    }
                    assert!(main.acknowledge_navigation(good).is_err());
                    Ok(())
                },
                |_| panic!("disappeared Main cannot hide Search"),
            );
            assert_eq!(result.unwrap_err(), "search_navigation_main_stale");
            assert_eq!(search.get(), original);
        }
    }
    #[test]
    fn handoff_emit_failure_disarms_ack_and_scoped_hide_rejects_last_moment_reopen() {
        let (search, main, original, payload) = handoff_fixture();
        let emitted = std::cell::RefCell::new(None);
        let result = handoff_search_navigation(
            &search,
            &main,
            payload.clone(),
            Duration::from_millis(100),
            |request| main.acknowledge(request.generation, request.nonce),
            |payload| {
                *emitted.borrow_mut() = Some(payload.clone());
                Err("emit_failed".to_string())
            },
            |_| panic!("emit failure cannot hide"),
        );
        assert_eq!(result.unwrap_err(), "emit_failed");
        assert!(main
            .acknowledge_navigation(applied_ack(emitted.borrow().as_ref().unwrap(), true))
            .is_err());
        assert_eq!(search.get(), original);
        let result = handoff_search_navigation(
            &search,
            &main,
            payload,
            Duration::from_millis(100),
            |request| main.acknowledge(request.generation, request.nonce),
            |payload| main.acknowledge_navigation(applied_ack(payload, true)),
            |request| {
                search.hide_with_native(Some(request), || Ok(()), |_| {})?;
                let showing = search.begin_show()?;
                search.complete_show(showing.session_id, showing.revision)?;
                search
                    .hide_with_native(
                        Some(request),
                        || panic!("new Search cannot be destroyed"),
                        |_| {},
                    )
                    .map(|_| ())
            },
        );
        assert_eq!(result.unwrap_err(), "search_window_session_stale");
        assert_eq!(search.get().phase, SearchWindowPhase::VisibleCollapsed);
    }

    #[test]
    fn navigation_ack_dto_rejects_renderer_authority_expansion() {
        assert!(serde_json::from_value::<SearchNavigationAcknowledgement>(serde_json::json!({
            "generation": 7, "nonce": 1, "sessionId": 4, "revision": 9, "applied": true, "path": "forbidden"
        })).is_err());
    }
}
