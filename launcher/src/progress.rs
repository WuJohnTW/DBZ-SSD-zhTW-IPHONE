//! What a download shows while it runs: the bytes so far, the phase (connecting, downloading, verifying,
//! installing), and the player's "cancel". The work runs on a worker thread ([`with_progress`]); on Windows the
//! main thread shows a task dialog (comctl32 v6 `TaskDialogIndirect`, `TDF_SHOW_PROGRESS_BAR` and
//! `TDF_CALLBACK_TIMER`, which calls back about every 200 ms), so the window keeps answering. The same task dialog
//! asks the questions with buttons of our own words ([`ask`]).
//!
//! `TaskDialogIndirect` is looked up when needed, never imported: without the manifest that selects comctl32 v6 the
//! process gets v5, which has none, and the launcher must still start. No comctl32 is ever loaded from a folder:
//! one already loaded is used only when it lies under the Windows folder (System32 or WinSxS); else it is loaded
//! by name only when this process's activation context (the exe's manifest) redirects `comctl32.dll` — the loader
//! then takes it from WinSxS before searching any folder (Microsoft Learn, "Dynamic-link library search order":
//! SxS manifest redirection comes before "the folder from which the application loaded"). Then
//! [`with_progress`] waits for the work without a window and [`ask`] answers `None` (the caller asks with rfd's
//! yes / no). Not Windows (the tests on a Mac): the same, no window.

use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;

/// Where a download is. In this order; the player can cancel until [`Phase::Verifying`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
pub enum Phase {
    Connecting = 0,
    Downloading = 1,
    Verifying = 2,
    Installing = 3,
}

impl Phase {
    fn from_u8(v: u8) -> Phase {
        match v {
            0 => Phase::Connecting,
            1 => Phase::Downloading,
            2 => Phase::Verifying,
            _ => Phase::Installing,
        }
    }
}

/// What the work reports, and asks: whether to stop.
pub trait ProgressSink: Sync {
    fn phase(&self, phase: Phase);
    /// `done` of `total` bytes.
    fn bytes(&self, done: u64, total: u64);
    /// Whether the work must stop now (and remove what it wrote).
    fn cancelled(&self) -> bool;
}

/// Reports to nobody, never cancelled.
#[cfg_attr(not(test), allow(dead_code))]
pub struct NoProgress;

impl ProgressSink for NoProgress {
    fn phase(&self, _: Phase) {}
    fn bytes(&self, _: u64, _: u64) {}
    fn cancelled(&self) -> bool {
        false
    }
}

/// The error of work the player cancelled: no message box for it.
pub const CANCELLED: &str = "已取消";

/// Shared by the work (writes) and the dialog (reads).
#[derive(Debug, Default)]
pub struct Progress {
    phase: AtomicU8,
    done: AtomicU64,
    total: AtomicU64,
    cancel: AtomicBool,
    finished: AtomicBool,
}

impl Progress {
    #[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
    pub fn request_cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    pub fn cancel_requested(&self) -> bool {
        self.cancel.load(Ordering::SeqCst)
    }

    /// The player may cancel only while nothing is verified or swapped.
    pub fn can_cancel(&self) -> bool {
        self.current_phase() < Phase::Verifying
    }

    pub fn current_phase(&self) -> Phase {
        Phase::from_u8(self.phase.load(Ordering::SeqCst))
    }

    #[cfg_attr(not(windows), allow(dead_code))] // read by the task dialog
    pub fn finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    #[cfg_attr(not(windows), allow(dead_code))] // read by the task dialog
    pub fn snapshot(&self) -> (Phase, u64, u64) {
        (self.current_phase(), self.done.load(Ordering::SeqCst), self.total.load(Ordering::SeqCst))
    }
}

impl ProgressSink for Progress {
    fn phase(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::SeqCst);
    }

    fn bytes(&self, done: u64, total: u64) {
        self.total.store(total, Ordering::SeqCst);
        self.done.store(done, Ordering::SeqCst);
    }

    fn cancelled(&self) -> bool {
        // guard: no cancel once verifying
        self.cancel_requested() && self.can_cancel()
    }
}

