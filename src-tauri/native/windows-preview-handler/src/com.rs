use std::{
    cell::RefCell,
    ffi::c_void,
    ptr::null_mut,
    rc::Rc,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::ThreadId,
};

use fylura_native_host::{
    BoundedContentRead, HostProvidedHandle, HostProvidedHost, HostProvidedReadContext,
    HostProvidedReadRequest, HostProvidedReadSource, HostProvidedRegistration,
    HostProvidedRegistry, HostProvidedSourceError,
};
use fylura_preview_representation::{self, RepresentationCompleteness, SafeRepresentation};
use uuid::Uuid;
use windows::{
    core::{implement, Error, IUnknown, Interface, Ref, Result, BOOL, GUID, HRESULT},
    Win32::{
        Foundation::{COLORREF, HWND, RECT},
        Graphics::Gdi::{DeleteObject, HFONT, LOGFONTW},
        System::{
            Com::{IClassFactory, IClassFactory_Impl, IStream},
            Ole::{IObjectWithSite, IObjectWithSite_Impl, IOleWindow, IOleWindow_Impl},
        },
        UI::{
            Shell::PropertiesSystem::{IInitializeWithStream, IInitializeWithStream_Impl},
            Shell::{
                IPreviewHandler, IPreviewHandlerFrame, IPreviewHandlerVisuals,
                IPreviewHandlerVisuals_Impl, IPreviewHandler_Impl,
            },
            WindowsAndMessaging::MSG,
        },
    },
};

use crate::{
    capture::{self, IStreamCaptureReader},
    class_id_is_supported,
    completion::{self, CompletionWindow, DeferredCompletion, DeferredPreview},
    host_registry,
    state::SharedHandlerState,
    window, ACTIVE_DEFERRED, ACTIVE_OBJECTS, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION,
    E_ABORT, E_FAIL, E_NOTIMPL, E_POINTER, E_UNEXPECTED, S_FALSE, S_OK,
};

const MAX_SURFACE_TEXT_BYTES: usize = capture::MAX_CAPTURE_BYTES;

#[derive(Default)]
struct VisualState {
    background_color: Option<COLORREF>,
    text_color: Option<COLORREF>,
    font: Option<HFONT>,
}

impl Drop for VisualState {
    fn drop(&mut self) {
        if let Some(font) = self.font.take().filter(|font| !font.is_invalid()) {
            unsafe {
                let _ = DeleteObject(font.into());
            }
        }
    }
}

fn error(hr: HRESULT, message: &'static str) -> Error {
    Error::new(hr, message)
}

fn owner_thread_error() -> Error {
    error(
        E_UNEXPECTED,
        "preview handler method was called from the wrong owner STA",
    )
}

#[implement(IClassFactory)]
struct ClassFactory;

impl ClassFactory {
    fn new() -> Self {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ClassFactory {
    fn drop(&mut self) {
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl IClassFactory_Impl for ClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Ref<'_, IUnknown>,
        riid: *const GUID,
        ppvobject: *mut *mut c_void,
    ) -> Result<()> {
        if !punkouter.is_null() {
            return Err(error(
                CLASS_E_NOAGGREGATION,
                "preview handler does not aggregate",
            ));
        }
        if riid.is_null() || ppvobject.is_null() {
            return Err(error(E_POINTER, "null COM output pointer"));
        }
        unsafe {
            *ppvobject = null_mut();
        }
        let object: IUnknown = PreviewHandler::new().into();
        let status = unsafe { object.query(riid, ppvobject) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(Error::from_hresult(status))
        }
    }

    fn LockServer(&self, flock: BOOL) -> Result<()> {
        if flock.as_bool() {
            crate::SERVER_LOCKS.fetch_add(1, Ordering::AcqRel);
        } else {
            let _ =
                crate::SERVER_LOCKS.fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                    value.checked_sub(1)
                });
        }
        Ok(())
    }
}

