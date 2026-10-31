#![cfg(windows)]

mod instrumented_stream;
mod negative_regression;

use std::{
    env,
    error::Error,
    ffi::{c_void, OsStr},
    fs::{self, OpenOptions},
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
    ptr::null_mut,
    time::{Duration, Instant},
};

use fylura_windows_preview_registration::PRODUCTION_CLSID_U128;
use windows::Win32::System::Ole::IObjectWithSite;
use windows::{
    core::{
        implement, w, Error as WindowsError, IUnknown, Interface, GUID, HRESULT, PCSTR, PCWSTR,
    },
    Win32::{
        Foundation::{FreeLibrary, COLORREF, HMODULE, HWND, RECT},
        Graphics::Gdi::LOGFONTW,
        System::{
            Com::{
                CoInitializeEx, CoUninitialize, IClassFactory, IStream, COINIT_APARTMENTTHREADED,
            },
            LibraryLoader::{GetProcAddress, LoadLibraryW},
        },
        UI::{
            Shell::PropertiesSystem::IInitializeWithStream,
            Shell::{
                IPreviewHandler, IPreviewHandlerFrame, IPreviewHandlerVisuals,
                SHCreateStreamOnFileEx, PREVIEWHANDLERFRAMEINFO,
            },
            WindowsAndMessaging::{
                CreateWindowExW, DestroyWindow, DispatchMessageW, GetClassNameW, GetWindowLongW,
                GetWindowTextW, IsChild, MsgWaitForMultipleObjectsEx, PeekMessageW,
                TranslateMessage, ES_AUTOHSCROLL, ES_AUTOVSCROLL, ES_MULTILINE, ES_NOHIDESEL,
                ES_READONLY, ES_WANTRETURN, GWL_STYLE, MSG, MWMO_INPUTAVAILABLE, PM_REMOVE,
                QS_ALLINPUT, WS_POPUP, WS_VISIBLE,
            },
        },
    },
};

const PREVIEW_HANDLER_CLSID: GUID = GUID::from_u128(PRODUCTION_CLSID_U128);
const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);
const E_FAIL: HRESULT = HRESULT(0x80004005_u32 as _);
const E_ABORT: HRESULT = HRESULT(0x80004004_u32 as _);
const E_NOTIMPL: HRESULT = HRESULT(0x80004001_u32 as _);
const CAPTURE_LIMIT: u64 = 512 * 1024;

