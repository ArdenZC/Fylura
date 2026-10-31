use std::{
    cell::RefCell,
    rc::Rc,
    sync::{atomic::AtomicBool, Arc},
};

use fylura_native_host::HostProvidedHandle;
use windows::{
    core::IUnknown,
    Win32::{
        Foundation::{HWND, RECT},
        System::Com::IStream,
        UI::Shell::IPreviewHandlerFrame,
    },
};

use crate::completion::DeferredCompletion;

#[derive(Default)]
pub(crate) struct HandlerState {
    pub(crate) initialized: bool,
    pub(crate) preview_started: bool,
    pub(crate) generation_id: Option<String>,
    /// This is the only shell stream reference retained by the handler. It is
    /// taken and dropped synchronously inside DoPreview before registration or
    /// worker admission.
    pub(crate) stream: Option<Rc<IStream>>,
    pub(crate) site: Option<Rc<IUnknown>>,
    pub(crate) preview_frame: Option<Rc<IPreviewHandlerFrame>>,
    pub(crate) parent: HWND,
    pub(crate) rect: RECT,
    pub(crate) child: Option<HWND>,
    pub(crate) host_handle: Option<HostProvidedHandle>,
    pub(crate) completion: Option<Arc<DeferredCompletion>>,
    pub(crate) deferred_cancel: Option<Arc<AtomicBool>>,
}

pub(crate) type SharedHandlerState = RefCell<HandlerState>;