pub(crate) fn dll_get_class_object(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    unsafe {
        *ppv = null_mut();
        if !class_id_is_supported(&*rclsid) {
            return CLASS_E_CLASSNOTAVAILABLE;
        }
        let factory: IUnknown = ClassFactory::new().into();
        factory.query(riid, ppv)
    }
}

struct MemorySource {
    bytes: Arc<[u8]>,
    complete: bool,
}

impl HostProvidedReadSource for MemorySource {
    fn read_bounded(
        &self,
        offset_bytes: u64,
        max_bytes: u32,
        context: &HostProvidedReadContext,
    ) -> std::result::Result<BoundedContentRead, HostProvidedSourceError> {
        if context.is_cancelled() {
            return Err(HostProvidedSourceError::Cancelled);
        }
        let start = usize::try_from(offset_bytes).map_err(|_| HostProvidedSourceError::Failed)?;
        let end = start
            .saturating_add(max_bytes as usize)
            .min(self.bytes.len());
        let bytes = self.bytes.get(start..end).unwrap_or_default().to_vec();
        Ok(BoundedContentRead {
            complete: self.complete && end >= self.bytes.len(),
            bytes,
        })
    }
}

struct ActiveDeferredGuard;

impl ActiveDeferredGuard {
    fn new() -> Self {
        ACTIVE_DEFERRED.fetch_add(1, Ordering::AcqRel);
        Self
    }
}

impl Drop for ActiveDeferredGuard {
    fn drop(&mut self) {
        ACTIVE_DEFERRED.fetch_sub(1, Ordering::AcqRel);
    }
}

#[implement(
    IInitializeWithStream,
    IPreviewHandler,
    IPreviewHandlerVisuals,
    IOleWindow,
    IObjectWithSite,
    Agile = false
)]
pub(crate) struct PreviewHandler {
    state: SharedHandlerState,
    registry: Arc<HostProvidedRegistry>,
    owner_thread: ThreadId,
    completion_window: RefCell<Option<CompletionWindow>>,
    visuals: RefCell<VisualState>,
}

impl PreviewHandler {
    fn new() -> Self {
        ACTIVE_OBJECTS.fetch_add(1, Ordering::AcqRel);
        Self {
            state: SharedHandlerState::default(),
            registry: host_registry(),
            owner_thread: std::thread::current().id(),
            completion_window: RefCell::new(None),
            visuals: RefCell::new(VisualState::default()),
        }
    }

    fn ensure_owner_thread(&self) -> Result<()> {
        if std::thread::current().id() == self.owner_thread {
            Ok(())
        } else {
            Err(owner_thread_error())
        }
    }

    fn completion_target(&self) -> Result<isize> {
        self.ensure_owner_thread()?;
        if self.completion_window.borrow().is_none() {
            self.completion_window
                .borrow_mut()
                .replace(CompletionWindow::create(self as *const Self)?);
        }
        Ok(self
            .completion_window
            .borrow()
            .as_ref()
            .expect("completion window was just created")
            .raw_handle())
    }

    fn set_window(&self, parent: HWND, rect: RECT) -> Result<()> {
        self.ensure_owner_thread()?;
        if parent.is_invalid() {
            return Err(error(E_POINTER, "preview parent window is null"));
        }
        let child = {
            let mut state = self.state.borrow_mut();
            if !state.initialized {
                return Err(error(E_UNEXPECTED, "preview stream is not initialized"));
            }
            state.parent = parent;
            state.rect = rect;
            state.child
        };
        if let Some(child) = child {
            window::reparent_surface(child, parent)?;
            window::resize_surface(child, rect)?;
        }
        Ok(())
    }

    fn set_rect(&self, rect: RECT) -> Result<()> {
        self.ensure_owner_thread()?;
        let child = {
            let mut state = self.state.borrow_mut();
            if !state.initialized {
                return Err(error(E_UNEXPECTED, "preview stream is not initialized"));
            }
            state.rect = rect;
            state.child
        };
        if let Some(child) = child {
            window::resize_surface(child, rect)?;
        }
        Ok(())
    }