/// The bar's position, 0 to 1000.
#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub fn bar(done: u64, total: u64) -> u32 {
    if total == 0 {
        return 0;
    }
    (u128::from(done.min(total)) * 1000 / u128::from(total)) as u32
}

/// The dialog's line.
#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub fn status_text(phase: Phase, done: u64, total: u64, cancelling: bool) -> String {
    if cancelling {
        return "取消中，正在刪除下載到一半的檔…".to_owned();
    }
    let mb = |b: u64| b as f64 / (1024.0 * 1024.0);
    match phase {
        Phase::Connecting => "連線中…".to_owned(),
        Phase::Downloading => {
            format!("已下載 {:.1}／{:.1} MB（{}%）", mb(done), mb(total), bar(done, total) / 10)
        }
        Phase::Verifying => "驗證中…".to_owned(),
        Phase::Installing => "安裝更新中…".to_owned(),
    }
}

/// The dialog's line while the player has (`cancel_clicked`) or has not asked to cancel: "cancelling" only while
/// the cancel is still accepted; once verifying or installing, the phase itself (the cancel came too late).
#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub fn shown_text(phase: Phase, done: u64, total: u64, cancel_clicked: bool) -> String {
    // guard: cancelling shown only while accepted
    status_text(phase, done, total, cancel_clicked && phase < Phase::Verifying)
}

/// Whether the dialog's "取消" button can be clicked.
#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub fn stop_enabled(phase: Phase, cancel_clicked: bool) -> bool {
    phase < Phase::Verifying && !cancel_clicked
}

/// What the dialog does on its timer, `ticks` timer ticks (about 200 ms each) after the work finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub enum Tick {
    /// The work is running: show where it is.
    Refresh,
    /// Finished: enable the "取消" button (it is disabled while verifying and installing) and click it, which the
    /// button handler answers by closing ([`on_button`]).
    EnableAndClick,
    /// Still open a second later: close the window (WM_CLOSE; with TDF_ALLOW_DIALOG_CANCELLATION it comes back as
    /// a button click, answered the same way).
    Close,
}

#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub fn on_tick(finished: bool, ticks: u32) -> Tick {
    match (finished, ticks) {
        (false, _) => Tick::Refresh,
        (true, 0) => Tick::EnableAndClick,
        // guard: close again if still open
        (true, _) if ticks.is_multiple_of(5) => Tick::Close,
        (true, _) => Tick::Refresh,
    }
}

/// What a click (the button, Esc, Alt+F4, the X, or the program's own click) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub enum Button {
    /// The work is finished: the window closes (never taken for the player's cancel).
    Close,
    /// Ask the work to stop; the window stays until it has removed what it wrote.
    Cancel,
    /// Already cancelling, or too late to cancel: the window stays.
    Ignore,
}

#[cfg_attr(not(windows), allow(dead_code))] // used by the task dialog
pub fn on_button(finished: bool, can_cancel: bool, cancel_clicked: bool) -> Button {
    // guard: a finished work closes
    if finished {
        return Button::Close;
    }
    if can_cancel && !cancel_clicked {
        return Button::Cancel;
    }
    Button::Ignore
}

/// Runs `job` on a worker thread and returns its result; meanwhile a task dialog titled `title` shows its
/// progress with a "取消" button (Windows with comctl32 v6), else nothing is shown and this just waits. `Err` only
/// when no thread could be made (the job did not run).
pub fn with_progress<T: Send + 'static>(
    title: &str,
    instruction: &str,
    job: impl FnOnce(&Progress) -> T + Send + 'static,
) -> Result<T, String> {
    let progress = Arc::new(Progress::default());
    let shared = progress.clone();
    let worker = std::thread::Builder::new()
        .name("download".to_owned())
        .spawn(move || {
            let result = job(&shared);
            shared.finished.store(true, Ordering::SeqCst);
            result
        })
        .map_err(|e| format!("無法建立下載執行緒：{e}"))?;
    #[cfg(windows)]
    let _shown = win::show_progress(title, instruction, &progress);
    #[cfg(not(windows))]
    let _ = (title, instruction, &progress);
    match worker.join() {
        Ok(result) => Ok(result),
        // only with panic = "unwind" (the tests): the release build aborts before this
        Err(panic) => std::panic::resume_unwind(panic),
    }
}

