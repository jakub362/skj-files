// SKJ Files addition: native Windows drag and drop (OLE).
//
// * Files dragged in from Explorer, the desktop or any other app reach Slint's `DropArea`s through
//   `WindowInner::process_drag_event`, with live hover feedback and the real copy/move cursor.
// * A drag that starts on a `DragArea` carrying file paths runs as a real OLE drag, so it can leave
//   the window and be dropped into Explorer, the desktop, mail apps, … (and back into this window).
//
// winit's own file-drop handler is replaced by ours, because winit only reports dropped paths
// without a position, so a drop could never target a folder row.

use std::cell::RefCell;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::rc::Weak;

use i_slint_core as corelib;
use corelib::api::LogicalPosition;
use corelib::data_transfer::DataTransfer;
use corelib::input::BackendDragEvent;
use corelib::items::{AllowedDragActions, DragAction, DropEvent};
use corelib::window::{WindowAdapter, WindowInner};

use windows::Win32::Foundation::{
    DATA_S_SAMEFORMATETC, DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS,
    DV_E_FORMATETC, E_FAIL, E_NOTIMPL, E_OUTOFMEMORY, E_POINTER, HWND, OLE_E_ADVISENOTSUPPORTED,
    POINT, POINTL, S_OK,
};
use windows::Win32::Graphics::Gdi::ScreenToClient;
use windows::Win32::System::Com::{
    DATADIR_GET, DVASPECT_CONTENT, FORMATETC, IAdviseSink, IDataObject, IDataObject_Impl,
    IEnumFORMATETC, IEnumSTATDATA, STGMEDIUM, STGMEDIUM_0, TYMED_HGLOBAL,
};
use windows::Win32::System::Memory::{GMEM_MOVEABLE, GMEM_ZEROINIT, GlobalAlloc, GlobalLock, GlobalUnlock};
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_LINK, DROPEFFECT_MOVE, DROPEFFECT_NONE,
    DoDragDrop, IDropSource, IDropSource_Impl, IDropTarget, IDropTarget_Impl, OleInitialize,
    RegisterDragDrop, ReleaseStgMedium, RevokeDragDrop,
};
use windows::Win32::System::SystemServices::{
    MK_CONTROL, MK_LBUTTON, MK_SHIFT, MODIFIERKEYS_FLAGS,
};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DROPFILES, DragQueryFileW, HDROP, ILFree, SHCreateDataObject, SHCreateStdEnumFmtEtc,
    SHDoDragDrop, SHParseDisplayName,
};
use windows::Win32::UI::WindowsAndMessaging::{KillTimer, SetTimer};
use windows::core::{BOOL, HRESULT, PCWSTR, Ref, implement};

use crate::winitwindowadapter::WinitWindowAdapter;

pub(crate) fn hwnd_of(window: &winit::window::Window) -> Option<HWND> {
    use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match window.window_handle().ok()?.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Replace winit's drop target on this window with ours.
pub(crate) fn register(window: &winit::window::Window, adapter: Weak<WinitWindowAdapter>) {
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        // winit already initialised OLE for its own drop target; this is a no-op then.
        let _ = OleInitialize(None);
        let _ = RevokeDragDrop(hwnd);
        let target: IDropTarget = DropTarget { hwnd, adapter, data: RefCell::new(None) }.into();
        let _ = RegisterDragDrop(hwnd, &target);
    }
}

fn effect_to_allowed(e: DROPEFFECT) -> AllowedDragActions {
    AllowedDragActions {
        copy: e.0 & DROPEFFECT_COPY.0 != 0,
        move_: e.0 & DROPEFFECT_MOVE.0 != 0,
        link: e.0 & DROPEFFECT_LINK.0 != 0,
    }
}

fn action_to_effect(a: Option<DragAction>) -> DROPEFFECT {
    match a {
        Some(DragAction::Copy) => DROPEFFECT_COPY,
        Some(DragAction::Move) => DROPEFFECT_MOVE,
        Some(DragAction::Link) => DROPEFFECT_LINK,
        _ => DROPEFFECT_NONE,
    }
}

fn effect_to_action(e: DROPEFFECT) -> DragAction {
    if e.0 & DROPEFFECT_MOVE.0 != 0 {
        DragAction::Move
    } else if e.0 & DROPEFFECT_COPY.0 != 0 {
        DragAction::Copy
    } else if e.0 & DROPEFFECT_LINK.0 != 0 {
        DragAction::Link
    } else {
        DragAction::None
    }
}