    fn do_preview(&self) -> Result<()> {
        self.ensure_owner_thread()?;
        let (generation_id, parent, rect, existing_child) = {
            let state = self.state.borrow();
            if !state.initialized {
                return Err(error(E_UNEXPECTED, "preview stream is not initialized"));
            }
            if state.preview_started {
                return Ok(());
            }
            (
                state
                    .generation_id
                    .clone()
                    .ok_or_else(|| error(E_UNEXPECTED, "preview generation is missing"))?,
                state.parent,
                state.rect,
                state.child,
            )
        };
        if parent.is_invalid() {
            return Err(error(E_UNEXPECTED, "preview window is not attached"));
        }

        let child = if let Some(child) = existing_child {
            child
        } else {
            let child = window::create_surface(parent, rect)?;
            let keep = {
                let mut state = self.state.borrow_mut();
                if !state.initialized
                    || state.generation_id.as_deref() != Some(generation_id.as_str())
                    || state.child.is_some()
                {
                    false
                } else {
                    state.child = Some(child);
                    true
                }
            };
            if !keep {
                window::destroy_surface(Some(child));
                return Err(error(E_ABORT, "preview handler was unloaded"));
            }
            child
        };
        self.apply_visuals(child);

        // Taking the sole retained Rc out of HandlerState creates an explicit
        // source-release boundary. The local Rc is dropped immediately after
        // capture, before registry registration and thread admission.
        let stream = {
            let mut state = self.state.borrow_mut();
            if !state.initialized
                || state.generation_id.as_deref() != Some(generation_id.as_str())
                || state.child != Some(child)
            {
                return Err(error(E_ABORT, "preview handler was unloaded"));
            }
            state
                .stream
                .take()
                .ok_or_else(|| error(E_UNEXPECTED, "preview stream is missing"))?
        };
        let captured_result = {
            let mut reader = IStreamCaptureReader::new(stream.as_ref());
            capture::capture(&mut reader)
        };
        drop(stream);

        let captured = match captured_result {
            Ok(captured) => {
                crate::record_capture(&captured);
                crate::record_stream_released();
                captured
            }
            Err(_) => {
                crate::record_stream_released();
                let _ = window::set_surface_text(child, "Fylura Preview Handler\r\ncapture failed");
                self.mark_preview_started(&generation_id, child);
                return Err(error(E_FAIL, "bounded shell capture failed"));
            }
        };

        window::set_surface_text(child, "Fylura Preview Handler\r\ncapturing complete")?;
        let target = self.completion_target()?;
        let memory_complete = captured.complete;
        let extension = captured.extension.clone();
        let memory: Arc<[u8]> = Arc::from(captured.bytes.into_boxed_slice());
        let handle = self
            .registry
            .register(HostProvidedRegistration {
                host: HostProvidedHost::WindowsPreviewHandler,
                generation_id: generation_id.clone(),
                source: Arc::new(MemorySource {
                    bytes: Arc::clone(&memory),
                    complete: memory_complete,
                }),
            })
            .map_err(|_| error(E_FAIL, "memory HostProvided registration failed"))?;
        let completion = Arc::new(DeferredCompletion::new());
        let cancel = Arc::new(AtomicBool::new(false));
        let admitted = {
            let mut state = self.state.borrow_mut();
            if !state.initialized
                || state.generation_id.as_deref() != Some(generation_id.as_str())
                || state.child != Some(child)
                || state.host_handle.is_some()
            {
                false
            } else {
                state.host_handle = Some(handle.clone());
                state.completion = Some(Arc::clone(&completion));
                state.deferred_cancel = Some(Arc::clone(&cancel));
                state.preview_started = true;
                true
            }
        };
        if !admitted {
            self.revoke_handle(&handle, &generation_id);
            return Err(error(E_ABORT, "preview handler was unloaded"));
        }

        let rollback_handle = handle.clone();
        let rollback_generation = generation_id.clone();
        let rollback_completion = Arc::clone(&completion);
        let rollback_cancel = Arc::clone(&cancel);
        if let Err(error) =
            self.spawn_deferred(handle, generation_id, target, extension, completion, cancel)
        {
            // Revoke the exact admitted token first, then clear only the
            // matching generation's deferred state. A later generation must
            // remain untouched if ownership changed before rollback.
            self.rollback_deferred_admission(
                &rollback_handle,
                &rollback_generation,
                &rollback_completion,
                &rollback_cancel,
            );
            return Err(error);
        }
        Ok(())
    }