/// Whether [`with_progress`] shows a window and [`ask`] asks (comctl32 v6 on Windows).
pub fn task_dialog_available() -> bool {
    #[cfg(windows)]
    return win::task_dialog().is_some();
    #[cfg(not(windows))]
    false
}

/// Asks `text` with one button per label (the first is the default): `Some(Some(i))` for the label clicked,
/// `Some(None)` when the dialog was closed, `None` when there is no task dialog (the caller asks otherwise).
pub fn ask(title: &str, text: &str, labels: &[&str]) -> Option<Option<usize>> {
    #[cfg(windows)]
    return win::ask(title, text, labels);
    #[cfg(not(windows))]
    {
        let _ = (title, text, labels);
        None
    }
}

#[cfg(windows)]
mod win {
    use super::{bar, on_button, on_tick, shown_text, stop_enabled, Button, Progress, Tick};
    use std::cell::{Cell, RefCell};
    use std::sync::OnceLock;
    use windows_sys::core::HRESULT;
    use windows_sys::Win32::Foundation::{BOOL, HMODULE, HWND, LPARAM, S_FALSE, S_OK, WPARAM};
    use windows_sys::Win32::System::ApplicationInstallationAndServicing::{FindActCtxSectionStringW, ACTCTX_SECTION_KEYED_DATA};
    use windows_sys::Win32::System::LibraryLoader::{GetModuleFileNameW, GetModuleHandleW, GetProcAddress, LoadLibraryW};
    use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;
    use windows_sys::Win32::UI::Controls::{
        TASKDIALOGCONFIG, TASKDIALOGCONFIG_0, TASKDIALOGCONFIG_1, TASKDIALOG_BUTTON, TDE_CONTENT,
        TDF_ALLOW_DIALOG_CANCELLATION, TDF_CALLBACK_TIMER, TDF_SHOW_PROGRESS_BAR, TDF_SIZE_TO_CONTENT, TDM_CLICK_BUTTON,
        TDM_ENABLE_BUTTON, TDM_SET_ELEMENT_TEXT, TDM_SET_PROGRESS_BAR_POS, TDM_SET_PROGRESS_BAR_RANGE, TDN_BUTTON_CLICKED,
        TDN_CREATED, TDN_TIMER, TD_INFORMATION_ICON,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, SendMessageW, WM_CLOSE};

    /// `ACTIVATION_CONTEXT_SECTION_DLL_REDIRECTION` (winnt.h; windows-sys `Win32::System::SystemServices`).
    const ACTIVATION_CONTEXT_SECTION_DLL_REDIRECTION: u32 = 2;