type DllGetClassObject = unsafe extern "system" fn(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT;
type DllCanUnloadNow = unsafe extern "system" fn() -> HRESULT;
type RecordCount = unsafe extern "system" fn() -> u32;
type DeferredCount = unsafe extern "system" fn() -> u32;
type CaptureBytes = unsafe extern "system" fn() -> u64;
type CaptureComplete = unsafe extern "system" fn() -> windows::core::BOOL;
type CaptureCalls = unsafe extern "system" fn() -> u32;
type CapturePhase = unsafe extern "system" fn() -> u32;
type CompletionWindowCount = unsafe extern "system" fn() -> u32;
type CompletionWindowCreates = unsafe extern "system" fn() -> u32;
type CompletionWindowDestroys = unsafe extern "system" fn() -> u32;
type ResetObservations = unsafe extern "system" fn();
type HoldDeferred = unsafe extern "system" fn();
type WaitDeferredHeld = unsafe extern "system" fn(u32) -> windows::core::BOOL;
type ReleaseDeferred = unsafe extern "system" fn();

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, Box<dyn Error>> {
        let status = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
        if !status.is_ok() {
            return Err(format!("CoInitializeEx failed: {status:?}").into());
        }
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct LoadedHandler {
    module: HMODULE,
    get_class_object: DllGetClassObject,
    can_unload_now: DllCanUnloadNow,
    record_count: RecordCount,
    deferred_count: DeferredCount,
    capture_bytes: CaptureBytes,
    capture_complete: CaptureComplete,
    capture_calls: CaptureCalls,
    capture_phase: CapturePhase,
    completion_window_count: CompletionWindowCount,
    completion_window_creates: CompletionWindowCreates,
    completion_window_destroys: CompletionWindowDestroys,
    reset_observations: ResetObservations,
    hold_deferred: HoldDeferred,
    wait_deferred_held: WaitDeferredHeld,
    release_deferred: ReleaseDeferred,
}

impl LoadedHandler {
    fn load(path: &Path) -> Result<Self, Box<dyn Error>> {
        let wide = wide_path(path);
        let module = unsafe { LoadLibraryW(PCWSTR(wide.as_ptr()))? };
        let loaded = unsafe {
            Self {
                module,
                get_class_object: load_symbol(module, "DllGetClassObject")?,
                can_unload_now: load_symbol(module, "DllCanUnloadNow")?,
                record_count: load_symbol(module, "W4_03_TestHostProvidedRecordCount")?,
                deferred_count: load_symbol(module, "W4_03_TestActiveDeferredCount")?,
                capture_bytes: load_symbol(module, "W4_03_TestLastCaptureBytes")?,
                capture_complete: load_symbol(module, "W4_03_TestLastCaptureComplete")?,
                capture_calls: load_symbol(module, "W4_03_TestLastCaptureReadCalls")?,
                capture_phase: load_symbol(module, "W4_03_TestCapturePhase")?,
                completion_window_count: load_symbol(module, "W4_03_TestCompletionWindowCount")?,
                completion_window_creates: load_symbol(
                    module,
                    "W4_03_TestCompletionWindowCreateCount",
                )?,
                completion_window_destroys: load_symbol(
                    module,
                    "W4_03_TestCompletionWindowDestroyCount",
                )?,
                reset_observations: load_symbol(module, "W4_03_TestResetObservations")?,
                hold_deferred: load_symbol(module, "W4_03_TestHoldDeferred")?,
                wait_deferred_held: load_symbol(module, "W4_03_TestWaitDeferredHeld")?,
                release_deferred: load_symbol(module, "W4_03_TestReleaseDeferred")?,
            }
        };
        Ok(loaded)
    }

    fn reset(&self) {
        unsafe { (self.reset_observations)() }
    }

    fn capture_summary(&self) -> (u64, bool, u32, u32) {
        unsafe {
            (
                (self.capture_bytes)(),
                (self.capture_complete)().as_bool(),
                (self.capture_calls)(),
                (self.capture_phase)(),
            )
        }
    }

    fn completion_window_counts(&self) -> (u32, u32, u32) {
        unsafe {
            (
                (self.completion_window_count)(),
                (self.completion_window_creates)(),
                (self.completion_window_destroys)(),
            )
        }
    }
}

impl Drop for LoadedHandler {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeLibrary(self.module);
        }
    }
}

struct HostWindow(HWND);

impl HostWindow {
    fn create(x: i32) -> Result<Self, Box<dyn Error>> {
        let hwnd = unsafe {
            CreateWindowExW(
                Default::default(),
                w!("STATIC"),
                w!("Fylura W4-03 v2 harness host"),
                WS_POPUP | WS_VISIBLE,
                x,
                0,
                720,
                520,
                None,
                None,
                None,
                None,
            )?
        };
        Ok(Self(hwnd))
    }

    fn hwnd(&self) -> HWND {
        self.0
    }
}

impl Drop for HostWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

#[allow(non_snake_case)]
#[implement(IPreviewHandlerFrame)]
struct FakePreviewHandlerFrame {
    translate_result: HRESULT,
}

#[allow(non_snake_case)]
impl windows::Win32::UI::Shell::IPreviewHandlerFrame_Impl for FakePreviewHandlerFrame_Impl {
    fn GetWindowContext(&self) -> windows::core::Result<PREVIEWHANDLERFRAMEINFO> {
        Err(WindowsError::from_hresult(E_NOTIMPL))
    }

    fn TranslateAccelerator(&self, _pmsg: *const MSG) -> windows::core::Result<()> {
        if self.translate_result == S_OK {
            Ok(())
        } else {
            Err(WindowsError::from_hresult(self.translate_result))
        }
    }
}

struct DeferredReleaseGuard {
    release: ReleaseDeferred,
}

impl DeferredReleaseGuard {
    fn new(release: ReleaseDeferred) -> Self {
        Self { release }
    }
}

impl Drop for DeferredReleaseGuard {
    fn drop(&mut self) {
        unsafe { (self.release)() };
    }
}

struct FixtureRoot {
    path: PathBuf,
}