    fn mark_preview_started(&self, generation_id: &str, child: HWND) {
        let mut state = self.state.borrow_mut();
        if state.initialized
            && state.generation_id.as_deref() == Some(generation_id)
            && state.child == Some(child)
        {
            state.preview_started = true;
        }
    }

    fn spawn_deferred(
        &self,
        handle: HostProvidedHandle,
        generation_id: String,
        completion_target: isize,
        extension: Option<String>,
        completion: Arc<DeferredCompletion>,
        cancel: Arc<AtomicBool>,
    ) -> Result<()> {
        let request = HostProvidedReadRequest {
            host_token: handle.host_token.clone(),
            host: HostProvidedHost::WindowsPreviewHandler,
            generation_id,
            offset_bytes: 0,
            max_bytes: 1024 * 1024,
        };
        let registry = Arc::clone(&self.registry);
        let active_deferred = ActiveDeferredGuard::new();
        let spawn = std::thread::Builder::new()
            .name("zen-preview-representation".to_string())
            .spawn(move || {
                let _active_deferred = active_deferred;
                #[cfg(feature = "test-observability")]
                crate::observations::wait_for_deferred_release();
                let result = registry
                    .read(&request)
                    .map_err(|error| error.to_string())
                    .and_then(|read| {
                        if cancel.load(Ordering::Acquire) {
                            return Err("deferred request cancelled".to_string());
                        }
                        let hint = fylura_preview_representation::RepresentationHint {
                            extension,
                            media_type: None,
                        };
                        let markdown = fylura_preview_representation::is_markdown_hint(&hint);
                        let language = if markdown {
                            // The native surface is intentionally a safe,
                            // read-only Markdown source view. Run the shared
                            // sanitizer first so hostile Markdown cannot
                            // become an active representation in this host.
                            fylura_preview_representation::render_markdown(
                                &read.bytes,
                                read.complete,
                            )
                            .map_err(|error| error.to_string())?;
                            Some("markdown")
                        } else {
                            fylura_preview_representation::source_code_language(&hint)
                        };
                        let (representation, completeness) =
                            fylura_preview_representation::render_text(
                                &read.bytes,
                                read.complete,
                                language,
                            )
                            .map_err(|error| error.to_string())?;
                        let SafeRepresentation::Text { text, language } = representation else {
                            return Err("unexpected representation family".to_string());
                        };
                        Ok(DeferredPreview {
                            text,
                            bytes_len: read.bytes.len(),
                            complete: matches!(completeness, RepresentationCompleteness::Complete),
                            language,
                        })
                    });
                completion.store(result);
                completion::post_completion(completion_target, completion.notification_id());
            });
        if spawn.is_err() {
            return Err(error(
                E_FAIL,
                "deferred representation work could not start",
            ));
        }
        crate::record_deferred_admitted();
        Ok(())
    }