/// Like Explorer: Ctrl = copy, Shift = move, Ctrl+Shift = link, otherwise move (the target may
/// still pick copy, e.g. for another drive).
fn propose(keys: MODIFIERKEYS_FLAGS, allowed: AllowedDragActions) -> DragAction {
    let ctrl = keys.0 & MK_CONTROL.0 != 0;
    let shift = keys.0 & MK_SHIFT.0 != 0;
    let wanted = match (ctrl, shift) {
        (true, true) => DragAction::Link,
        (true, false) => DragAction::Copy,
        _ => DragAction::Move,
    };
    let ok = |a: DragAction| match a {
        DragAction::Copy => allowed.copy,
        DragAction::Move => allowed.move_,
        DragAction::Link => allowed.link,
        _ => false,
    };
    if ok(wanted) {
        wanted
    } else if allowed.move_ {
        DragAction::Move
    } else if allowed.copy {
        DragAction::Copy
    } else if allowed.link {
        DragAction::Link
    } else {
        DragAction::None
    }
}

fn read_file_paths(obj: &IDataObject) -> Vec<PathBuf> {
    let fmt = FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    };
    let mut out = Vec::new();
    unsafe {
        let Ok(mut medium) = obj.GetData(&fmt) else { return out };
        let hdrop = HDROP(medium.u.hGlobal.0);
        let n = DragQueryFileW(hdrop, u32::MAX, None);
        for i in 0..n {
            let len = DragQueryFileW(hdrop, i, None) as usize;
            let mut buf = vec![0u16; len + 1];
            let got = DragQueryFileW(hdrop, i, Some(&mut buf)) as usize;
            buf.truncate(got);
            out.push(PathBuf::from(std::ffi::OsString::from_wide(&buf)));
        }
        ReleaseStgMedium(&mut medium);
    }
    out
}

#[implement(IDropTarget)]
struct DropTarget {
    hwnd: HWND,
    adapter: Weak<WinitWindowAdapter>,
    data: RefCell<Option<DataTransfer>>,
}

impl DropTarget {
    fn dispatch(
        &self,
        keys: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
        drop: bool,
    ) -> windows::core::Result<()> {
        if pdweffect.is_null() {
            return Ok(());
        }
        let source_allows = unsafe { *pdweffect };
        let data = self.data.borrow().clone();
        let (Some(data), Some(adapter)) = (data, self.adapter.upgrade()) else {
            unsafe { *pdweffect = DROPEFFECT_NONE };
            return Ok(());
        };
        let mut p = POINT { x: pt.x, y: pt.y };
        unsafe {
            let _ = ScreenToClient(self.hwnd, &mut p);
        }
        let sf = adapter.window().scale_factor().max(0.1);
        let allowed = effect_to_allowed(source_allows);
        let mut event = DropEvent::default();
        event.data = data;
        event.position = LogicalPosition::new(p.x as f32 / sf, p.y as f32 / sf);
        event.proposed_action = propose(keys, allowed);
        let inner = WindowInner::from_pub(adapter.window());
        if drop && std::env::var_os("SKJ_DND_DEBUG").is_some() {
            eprintln!("[skj dnd] drop at {:?} keys {:#x} source allows {:#x}", (p.x, p.y), keys.0, source_allows.0);
        }
        let result = if drop {
            inner.process_drag_event(BackendDragEvent::Drop { event, allowed })
        } else {
            inner.process_drag_event(BackendDragEvent::Move { event, allowed })
        };
        repaint_now(&adapter);
        let mut effect = action_to_effect(result);
        if effect.0 & source_allows.0 == 0 {
            effect = DROPEFFECT_NONE;
        }
        // SKJ Files performs moves itself. Reporting "copy" back to the source makes sure the
        // source (e.g. Explorer) never deletes the originals on its own afterwards.
        if drop && effect == DROPEFFECT_MOVE && source_allows.0 & DROPEFFECT_COPY.0 != 0 {
            effect = DROPEFFECT_COPY;
        }
        unsafe { *pdweffect = effect };
        Ok(())
    }
}

impl IDropTarget_Impl for DropTarget_Impl {
    fn DragEnter(
        &self,
        pdataobj: Ref<IDataObject>,
        grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let paths = pdataobj.ok().map(read_file_paths).unwrap_or_default();
        *self.data.borrow_mut() = if paths.is_empty() {
            None
        } else {
            let mut dt = DataTransfer::default();
            dt.set_file_paths(paths);
            Some(dt)
        };
        self.dispatch(grfkeystate, pt, pdweffect, false)
    }