impl FixtureRoot {
    fn new(path: Option<PathBuf>) -> Result<Self, Box<dyn Error>> {
        let path = path.unwrap_or_else(|| {
            env::current_dir()
                .expect("harness current directory")
                .join(".tmp-tests")
                .join("w4-03-v2-harness")
        });
        if path.as_os_str().is_empty() || path.parent().is_none() {
            return Err("fixture root must be a dedicated child path".into());
        }
        fs::create_dir_all(&path)?;
        Ok(Self { path })
    }

    fn source(&self, name: &str, bytes: &[u8]) -> Result<PathBuf, Box<dyn Error>> {
        let path = self.path.join(name);
        fs::write(&path, bytes)?;
        Ok(path)
    }
}

impl Drop for FixtureRoot {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = env::args_os();
    let _program = args.next();
    let dll_path = args.next().map(PathBuf::from).ok_or(
        "usage: fylura-windows-preview-handler-harness <preview-handler.dll> [fixture-root]",
    )?;
    let fixture_root = args.next().map(PathBuf::from);
    run(&dll_path, fixture_root)
}

fn run(dll_path: &Path, fixture_path: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    let _com = ComApartment::initialize()?;
    let fixtures = FixtureRoot::new(fixture_path)?;
    let host_a = HostWindow::create(0)?;
    let host_b = HostWindow::create(760)?;
    let handler_dll = LoadedHandler::load(dll_path)?;

    check_hr(
        unsafe { (handler_dll.can_unload_now)() },
        S_OK,
        "baseline DllCanUnloadNow",
    )?;
    if handler_dll.completion_window_counts() != (0, 0, 0) {
        return Err("completion HWND/class was not at the baseline before use".into());
    }
    let mut factory_output = null_mut();
    check_hr(
        unsafe {
            (handler_dll.get_class_object)(
                &PREVIEW_HANDLER_CLSID,
                &<IClassFactory as Interface>::IID,
                &mut factory_output,
            )
        },
        S_OK,
        "DllGetClassObject",
    )?;
    let factory: IClassFactory = unsafe { IClassFactory::from_raw(factory_output) };
    let handler: IPreviewHandler = unsafe { factory.CreateInstance(None)? };
    let initializer: IInitializeWithStream = handler.cast()?;
    let ole_window: windows::Win32::System::Ole::IOleWindow = handler.cast()?;
    let object_with_site: IObjectWithSite = handler.cast()?;
    let visuals: IPreviewHandlerVisuals = handler.cast()?;
    run_visuals_contract(&visuals)?;
    let rect = RECT {
        left: 4,
        top: 8,
        right: 700,
        bottom: 500,
    };
    let rect_after = RECT {
        left: 12,
        top: 16,
        right: 620,
        bottom: 420,
    };

    run_zero_read_initialize(&handler_dll, &initializer, &fixtures)?;
    run_frame_hresult_tests(&object_with_site, &handler)?;

    for generation in 0..3 {
        let path = fixtures.source(
            &format!("generation-{generation}.zcv2preview"),
            format!("Fylura W4-03 v2 generation {generation}\r\n你好\r\n").as_bytes(),
        )?;
        run_memory_release_case(
            &handler_dll,
            &handler,
            &initializer,
            &ole_window,
            host_a.hwnd(),
            host_b.hwnd(),
            rect,
            rect_after,
            &path,
            generation,
        )?;
    }

    run_partial_case(
        &handler_dll,
        &handler,
        &initializer,
        &ole_window,
        host_a.hwnd(),
        rect,
        &fixtures,
    )?;
    run_binary_case(
        &handler_dll,
        &handler,
        &initializer,
        host_a.hwnd(),
        rect,
        &fixtures,
    )?;
    run_stale_generation_case(
        &handler_dll,
        &handler,
        &initializer,
        &ole_window,
        host_a.hwnd(),
        rect,
        &fixtures,
    )?;
    run_completion_recreation_case(&handler_dll, &factory, host_a.hwnd(), rect, &fixtures)?;
    let negative_path = fixtures.source(
        "non-cooperative-source.zcv2preview",
        b"negative COM source fixture\r\n",
    )?;
    negative_regression::run(&negative_path)?;

    unsafe { handler.Unload()? };
    drop(visuals);
    drop(ole_window);
    drop(object_with_site);
    drop(initializer);
    drop(handler);
    drop(factory);
    let final_completion_counts = handler_dll.completion_window_counts();
    if final_completion_counts != (0, 2, 2) {
        return Err(format!(
            "completion HWND/class lifecycle did not return to baseline: live={}, creates={}, destroys={}",
            final_completion_counts.0,
            final_completion_counts.1,
            final_completion_counts.2,
        )
        .into());
    }
    if unsafe { (handler_dll.record_count)() } != 0
        || unsafe { (handler_dll.deferred_count)() } != 0
    {
        return Err("HostProvided or deferred records remained after the handler lifecycle".into());
    }
    check_hr(
        unsafe { (handler_dll.can_unload_now)() },
        S_OK,
        "post-lifecycle DllCanUnloadNow",
    )?;
    println!("HARNESS HostProvided record/deferred accounting baseline: PASS");
    println!("HARNESS DllCanUnloadNow baseline: PASS");
    println!("HARNESS child HWND/completion class/resource lifetime baseline: PASS");
    println!("HARNESS bounded-capture/source-release/COM/window lifecycle: PASS");
    println!("HARNESS registry writes: NONE (controlled harness)");
    println!("HARNESS real Explorer/prevhost evidence: NOT RUN");
    Ok(())
}