    pub(crate) fn publish_deferred(&self, notification_id: u32) {
        if self.ensure_owner_thread().is_err() {
            return;
        }
        let (completion, generation_id, handle, child) = {
            let state = self.state.borrow();
            (
                state.completion.clone(),
                state.generation_id.clone(),
                state.host_handle.clone(),
                state.child,
            )
        };
        let (Some(completion), Some(generation_id), Some(handle), Some(child)) =
            (completion, generation_id, handle, child)
        else {
            return;
        };
        if completion.notification_id() != notification_id {
            return;
        }
        let Some(result) = completion.take() else {
            return;
        };
        {
            let mut state = self.state.borrow_mut();
            if state
                .completion
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &completion))
            {
                state.completion = None;
            } else {
                return;
            }
        }
        if !self.is_current(&generation_id, &handle) {
            return;
        }
        match result {
            Ok(preview) => {
                let text = format_preview_text(&preview);
                if window::set_surface_text(child, &text).is_err() {
                    let _ = window::set_surface_text(child, &format_preview_summary(&preview));
                }
                self.revoke_handle(&handle, &generation_id);
            }
            Err(message) => {
                let _ = window::set_surface_text(
                    child,
                    &format!("Fylura Preview Handler\r\nunsupported or corrupt input\r\n{message}"),
                );
                self.revoke_handle(&handle, &generation_id);
            }
        }
    }

    fn is_current(&self, generation_id: &str, handle: &HostProvidedHandle) -> bool {
        let state = self.state.borrow();
        state.initialized
            && state.generation_id.as_deref() == Some(generation_id)
            && state
                .host_handle
                .as_ref()
                .is_some_and(|current| current.host_token == handle.host_token)
    }

    fn apply_visuals(&self, child: HWND) {
        let visuals = self.visuals.borrow();
        if let Some(background_color) = visuals.background_color {
            window::set_surface_background_color(child, background_color);
        }
        if let Some(text_color) = visuals.text_color {
            window::set_surface_text_color(child, text_color);
        }
        if let Some(font) = visuals.font {
            window::set_surface_font(child, font);
        }
    }

    fn set_background_color(&self, color: COLORREF) {
        self.visuals.borrow_mut().background_color = Some(color);
        if let Some(child) = self.state.borrow().child {
            window::set_surface_background_color(child, color);
        }
    }

    fn set_text_color(&self, color: COLORREF) {
        self.visuals.borrow_mut().text_color = Some(color);
        if let Some(child) = self.state.borrow().child {
            window::set_surface_text_color(child, color);
        }
    }

    fn set_font(&self, logfont: *const LOGFONTW) -> Result<()> {
        if logfont.is_null() {
            return Err(error(E_POINTER, "preview font is null"));
        }
        let font = window::create_surface_font(unsafe { &*logfont })?;
        let old_font = {
            let mut visuals = self.visuals.borrow_mut();
            let old_font = visuals.font.replace(font);
            if let Some(child) = self.state.borrow().child {
                window::set_surface_font(child, font);
            }
            old_font
        };
        if let Some(old_font) = old_font.filter(|font| !font.is_invalid()) {
            unsafe {
                let _ = DeleteObject(old_font.into());
            }
        }
        Ok(())
    }

    fn revoke_handle(&self, handle: &HostProvidedHandle, generation_id: &str) {
        let _ = self.registry.revoke(
            &handle.host_token,
            HostProvidedHost::WindowsPreviewHandler,
            generation_id,
        );
        let mut state = self.state.borrow_mut();
        if state
            .host_handle
            .as_ref()
            .is_some_and(|current| current.host_token == handle.host_token)
        {
            state.host_handle = None;
            state.deferred_cancel = None;
        }
    }

    fn rollback_deferred_admission(
        &self,
        handle: &HostProvidedHandle,
        generation_id: &str,
        completion: &Arc<DeferredCompletion>,
        cancel: &Arc<AtomicBool>,
    ) {
        let matches_admission = {
            let state = self.state.borrow();
            state.generation_id.as_deref() == Some(generation_id)
                && state
                    .host_handle
                    .as_ref()
                    .is_some_and(|current| current.host_token == handle.host_token)
                && state
                    .completion
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, completion))
                && state
                    .deferred_cancel
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, cancel))
        };
        if !matches_admission {
            return;
        }

        self.revoke_handle(handle, generation_id);

        let detached = {
            let mut state = self.state.borrow_mut();
            if state.generation_id.as_deref() != Some(generation_id)
                || !state
                    .completion
                    .as_ref()
                    .is_some_and(|current| Arc::ptr_eq(current, completion))
            {
                None
            } else {
                state.host_handle = None;
                state.preview_started = false;
                Some((state.completion.take(), state.deferred_cancel.take()))
            }
        };
        if let Some((completion, cancel)) = detached {
            drop(completion);
            drop(cancel);
        }
    }

    fn unload_internal(&self) {
        let (handle, generation_id, child, site, frame, stream, completion, cancel) = {
            let mut state = self.state.borrow_mut();
            if !state.initialized
                && state.host_handle.is_none()
                && state.generation_id.is_none()
                && state.stream.is_none()
                && state.child.is_none()
                && state.site.is_none()
                && state.preview_frame.is_none()
                && state.completion.is_none()
                && state.deferred_cancel.is_none()
            {
                return;
            }
            state.initialized = false;
            state.preview_started = false;
            state.parent = HWND(std::ptr::null_mut());
            state.rect = RECT::default();
            (
                state.host_handle.take(),
                state.generation_id.take(),
                state.child.take(),
                state.site.take(),
                state.preview_frame.take(),
                state.stream.take(),
                state.completion.take(),
                state.deferred_cancel.take(),
            )
        };

        if let Some(cancel) = cancel.as_ref() {
            cancel.store(true, Ordering::Release);
        }
        if let (Some(handle), Some(generation_id)) = (handle, generation_id.as_deref()) {
            let _ = self.registry.revoke(
                &handle.host_token,
                HostProvidedHost::WindowsPreviewHandler,
                generation_id,
            );
        }
        window::destroy_surface(child);
        // All objects that may release/reenter COM are detached before these
        // drops. A post-capture generation has no stream; a pre-capture
        // Unload drops the sole retained stream here without cancellation.
        drop(completion);
        drop(site);
        drop(frame);
        drop(stream);
    }
}