    fn DragOver(
        &self,
        grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        self.dispatch(grfkeystate, pt, pdweffect, false)
    }

    fn DragLeave(&self) -> windows::core::Result<()> {
        if self.data.borrow_mut().take().is_some() {
            if let Some(adapter) = self.adapter.upgrade() {
                WindowInner::from_pub(adapter.window()).process_drag_event(BackendDragEvent::Leave);
                repaint_now(&adapter);
            }
        }
        Ok(())
    }

    fn Drop(
        &self,
        pdataobj: Ref<IDataObject>,
        grfkeystate: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        pdweffect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        if self.data.borrow().is_none() {
            let paths = pdataobj.ok().map(read_file_paths).unwrap_or_default();
            if !paths.is_empty() {
                let mut dt = DataTransfer::default();
                dt.set_file_paths(paths);
                *self.data.borrow_mut() = Some(dt);
            }
        }
        let r = self.dispatch(grfkeystate, pt, pdweffect, true);
        *self.data.borrow_mut() = None;
        r
    }
}

/// Redraws are normally paced by a Slint timer, but timers don't run inside OLE's modal drag
/// loop; ask for the paint directly so drop targets light up while dragging.
fn repaint_now(adapter: &WinitWindowAdapter) {
    if adapter.pending_redraw() {
        if let Some(w) = adapter.winit_window() {
            w.request_redraw();
        }
    }
}

#[implement(IDropSource)]
struct DropSource;

impl IDropSource_Impl for DropSource_Impl {
    fn QueryContinueDrag(&self, fescapepressed: BOOL, grfkeystate: MODIFIERKEYS_FLAGS) -> HRESULT {
        if fescapepressed.as_bool() {
            DRAGDROP_S_CANCEL
        } else if grfkeystate.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }

    fn GiveFeedback(&self, _dweffect: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

struct PendingDrag {
    hwnd: HWND,
    adapter: Weak<WinitWindowAdapter>,
    paths: Vec<PathBuf>,
    allowed: DROPEFFECT,
}

thread_local! {
    static PENDING: RefCell<Option<PendingDrag>> = const { RefCell::new(None) };
}

/// Start an OLE drag for `paths`. Called from inside Slint's mouse handling, so the actual
/// (modal) drag starts a moment later from a thread timer, outside of winit's event handler —
/// that way the window keeps repainting (and showing drop targets) while the drag runs.
pub(crate) fn start_drag(
    window: &winit::window::Window,
    adapter: Weak<WinitWindowAdapter>,
    paths: Vec<PathBuf>,
    allowed: AllowedDragActions,
) -> bool {
    let Some(hwnd) = hwnd_of(window) else { return false };
    if paths.is_empty() {
        return false;
    }
    let mut effect = 0u32;
    if allowed.copy {
        effect |= DROPEFFECT_COPY.0;
    }
    if allowed.move_ {
        effect |= DROPEFFECT_MOVE.0;
    }
    if allowed.link {
        effect |= DROPEFFECT_LINK.0;
    }
    PENDING.with(|p| {
        *p.borrow_mut() = Some(PendingDrag { hwnd, adapter, paths, allowed: DROPEFFECT(effect) })
    });
    unsafe { SetTimer(None, 0, 1, Some(drag_timer_proc)) != 0 }
}

unsafe extern "system" fn drag_timer_proc(_hwnd: HWND, _msg: u32, id: usize, _time: u32) {
    unsafe {
        let _ = KillTimer(None, id);
    }
    let Some(d) = PENDING.with(|p| p.borrow_mut().take()) else { return };
    let result = unsafe { run_drag(d.hwnd, &d.paths, d.allowed) };
    if std::env::var_os("SKJ_DND_DEBUG").is_some() {
        eprintln!("[skj dnd] native drag of {} item(s): {:?}", d.paths.len(), result.as_ref().map(|e| e.0));
    }
    let Some(adapter) = d.adapter.upgrade() else { return };
    let inner = WindowInner::from_pub(adapter.window());
    match result {
        Ok(effect) => {
            inner.report_drag_finished(effect_to_action(effect));
            // OLE swallowed the button release: reset the pointer state.
            adapter.after_native_drag();
        }
        Err(_) => inner.start_in_window_drag(),
    }
}

/// The shell's own data object: Explorer gets everything it knows (drag image, shortcuts, …).
unsafe fn shell_data_object(paths: &[PathBuf]) -> windows::core::Result<IDataObject> {
    let mut pidls: Vec<*mut ITEMIDLIST> = Vec::new();
    for p in paths {
        let wide: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
        if unsafe { SHParseDisplayName(PCWSTR(wide.as_ptr()), None, &mut pidl, 0, None) }.is_ok()
            && !pidl.is_null()
        {
            pidls.push(pidl);
        }
    }
    if pidls.is_empty() {
        return Err(E_FAIL.into());
    }
    let consts: Vec<*const ITEMIDLIST> = pidls.iter().map(|p| *p as *const ITEMIDLIST).collect();
    let obj: windows::core::Result<IDataObject> =
        unsafe { SHCreateDataObject(None, Some(&consts), None::<&IDataObject>) };
    for p in pidls {
        unsafe { ILFree(Some(p)) };
    }
    obj
}

unsafe fn run_drag(hwnd: HWND, paths: &[PathBuf], allowed: DROPEFFECT) -> windows::core::Result<DROPEFFECT> {
    // fall back to a plain file list where the shell can't make one
    let obj = match unsafe { shell_data_object(paths) } {
        Ok(o) => o,
        Err(_) => FileList { paths: paths.to_vec() }.into(),
    };
    let source: IDropSource = DropSource.into();
    match unsafe { SHDoDragDrop(Some(hwnd), &obj, &source, allowed) } {
        Err(e) if e.code() == E_NOTIMPL => {
            let mut effect = DROPEFFECT_NONE;
            let hr = unsafe { DoDragDrop(&obj, &source, allowed, &mut effect) };
            if hr == DRAGDROP_S_DROP || hr == DRAGDROP_S_CANCEL || hr.is_ok() {
                Ok(if hr == DRAGDROP_S_CANCEL { DROPEFFECT_NONE } else { effect })
            } else {
                Err(hr.into())
            }
        }
        r => r,
    }
}

fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}

/// Minimal data object: the dragged files as CF_HDROP — what Explorer and nearly every app accept.
#[implement(IDataObject)]
struct FileList {
    paths: Vec<PathBuf>,
}

impl IDataObject_Impl for FileList_Impl {
    fn GetData(&self, pformatetcin: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        let f = unsafe { pformatetcin.as_ref() }.ok_or(windows::core::Error::from(E_POINTER))?;
        if f.cfFormat != CF_HDROP.0 || f.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
            return Err(DV_E_FORMATETC.into());
        }
        let mut wide: Vec<u16> = Vec::new();
        for p in &self.paths {
            wide.extend(p.as_os_str().encode_wide());
            wide.push(0);
        }
        wide.push(0);
        let header = std::mem::size_of::<DROPFILES>();
        let bytes = header + wide.len() * 2;
        unsafe {
            let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes)?;
            let ptr = GlobalLock(h) as *mut u8;
            if ptr.is_null() {
                return Err(E_OUTOFMEMORY.into());
            }
            let df = ptr as *mut DROPFILES;
            (*df).pFiles = header as u32;
            (*df).fWide = BOOL(1);
            std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr.add(header), wide.len() * 2);
            let _ = GlobalUnlock(h);
            Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 { hGlobal: h },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            })
        }
    }

    fn GetDataHere(&self, _pformatetc: *const FORMATETC, _pmedium: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, pformatetc: *const FORMATETC) -> HRESULT {
        match unsafe { pformatetc.as_ref() } {
            Some(f) if f.cfFormat == CF_HDROP.0 && f.tymed & TYMED_HGLOBAL.0 as u32 != 0 => S_OK,
            _ => DV_E_FORMATETC,
        }
    }

    fn GetCanonicalFormatEtc(&self, _pformatectin: *const FORMATETC, pformatetcout: *mut FORMATETC) -> HRESULT {
        if let Some(o) = unsafe { pformatetcout.as_mut() } {
            o.ptd = std::ptr::null_mut();
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(&self, _pformatetc: *const FORMATETC, _pmedium: *const STGMEDIUM, _frelease: BOOL) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn EnumFormatEtc(&self, dwdirection: u32) -> windows::core::Result<IEnumFORMATETC> {
        if dwdirection == DATADIR_GET.0 as u32 {
            unsafe { SHCreateStdEnumFmtEtc(&[hdrop_format()]) }
        } else {
            Err(E_NOTIMPL.into())
        }
    }

    fn DAdvise(&self, _pformatetc: *const FORMATETC, _advf: u32, _padvsink: Ref<IAdviseSink>) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _dwconnection: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}