fn run_visuals_contract(visuals: &IPreviewHandlerVisuals) -> Result<(), Box<dyn Error>> {
    let mut logfont = LOGFONTW {
        lfHeight: -12,
        ..Default::default()
    };
    let face = "Consolas".encode_utf16().collect::<Vec<_>>();
    for (destination, source) in logfont.lfFaceName.iter_mut().zip(face) {
        *destination = source;
    }
    unsafe {
        visuals.SetBackgroundColor(COLORREF(0x00ffffff))?;
        visuals.SetTextColor(COLORREF(0x00000000))?;
        visuals.SetFont(&logfont)?;
    }
    println!("HARNESS IPreviewHandlerVisuals background/text/fixed-font contract: PASS");
    Ok(())
}

fn run_zero_read_initialize(
    dll: &LoadedHandler,
    initializer: &IInitializeWithStream,
    fixtures: &FixtureRoot,
) -> Result<(), Box<dyn Error>> {
    dll.reset();
    let _path = fixtures.source("initialize-only.zcv2preview", b"initialize must not read")?;
    let (stream, observation) = instrumented_stream::create(b"initialize must not read");
    unsafe { initializer.Initialize(&stream, 0)? };
    drop(stream);
    let observation = observation.lock().expect("stream observation lock");
    let (bytes, _complete, calls, phase) = dll.capture_summary();
    let records = unsafe { (dll.record_count)() };
    let deferred = unsafe { (dll.deferred_count)() };
    if observation.read_calls != 0
        || observation.accepted_bytes != 0
        || bytes != 0
        || calls != 0
        || phase != 0
        || records != 0
        || deferred != 0
    {
        return Err(format!(
            "Initialize performed work: stream_read_calls={}, stream_bytes={}, capture_bytes={bytes}, read_calls={calls}, phase={phase}, records={records}, deferred={deferred}",
            observation.read_calls, observation.accepted_bytes
        )
        .into());
    }
    println!("HARNESS Initialize zero content reads/provider work: PASS");
    drop(observation);
    unsafe { initializer.cast::<IPreviewHandler>()?.Unload()? };
    Ok(())
}