impl Drop for PreviewHandler {
    fn drop(&mut self) {
        self.unload_internal();
        self.completion_window.get_mut().take();
        ACTIVE_OBJECTS.fetch_sub(1, Ordering::AcqRel);
    }
}

impl IInitializeWithStream_Impl for PreviewHandler_Impl {
    fn Initialize(&self, pstream: Ref<'_, IStream>, _grfmode: u32) -> Result<()> {
        self.ensure_owner_thread()?;
        if self.state.borrow().initialized {
            return Err(error(
                E_UNEXPECTED,
                "preview handler is already initialized",
            ));
        }
        let stream = pstream
            .cloned()
            .ok_or_else(|| error(E_POINTER, "preview stream is null"))?;
        let mut state = self.state.borrow_mut();
        if state.initialized {
            return Err(error(
                E_UNEXPECTED,
                "preview handler is already initialized",
            ));
        }
        state.initialized = true;
        state.generation_id = Some(Uuid::new_v4().to_string());
        state.stream = Some(Rc::new(stream));
        Ok(())
    }
}

impl IPreviewHandler_Impl for PreviewHandler_Impl {
    fn SetWindow(&self, hwnd: HWND, prc: *const RECT) -> Result<()> {
        self.ensure_owner_thread()?;
        if prc.is_null() {
            return Err(error(E_POINTER, "preview rectangle is null"));
        }
        self.set_window(hwnd, unsafe { *prc })
    }

    fn SetRect(&self, prc: *const RECT) -> Result<()> {
        self.ensure_owner_thread()?;
        if prc.is_null() {
            return Err(error(E_POINTER, "preview rectangle is null"));
        }
        self.set_rect(unsafe { *prc })
    }

    fn DoPreview(&self) -> Result<()> {
        self.do_preview()
    }

    fn Unload(&self) -> Result<()> {
        self.ensure_owner_thread()?;
        self.unload_internal();
        Ok(())
    }