    type TaskDialogIndirect = unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut BOOL) -> HRESULT;

    const ID_STOP: i32 = 100;
    const ID_FIRST: i32 = 200;

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    /// `TaskDialogIndirect` of the comctl32 v6 this process uses, or `None`: no manifest (v5, which has none), or
    /// a comctl32 that is not the system's. Never loads a DLL from a folder (see the module's doc).
    pub(super) fn task_dialog() -> Option<TaskDialogIndirect> {
        static FOUND: OnceLock<Option<TaskDialogIndirect>> = OnceLock::new();
        *FOUND.get_or_init(|| {
            let name = wide("comctl32.dll");
            // SAFETY: plain Win32 calls with NUL-terminated names and buffers of the sizes given; the pointer
            // GetProcAddress returns is the documented TaskDialogIndirect, whose signature the type spells out.
            unsafe {
                let mut module = GetModuleHandleW(name.as_ptr());
                if module.is_null() {
                    // guard: load only through the manifest's redirection
                    if !redirected_by_manifest(&name) {
                        return None;
                    }
                    module = LoadLibraryW(name.as_ptr());
                }
                // guard: only the system's comctl32
                if module.is_null() || !under_windows_folder(module) {
                    return None;
                }
                let f = GetProcAddress(module, c"TaskDialogIndirect".as_ptr().cast())?;
                Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, TaskDialogIndirect>(f))
            }
        })
    }

    /// Whether this process's activation context redirects `name` (`FindActCtxSectionStringW`, the DLL redirection
    /// section: filled from the assemblies the exe's manifest depends on, here Common Controls 6).
    unsafe fn redirected_by_manifest(name: &[u16]) -> bool {
        let mut data: ACTCTX_SECTION_KEYED_DATA = std::mem::zeroed();
        data.cbSize = std::mem::size_of::<ACTCTX_SECTION_KEYED_DATA>() as u32;
        FindActCtxSectionStringW(0, std::ptr::null(), ACTIVATION_CONTEXT_SECTION_DLL_REDIRECTION, name.as_ptr(), &mut data) != 0
    }

    /// Whether `module`'s file lies under the Windows folder (System32, WinSxS), not beside the exe.
    unsafe fn under_windows_folder(module: HMODULE) -> bool {
        let mut path = [0u16; 1024];
        let mut windows = [0u16; 512];
        let n = GetModuleFileNameW(module, path.as_mut_ptr(), path.len() as u32) as usize;
        let m = GetSystemWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) as usize;
        let (Some(path), Some(windows)) = (path.get(..n).filter(|_| n > 0 && n < 1024), windows.get(..m).filter(|_| m > 0 && m < 512)) else {
            return false;
        };
        let path = String::from_utf16_lossy(path).to_lowercase();
        let windows = String::from_utf16_lossy(windows).to_lowercase();
        path.starts_with(&format!("{}\\", windows.trim_end_matches('\\')))
    }

    fn config(title: &[u16], instruction: *const u16, content: &[u16], buttons: &[TASKDIALOG_BUTTON]) -> TASKDIALOGCONFIG {
        TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: std::ptr::null_mut(),
            hInstance: std::ptr::null_mut(),
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
            dwCommonButtons: 0,
            pszWindowTitle: title.as_ptr(),
            Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: TD_INFORMATION_ICON },
            pszMainInstruction: instruction,
            pszContent: content.as_ptr(),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: buttons.first().map_or(0, |b| b.nButtonID),
            cRadioButtons: 0,
            pRadioButtons: std::ptr::null(),
            nDefaultRadioButton: 0,
            pszVerificationText: std::ptr::null(),
            pszExpandedInformation: std::ptr::null(),
            pszExpandedControlText: std::ptr::null(),
            pszCollapsedControlText: std::ptr::null(),
            Anonymous2: TASKDIALOGCONFIG_1 { pszFooterIcon: std::ptr::null() },
            pszFooter: std::ptr::null(),
            pfCallback: None,
            lpCallbackData: 0,
            cxWidth: 0,
        }
    }

    pub fn ask(title: &str, text: &str, labels: &[&str]) -> Option<Option<usize>> {
        let show = task_dialog()?;
        let (title, text) = (wide(title), wide(text));
        let label_text: Vec<Vec<u16>> = labels.iter().map(|l| wide(l)).collect();
        let buttons: Vec<TASKDIALOG_BUTTON> = label_text
            .iter()
            .enumerate()
            .map(|(i, l)| TASKDIALOG_BUTTON { nButtonID: ID_FIRST + i as i32, pszButtonText: l.as_ptr() })
            .collect();
        let mut cfg = config(&title, std::ptr::null(), &text, &buttons);
        cfg.dwFlags |= TDF_SIZE_TO_CONTENT;
        let mut clicked = 0i32;
        // SAFETY: every pointer in cfg points into the Vecs above, alive until the dialog returns.
        let hr = unsafe { show(&cfg, &mut clicked, std::ptr::null_mut(), std::ptr::null_mut()) };
        if hr != S_OK {
            return None;
        }
        let i = clicked - ID_FIRST;
        Some((0..labels.len() as i32).contains(&i).then_some(i as usize))
    }

    struct State<'a> {
        progress: &'a Progress,
        /// The player clicked "取消" (the work may have refused it: too late).
        cancel_clicked: Cell<bool>,
        /// Timer ticks since the work finished.
        finished_ticks: Cell<u32>,
        shown: RefCell<String>,
    }

    fn make_lparam(low: u16, high: u16) -> LPARAM {
        ((u32::from(high) << 16) | u32::from(low)) as LPARAM
    }

    /// SAFETY (callers): `hwnd` is the task dialog's, during its callback.
    unsafe fn refresh(hwnd: HWND, state: &State) {
        let (phase, done, total) = state.progress.snapshot();
        let clicked = state.cancel_clicked.get();
        SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_POS as u32, bar(done, total) as WPARAM, 0);
        let text = shown_text(phase, done, total, clicked);
        let changed = *state.shown.borrow() != text;
        if changed {
            let w = wide(&text);
            SendMessageW(hwnd, TDM_SET_ELEMENT_TEXT as u32, TDE_CONTENT as WPARAM, w.as_ptr() as LPARAM);
            *state.shown.borrow_mut() = text;
        }
        SendMessageW(hwnd, TDM_ENABLE_BUTTON as u32, ID_STOP as WPARAM, LPARAM::from(stop_enabled(phase, clicked)));
    }

    unsafe extern "system" fn callback(hwnd: HWND, msg: i32, _: WPARAM, _: LPARAM, data: isize) -> HRESULT {
        // SAFETY: data is the &State given to TaskDialogIndirect below, alive while it runs (this callback runs
        // inside that call, on the same thread).
        let state = &*(data as *const State);
        match msg {
            TDN_CREATED => {
                SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_RANGE as u32, 0, make_lparam(0, 1000));
                refresh(hwnd, state);
                S_OK
            }
            TDN_TIMER => {
                let finished = state.progress.finished();
                let ticks = state.finished_ticks.get();
                match on_tick(finished, ticks) {
                    Tick::Refresh => refresh(hwnd, state),
                    Tick::EnableAndClick => {
                        // a disabled button may ignore TDM_CLICK_BUTTON: enabled first
                        SendMessageW(hwnd, TDM_ENABLE_BUTTON as u32, ID_STOP as WPARAM, 1);
                        SendMessageW(hwnd, TDM_CLICK_BUTTON as u32, ID_STOP as WPARAM, 0);
                    }
                    Tick::Close => {
                        PostMessageW(hwnd, WM_CLOSE, 0, 0);
                    }
                }
                if finished {
                    state.finished_ticks.set(ticks.saturating_add(1));
                }
                S_OK
            }
            // the button, Esc, Alt+F4, the title bar's X, or the program's own click / WM_CLOSE
            TDN_BUTTON_CLICKED => {
                let clicked = state.cancel_clicked.get();
                match on_button(state.progress.finished(), state.progress.can_cancel(), clicked) {
                    Button::Close => S_OK,
                    Button::Cancel => {
                        state.progress.request_cancel();
                        state.cancel_clicked.set(true);
                        refresh(hwnd, state);
                        // kept open until the work has removed what it wrote
                        S_FALSE
                    }
                    Button::Ignore => S_FALSE,
                }
            }
            _ => S_OK,
        }
    }

    /// Shows `progress` until the work is finished; `false` when there is no task dialog (nothing shown).
    pub fn show_progress(title: &str, instruction: &str, progress: &Progress) -> bool {
        let Some(show) = task_dialog() else { return false };
        let (title, instruction, content) = (wide(title), wide(instruction), wide("連線中…"));
        let stop = wide("取消");
        let buttons = [TASKDIALOG_BUTTON { nButtonID: ID_STOP, pszButtonText: stop.as_ptr() }];
        let state = State { progress, cancel_clicked: Cell::new(false), finished_ticks: Cell::new(0), shown: RefCell::new(String::new()) };
        let mut cfg = config(&title, instruction.as_ptr(), &content, &buttons);
        cfg.dwFlags |= TDF_SHOW_PROGRESS_BAR | TDF_CALLBACK_TIMER;
        cfg.pfCallback = Some(callback);
        cfg.lpCallbackData = &state as *const State as isize;
        cfg.cxWidth = 260;
        let mut clicked = 0i32;
        // SAFETY: the strings, the button and `state` outlive the call; the callback only uses them inside it.
        let hr = unsafe { show(&cfg, &mut clicked, std::ptr::null_mut(), std::ptr::null_mut()) };
        hr == S_OK
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_and_the_text_follow_the_bytes() {
        assert_eq!(bar(0, 0), 0);
        assert_eq!(bar(0, 400), 0);
        assert_eq!(bar(100, 400), 250);
        assert_eq!(bar(400, 400), 1000);
        assert_eq!(bar(999, 400), 1000);
        assert_eq!(bar(484_121_119, 484_121_119), 1000, "no overflow at the pack's size");
        assert_eq!(status_text(Phase::Downloading, 3 * 1024 * 1024, 12 * 1024 * 1024, false), "已下載 3.0／12.0 MB（25%）");
        assert_eq!(status_text(Phase::Verifying, 1, 1, false), "驗證中…");
        assert_eq!(status_text(Phase::Installing, 1, 1, false), "安裝更新中…");
        assert!(status_text(Phase::Downloading, 1, 2, true).starts_with("取消中"));
    }

    /// Why: once verifying (or swapping) the player's cancel is ignored: a half-swapped .exe is worse than waiting.
    #[test]
    fn cancel_is_honoured_only_until_verifying() {
        let p = Progress::default();
        p.request_cancel();
        assert!(p.cancelled(), "connecting");
        p.phase(Phase::Downloading);
        assert!(p.cancelled());
        p.phase(Phase::Verifying);
        assert!(!p.cancelled());
        p.phase(Phase::Installing);
        assert!(!p.cancelled());
    }

    /// Why: a cancel that came too late (the download was already checked) does not leave "cancelling" on the
    /// screen while the update goes on: the line says what is really happening.
    #[test]
    fn a_late_cancel_shows_the_real_phase() {
        assert!(shown_text(Phase::Downloading, 1, 2, true).starts_with("取消中"));
        assert!(shown_text(Phase::Connecting, 0, 2, true).starts_with("取消中"));
        assert_eq!(shown_text(Phase::Verifying, 2, 2, true), "驗證中…");
        assert_eq!(shown_text(Phase::Installing, 2, 2, true), "安裝更新中…");
        assert!(stop_enabled(Phase::Downloading, false));
        assert!(!stop_enabled(Phase::Downloading, true));
        assert!(!stop_enabled(Phase::Verifying, false));
    }

    /// Why: the window closes by itself once the work is finished — by a click on the button enabled first, and
    /// if it is still open, by WM_CLOSE every second — and that click is never taken for the player's cancel.
    #[test]
    fn the_window_closes_itself_when_the_work_is_finished() {
        assert_eq!(on_tick(false, 0), Tick::Refresh);
        assert_eq!(on_tick(true, 0), Tick::EnableAndClick);
        assert_eq!((1..5).map(|t| on_tick(true, t)).collect::<Vec<_>>(), vec![Tick::Refresh; 4]);
        assert_eq!(on_tick(true, 5), Tick::Close);
        assert_eq!(on_tick(true, 10), Tick::Close);
        // finished: the click closes, whatever else is true
        for (can, clicked) in [(true, false), (false, false), (true, true), (false, true)] {
            assert_eq!(on_button(true, can, clicked), Button::Close);
        }
        assert_eq!(on_button(false, true, false), Button::Cancel);
        assert_eq!(on_button(false, true, true), Button::Ignore);
        assert_eq!(on_button(false, false, false), Button::Ignore);
    }

    /// Why: without a task dialog (here: not Windows) the work still runs on its thread and gives the same result.
    /// Not on Windows: the test exe carries the launcher's manifest (build.rs), so there is a task dialog and
    /// `ask` would wait for a click.
    #[cfg(not(windows))]
    #[test]
    fn without_a_task_dialog_the_work_runs_and_its_result_comes_back() {
        assert_eq!(ask("t", "q", &["a", "b"]), None);
        assert!(!task_dialog_available());
        let main = std::thread::current().id();
        let (result, worker) = with_progress("t", "i", move |p: &Progress| {
            p.phase(Phase::Downloading);
            p.bytes(5, 5);
            (42, std::thread::current().id())
        })
        .unwrap();
        assert_eq!(result, 42);
        assert_ne!(worker, main, "the work ran on the main thread");
    }

    /// Why: on Windows the manifest build.rs embeds selects comctl32 v6, so `TaskDialogIndirect` is found and a
    /// download shows its progress window and the questions their own buttons (no dialog is opened here).
    #[cfg(windows)]
    #[test]
    fn with_the_manifest_the_task_dialog_is_found() {
        assert!(task_dialog_available());
    }
}