fn run_frame_hresult_tests(
    object_with_site: &IObjectWithSite,
    handler: &IPreviewHandler,
) -> Result<(), Box<dyn Error>> {
    let message = MSG::default();
    for (label, expected) in [
        ("frame S_OK", S_OK),
        ("frame S_FALSE", S_FALSE),
        ("frame failure", E_ABORT),
    ] {
        let frame: IUnknown = FakePreviewHandlerFrame {
            translate_result: expected,
        }
        .into();
        unsafe { object_with_site.SetSite(&frame)? };
        check_hr(
            unsafe { raw_translate_accelerator(handler, &message) },
            expected,
            label,
        )?;
    }
    unsafe { object_with_site.SetSite(None::<&IUnknown>)? };
    check_hr(
        unsafe { raw_translate_accelerator(handler, &message) },
        S_FALSE,
        "no frame",
    )?;
    println!("HARNESS raw frame TranslateAccelerator HRESULT matrix: PASS");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn run_memory_release_case(
    dll: &LoadedHandler,
    handler: &IPreviewHandler,
    initializer: &IInitializeWithStream,
    ole_window: &windows::Win32::System::Ole::IOleWindow,
    host_a: HWND,
    host_b: HWND,
    rect: RECT,
    rect_after: RECT,
    path: &Path,
    generation: usize,
) -> Result<(), Box<dyn Error>> {
    dll.reset();
    let stream = open_stream(path)?;
    unsafe { initializer.Initialize(&stream, 0)? };
    drop(stream);
    unsafe { handler.SetWindow(host_a, &rect)? };
    let (before_status, before_child) = unsafe { raw_get_window(ole_window) };
    check_hr(before_status, E_FAIL, "GetWindow before DoPreview")?;
    if !before_child.is_invalid() {
        return Err("child existed before DoPreview".into());
    }
    unsafe { (dll.hold_deferred)() };
    let release_gate = DeferredReleaseGuard::new(dll.release_deferred);
    let do_preview_started = Instant::now();
    unsafe { handler.DoPreview()? };
    let do_preview_elapsed = do_preview_started.elapsed();
    if !unsafe { (dll.wait_deferred_held)(5_000).as_bool() } {
        return Err(
            "deferred worker did not reach the deterministic capture-release barrier".into(),
        );
    }
    let (completion_windows, completion_creates, completion_destroys) =
        dll.completion_window_counts();
    if completion_windows != 1 || completion_creates != 1 || completion_destroys != 0 {
        return Err(format!(
            "completion window lifecycle was not stable after admission: live={completion_windows}, creates={completion_creates}, destroys={completion_destroys}"
        )
        .into());
    }

    // The handler's only stream reference is released before DoPreview returns;
    // these mutations happen while deferred representation work is held at a
    // deterministic barrier. No Unload is used to make the source mutable.
    OpenOptions::new().write(true).open(path)?.sync_all()?;
    let renamed = path.with_extension(format!("renamed-{generation}"));
    fs::rename(path, &renamed)?;
    let moved = path
        .parent()
        .expect("fixture parent")
        .join(format!("moved-{generation}.zcv2preview"));
    fs::rename(&renamed, &moved)?;
    fs::remove_file(&moved)?;

    let (_, child) = unsafe { raw_get_window(ole_window) };
    if child.is_invalid() || child == host_a || !unsafe { IsChild(host_a, child).as_bool() } {
        return Err("DoPreview did not create one host-owned child".into());
    }
    assert_native_surface_contract(child)?;
    unsafe { handler.SetRect(&rect_after)? };
    if unsafe { raw_get_window(ole_window) }.1 != child {
        return Err("SetRect replaced the child surface".into());
    }
    unsafe { handler.SetWindow(host_b, &rect_after)? };
    if unsafe { raw_get_window(ole_window) }.1 != child
        || !unsafe { IsChild(host_b, child).as_bool() }
    {
        return Err("SetWindow did not reuse/reparent the one child surface".into());
    }
    unsafe { handler.SetFocus()? };
    if unsafe { handler.QueryFocus()? } != child {
        return Err("QueryFocus did not report the focused child".into());
    }
    check_hr(
        unsafe { raw_translate_accelerator(handler, &MSG::default()) },
        S_FALSE,
        "TranslateAccelerator without frame",
    )?;

    drop(release_gate);
    let publication_started = Instant::now();
    let text = wait_for_child_text(dll, child)?;
    let publication_elapsed = publication_started.elapsed();
    let (bytes, complete, calls, phase) = dll.capture_summary();
    if bytes > CAPTURE_LIMIT || !complete || calls == 0 || phase != 3 {
        return Err(format!(
            "capture evidence invalid: bytes={bytes}, complete={complete}, calls={calls}, phase={phase}"
        )
        .into());
    }
    if !text.contains(&format!("generation {generation}")) || !text.contains("Complete") {
        return Err(format!("deferred representation was not published: {text:?}").into());
    }
    if unsafe { (dll.record_count)() } != 0 {
        return Err("completed memory HostProvided record was not revoked".into());
    }
    println!(
        "HARNESS generation {generation}: capture={} bytes, Complete, source mutations before render, one-child publication: PASS (DoPreview-return={}ms, publication-after-release={}ms)",
        bytes,
        do_preview_elapsed.as_millis(),
        publication_elapsed.as_millis()
    );
    unsafe { handler.Unload()? };
    let (status, after_child) = unsafe { raw_get_window(ole_window) };
    check_hr(status, E_FAIL, "GetWindow after Unload")?;
    if !after_child.is_invalid() {
        return Err("Unload retained child".into());
    }
    Ok(())
}

fn assert_native_surface_contract(child: HWND) -> Result<(), Box<dyn Error>> {
    let style = unsafe { GetWindowLongW(child, GWL_STYLE) as u32 };
    // RichEdit normalizes the WS_* scrollbar bits out of the returned window
    // style while retaining the edit-control auto-scroll contract. The
    // production creation request still includes both WS_* bits; assert the
    // stable RichEdit surface flags that are observable after normalization.
    let required_styles = ES_MULTILINE as u32
        | ES_READONLY as u32
        | ES_AUTOVSCROLL as u32
        | ES_AUTOHSCROLL as u32
        | ES_NOHIDESEL as u32
        | ES_WANTRETURN as u32;
    if style & required_styles != required_styles {
        return Err(format!(
            "native preview surface edit styles missing: style=0x{style:08x}, required=0x{required_styles:08x}"
        )
        .into());
    }
    let mut class_name = [0_u16; 32];
    let length = unsafe { GetClassNameW(child, &mut class_name) };
    if length <= 0 || String::from_utf16_lossy(&class_name[..length as usize]) != "RICHEDIT50W" {
        return Err("native preview surface is not the bounded RichEdit control".into());
    }
    println!("HARNESS read-only multiline auto-scroll native surface: PASS");
    Ok(())
}

fn run_partial_case(
    dll: &LoadedHandler,
    handler: &IPreviewHandler,
    initializer: &IInitializeWithStream,
    ole_window: &windows::Win32::System::Ole::IOleWindow,
    host: HWND,
    rect: RECT,
    fixtures: &FixtureRoot,
) -> Result<(), Box<dyn Error>> {
    dll.reset();
    let bytes = vec![b'x'; CAPTURE_LIMIT as usize + 1];
    let path = fixtures.source("larger-than-cap.zcv2preview", &bytes)?;
    let stream = open_stream(&path)?;
    unsafe { (dll.hold_deferred)() };
    let release_gate = DeferredReleaseGuard::new(dll.release_deferred);
    unsafe {
        initializer.Initialize(&stream, 0)?;
        handler.SetWindow(host, &rect)?;
        handler.DoPreview()?;
    }
    if !unsafe { (dll.wait_deferred_held)(5_000).as_bool() } {
        return Err("partial-case deferred worker did not reach the deterministic barrier".into());
    }
    drop(release_gate);
    drop(stream);
    let child = unsafe { raw_get_window(ole_window) }.1;
    let text = wait_for_child_text(dll, child)?;
    let (captured, complete, _, _) = dll.capture_summary();
    if captured != CAPTURE_LIMIT || complete || !text.contains("Partial") {
        return Err(format!(
            "larger source capture was not truthful: bytes={captured}, complete={complete}, text={text:?}"
        )
        .into());
    }
    println!("HARNESS >512 KiB source: 512 KiB cap and Partial: PASS");
    unsafe { handler.Unload()? };
    Ok(())
}

fn run_binary_case(
    dll: &LoadedHandler,
    handler: &IPreviewHandler,
    initializer: &IInitializeWithStream,
    host: HWND,
    rect: RECT,
    fixtures: &FixtureRoot,
) -> Result<(), Box<dyn Error>> {
    dll.reset();
    let path = fixtures.source("binary.zcv2preview", b"safe\0binary")?;
    let stream = open_stream(&path)?;
    unsafe {
        initializer.Initialize(&stream, 0)?;
        handler.SetWindow(host, &rect)?;
        handler.DoPreview()?;
    }
    drop(stream);
    let child = unsafe { raw_get_window(&handler.cast()?).1 };
    let text = wait_for_child_text(dll, child)?;
    if !text.contains("unsupported or corrupt input") {
        return Err(format!("binary input was not rejected locally: {text:?}").into());
    }
    println!("HARNESS corrupt/binary input fails locally without Zen UI: PASS");
    unsafe { handler.Unload()? };
    Ok(())
}

fn run_stale_generation_case(
    dll: &LoadedHandler,
    handler: &IPreviewHandler,
    initializer: &IInitializeWithStream,
    ole_window: &windows::Win32::System::Ole::IOleWindow,
    host: HWND,
    rect: RECT,
    fixtures: &FixtureRoot,
) -> Result<(), Box<dyn Error>> {
    dll.reset();
    let path = fixtures.source("stale.zcv2preview", vec![b'z'; 256 * 1024].as_slice())?;
    let stream = open_stream(&path)?;
    unsafe { (dll.hold_deferred)() };
    let release_gate = DeferredReleaseGuard::new(dll.release_deferred);
    unsafe {
        initializer.Initialize(&stream, 0)?;
        handler.SetWindow(host, &rect)?;
        handler.DoPreview()?;
        if !(dll.wait_deferred_held)(5_000).as_bool() {
            return Err(
                "stale-case deferred worker did not reach the deterministic barrier".into(),
            );
        }
        handler.Unload()?;
    }
    drop(stream);

    // Start a new generation before releasing the old worker. The old
    // completion notification must therefore be stale even though the
    // owner-STA HWND is still alive.
    let replacement_path = fixtures.source(
        "stale-replacement.zcv2preview",
        b"replacement generation must not be repainted by stale work\r\n",
    )?;
    let replacement_stream = open_stream(&replacement_path)?;
    unsafe {
        initializer.Initialize(&replacement_stream, 0)?;
        handler.SetWindow(host, &rect)?;
    }
    drop(replacement_stream);
    drop(release_gate);
    wait_for_deferred_quiescence(dll)?;
    let (status, child) = unsafe { raw_get_window(ole_window) };
    check_hr(status, E_FAIL, "stale generation GetWindow")?;
    if !child.is_invalid()
        || unsafe { (dll.record_count)() } != 0
        || dll.completion_window_counts() != (1, 1, 0)
    {
        return Err("stale completion repainted or retained revoked state".into());
    }
    unsafe { handler.Unload()? };
    println!("HARNESS Unload/new-generation stale completion rejection: PASS");
    Ok(())
}

fn run_completion_recreation_case(
    dll: &LoadedHandler,
    factory: &IClassFactory,
    host: HWND,
    rect: RECT,
    fixtures: &FixtureRoot,
) -> Result<(), Box<dyn Error>> {
    let (before_live, before_creates, before_destroys) = dll.completion_window_counts();
    if before_live != 1 || before_creates != 1 || before_destroys != 0 {
        return Err(format!(
            "completion window was not at the reusable lifecycle baseline: live={before_live}, creates={before_creates}, destroys={before_destroys}"
        )
        .into());
    }

    let handler: IPreviewHandler = unsafe { factory.CreateInstance(None)? };
    let initializer: IInitializeWithStream = handler.cast()?;
    let ole_window: windows::Win32::System::Ole::IOleWindow = handler.cast()?;
    let path = fixtures.source(
        "completion-recreated.zcv2preview",
        b"completion window recreation generation\r\n",
    )?;
    let stream = open_stream(&path)?;
    unsafe {
        initializer.Initialize(&stream, 0)?;
        handler.SetWindow(host, &rect)?;
        handler.DoPreview()?;
    }
    drop(stream);

    let child = unsafe { raw_get_window(&ole_window) }.1;
    let text = wait_for_child_text(dll, child)?;
    let (live, creates, destroys) = dll.completion_window_counts();
    if live != before_live + 1
        || creates != before_creates + 1
        || destroys != before_destroys
        || !text.contains("completion window recreation generation")
    {
        return Err(format!(
            "completion window was not recreated and published: live={live}, creates={creates}, destroys={destroys}, text={text:?}"
        )
        .into());
    }
    if unsafe { (dll.record_count)() } != 0 {
        return Err("recreated handler retained a HostProvided record".into());
    }

    unsafe { handler.Unload()? };
    let recreated_child = unsafe { raw_get_window(&ole_window) }.1;
    if !recreated_child.is_invalid() {
        return Err("recreated handler retained its child after Unload".into());
    }
    drop(ole_window);
    drop(initializer);
    drop(handler);
    if dll.completion_window_counts() != (before_live, before_creates + 1, before_destroys + 1) {
        return Err("destroying the recreated handler did not restore HWND baseline".into());
    }
    println!("HARNESS completion HWND create/post/destroy/recreate lifecycle: PASS");
    Ok(())
}

fn open_stream(path: &Path) -> Result<IStream, Box<dyn Error>> {
    let wide = wide_path(path);
    let access = (windows::Win32::System::Com::STGM_READ
        | windows::Win32::System::Com::STGM_SHARE_DENY_WRITE)
        .0;
    Ok(unsafe {
        SHCreateStreamOnFileEx(PCWSTR(wide.as_ptr()), access, 0, false, None::<&IStream>)?
    })
}

fn wait_for_child_text(dll: &LoadedHandler, child: HWND) -> Result<String, Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pump_messages();
        let text = get_window_text(child)?;
        if !text.contains("capturing complete") && !text.is_empty() {
            return Ok(text);
        }
        if unsafe { (dll.deferred_count)() } == 0 {
            pump_messages();
            let text = get_window_text(child)?;
            if !text.contains("capturing complete") && !text.is_empty() {
                return Ok(text);
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            let (_, complete, calls, phase) = dll.capture_summary();
            return Err(format!(
                "deferred representation did not publish within five seconds: text={text:?}, deferred={}, records={}, capture_complete={complete}, capture_calls={calls}, phase={phase}",
                unsafe { (dll.deferred_count)() },
                unsafe { (dll.record_count)() },
            )
            .into());
        }
        unsafe {
            let _ = MsgWaitForMultipleObjectsEx(
                None,
                remaining.as_millis().min(u128::from(u32::MAX)) as u32,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
        }
    }
}

fn wait_for_deferred_quiescence(dll: &LoadedHandler) -> Result<(), Box<dyn Error>> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pump_messages();
        if unsafe { (dll.deferred_count)() } == 0 {
            pump_messages();
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err("deferred worker did not quiesce within five seconds".into());
        }
        unsafe {
            let _ = MsgWaitForMultipleObjectsEx(
                None,
                remaining.as_millis().min(u128::from(u32::MAX)) as u32,
                QS_ALLINPUT,
                MWMO_INPUTAVAILABLE,
            );
        }
    }
}