    fn SetFocus(&self) -> Result<()> {
        self.ensure_owner_thread()?;
        let child = self
            .state
            .borrow()
            .child
            .ok_or_else(|| error(E_UNEXPECTED, "preview window is not attached"))?;
        window::focus_surface(child);
        Ok(())
    }

    fn QueryFocus(&self) -> Result<HWND> {
        self.ensure_owner_thread()?;
        Ok(window::focused_window())
    }

    fn TranslateAccelerator(&self, pmsg: *const MSG) -> Result<()> {
        self.ensure_owner_thread()?;
        if pmsg.is_null() {
            return Err(error(E_POINTER, "preview message is null"));
        }
        let frame = self.state.borrow().preview_frame.clone();
        match frame {
            Some(frame) => {
                let status = unsafe {
                    let vtable = <IPreviewHandlerFrame as Interface>::vtable(&frame);
                    (vtable.TranslateAccelerator)(
                        <IPreviewHandlerFrame as Interface>::as_raw(&frame),
                        pmsg,
                    )
                };
                if status == S_OK {
                    Ok(())
                } else {
                    Err(Error::from_hresult(status))
                }
            }
            // Preserve S_FALSE exactly for callers that inspect the raw ABI.
            None => Err(Error::new(S_FALSE, "preview frame did not handle message")),
        }
    }
}

impl IOleWindow_Impl for PreviewHandler_Impl {
    fn GetWindow(&self) -> Result<HWND> {
        self.ensure_owner_thread()?;
        self.state
            .borrow()
            .child
            .ok_or_else(|| error(E_FAIL, "preview child window is unavailable"))
    }

    fn ContextSensitiveHelp(&self, _fentermode: BOOL) -> Result<()> {
        Err(Error::from_hresult(E_NOTIMPL))
    }
}

impl IObjectWithSite_Impl for PreviewHandler_Impl {
    fn SetSite(&self, punksite: Ref<'_, IUnknown>) -> Result<()> {
        self.ensure_owner_thread()?;
        let site = punksite.cloned().map(Rc::new);
        let frame = site
            .as_ref()
            .and_then(|site| site.cast::<IPreviewHandlerFrame>().ok())
            .map(Rc::new);
        let (previous_site, previous_frame) = {
            let mut state = self.state.borrow_mut();
            (
                std::mem::replace(&mut state.site, site),
                std::mem::replace(&mut state.preview_frame, frame),
            )
        };
        drop(previous_site);
        drop(previous_frame);
        Ok(())
    }

    fn GetSite(&self, riid: *const GUID, ppvsite: *mut *mut c_void) -> Result<()> {
        self.ensure_owner_thread()?;
        if riid.is_null() || ppvsite.is_null() {
            return Err(error(E_POINTER, "null site output pointer"));
        }
        unsafe {
            *ppvsite = null_mut();
        }
        let site = self
            .state
            .borrow()
            .site
            .clone()
            .ok_or_else(|| error(E_FAIL, "preview site is not set"))?;
        let status = unsafe { site.query(riid, ppvsite) };
        if status.is_ok() {
            Ok(())
        } else {
            Err(Error::from_hresult(status))
        }
    }
}

impl IPreviewHandlerVisuals_Impl for PreviewHandler_Impl {
    fn SetBackgroundColor(&self, color: COLORREF) -> Result<()> {
        self.ensure_owner_thread()?;
        self.set_background_color(color);
        Ok(())
    }

    fn SetFont(&self, plf: *const LOGFONTW) -> Result<()> {
        self.ensure_owner_thread()?;
        self.set_font(plf)
    }

    fn SetTextColor(&self, color: COLORREF) -> Result<()> {
        self.ensure_owner_thread()?;
        self.set_text_color(color);
        Ok(())
    }
}

fn format_preview_text(preview: &DeferredPreview) -> String {
    let completeness = if preview.complete {
        "Complete"
    } else {
        "Partial"
    };
    let language = preview.language.as_deref().unwrap_or("text");
    let (text, truncated) = bounded_surface_text(&preview.text);
    let truncation_note = if truncated {
        "\r\n[display truncated to bounded surface size]"
    } else {
        ""
    };
    format!(
        "Fylura Preview Handler\r\nbounded bytes: {}\r\ningress: {completeness}\r\nlanguage: {language}\r\n\r\n{text}{truncation_note}",
        preview.bytes_len
    )
}

fn bounded_surface_text(text: &str) -> (&str, bool) {
    if text.len() <= MAX_SURFACE_TEXT_BYTES {
        return (text, false);
    }
    let mut end = MAX_SURFACE_TEXT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

fn format_preview_summary(preview: &DeferredPreview) -> String {
    let completeness = if preview.complete {
        "Complete"
    } else {
        "Partial"
    };
    let language = preview.language.as_deref().unwrap_or("text");
    format!(
        "Fylura Preview Handler\r\nbounded bytes: {}\r\ningress: {completeness}\r\nlanguage: {language}\r\n\r\n[preview body unavailable within the native surface bound]",
        preview.bytes_len
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, OnceLock};
    use windows::{
        core::Interface,
        Win32::{
            System::Com::IClassFactory,
            UI::{Shell::IPreviewHandler, Shell::PropertiesSystem::IInitializeWithStream},
        },
    };

    fn com_test_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(())).lock().unwrap()
    }

    #[test]
    fn class_factory_constructs_interfaces_and_unload_is_idempotent() {
        let _guard = com_test_lock();
        let before = ACTIVE_OBJECTS.load(Ordering::Acquire);
        let factory: IClassFactory = ClassFactory::new().into();
        assert!(ACTIVE_OBJECTS.load(Ordering::Acquire) > before);
        let handler: IPreviewHandler = unsafe { factory.CreateInstance(None).unwrap() };
        let _initializer: IInitializeWithStream = handler.cast().unwrap();
        unsafe {
            handler.Unload().unwrap();
            handler.Unload().unwrap();
        }
        drop(_initializer);
        drop(handler);
        drop(factory);
        assert_eq!(ACTIVE_OBJECTS.load(Ordering::Acquire), before);
    }

    #[test]
    fn active_deferred_guard_restores_unload_accounting_on_drop() {
        let _guard = com_test_lock();
        let before = ACTIVE_DEFERRED.load(Ordering::Acquire);
        {
            let _active = ActiveDeferredGuard::new();
            assert_eq!(ACTIVE_DEFERRED.load(Ordering::Acquire), before + 1);
        }
        assert_eq!(ACTIVE_DEFERRED.load(Ordering::Acquire), before);
    }

    #[test]
    fn unsupported_class_and_invalid_call_order_fail_closed() {
        let _guard = com_test_lock();
        let mut output = null_mut();
        let wrong_clsid = GUID::from_u128(0x11111111_2222_3333_4444_555555555555);
        let status = super::dll_get_class_object(
            &wrong_clsid,
            &<IClassFactory as Interface>::IID,
            &mut output,
        );
        assert_eq!(status, CLASS_E_CLASSNOTAVAILABLE);
        assert!(output.is_null());

        let factory: IClassFactory = ClassFactory::new().into();
        let handler: IPreviewHandler = unsafe { factory.CreateInstance(None).unwrap() };
        assert!(unsafe { handler.DoPreview() }.is_err());
        let initializer: IInitializeWithStream = handler.cast().unwrap();
        let stream =
            unsafe { windows::Win32::UI::Shell::SHCreateMemStream(Some(b"duplicate initialize")) }
                .expect("Windows memory IStream");
        unsafe {
            initializer.Initialize(&stream, 0).unwrap();
            assert!(initializer.Initialize(&stream, 0).is_err());
            handler.Unload().unwrap();
            initializer.Initialize(&stream, 0).unwrap();
            handler.Unload().unwrap();
        }
    }
}