fn pump_messages() {
    unsafe {
        let mut message = MSG::default();
        while PeekMessageW(&mut message, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&message);
            let _ = DispatchMessageW(&message);
        }
    }
}

unsafe fn raw_get_window(ole_window: &windows::Win32::System::Ole::IOleWindow) -> (HRESULT, HWND) {
    let mut hwnd = HWND(null_mut());
    let vtable = <windows::Win32::System::Ole::IOleWindow as Interface>::vtable(ole_window);
    let status = (vtable.GetWindow)(
        <windows::Win32::System::Ole::IOleWindow as Interface>::as_raw(ole_window),
        &mut hwnd,
    );
    (status, hwnd)
}

unsafe fn raw_translate_accelerator(handler: &IPreviewHandler, message: &MSG) -> HRESULT {
    let vtable = <IPreviewHandler as Interface>::vtable(handler);
    (vtable.TranslateAccelerator)(<IPreviewHandler as Interface>::as_raw(handler), message)
}

fn get_window_text(hwnd: HWND) -> Result<String, Box<dyn Error>> {
    let mut buffer = [0_u16; 1024];
    let length = unsafe { GetWindowTextW(hwnd, &mut buffer) };
    if length < 0 {
        return Err(WindowsError::from_win32().into());
    }
    Ok(String::from_utf16_lossy(&buffer[..length as usize]))
}

fn check_hr(actual: HRESULT, expected: HRESULT, label: &str) -> Result<(), Box<dyn Error>> {
    if actual == expected {
        Ok(())
    } else {
        Err(format!("{label}: expected {expected:?}, got {actual:?}").into())
    }
}

unsafe fn load_symbol<T>(module: HMODULE, name: &str) -> Result<T, Box<dyn Error>> {
    let nul_name = format!("{name}\0");
    let symbol = GetProcAddress(module, PCSTR(nul_name.as_ptr().cast()))
        .ok_or_else(WindowsError::from_win32)?;
    Ok(std::mem::transmute_copy(&symbol))
}

fn wide_path(path: &Path) -> Vec<u16> {
    OsStr::new(path)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}
