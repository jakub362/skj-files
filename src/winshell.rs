//! Windows: Explorer's own right-click menu ("Show more options") — 7-Zip, VS Code, Send to,
//! Open with, Properties, OneDrive… whatever the user has installed.

use std::cell::RefCell;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};

use windows::core::{Interface, PCSTR, PCWSTR, PSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::{
    DefSubclassProc, ILFree, IContextMenu, IContextMenu2, IContextMenu3, IShellFolder, RemoveWindowSubclass, SHBindToParent, SHGetDesktopFolder, SHParseDisplayName,
    SetWindowSubclass, CMINVOKECOMMANDINFO, CMINVOKECOMMANDINFOEX,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreatePopupMenu, DestroyMenu, KillTimer, SetForegroundWindow, SetTimer, TrackPopupMenuEx, SW_SHOWNORMAL, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    WM_DRAWITEM, WM_INITMENUPOPUP, WM_MEASUREITEM, WM_MENUCHAR,
};

const CMF_NORMAL: u32 = 0x0;
const CMF_EXPLORE: u32 = 0x4;
const CMF_CANRENAME: u32 = 0x10;
const CMF_EXTENDEDVERBS: u32 = 0x100;
const CMIC_MASK_UNICODE: u32 = 0x4000;
const CMIC_MASK_PTINVOKE: u32 = 0x2000_0000;
const GCS_VERBW: u32 = 4;

/// What happened in the menu, for the parts SKJ Files does itself.
pub enum Outcome {
    Nothing,
    Rename,
    Done,
}

struct Request {
    hwnd: HWND,
    items: Vec<PathBuf>,
    background: Option<PathBuf>,
    pt: POINT,
    extended: bool,
    done: Box<dyn FnOnce(Outcome)>,
}

thread_local! {
    static PENDING: RefCell<Option<Request>> = const { RefCell::new(None) };
    static MENU2: RefCell<Option<IContextMenu2>> = const { RefCell::new(None) };
    static MENU3: RefCell<Option<IContextMenu3>> = const { RefCell::new(None) };
}

fn wide(p: &Path) -> Vec<u16> {
    p.as_os_str().encode_wide().chain(Some(0)).collect()
}

/// Show the menu for `items` (or for the folder `background` when nothing is selected) at a
/// point in window coordinates (physical pixels). Runs a moment later, outside the UI event
/// handler, so the window keeps painting; `done` is called afterwards on the UI thread.
pub fn show(hwnd: HWND, items: Vec<PathBuf>, background: Option<PathBuf>, x: i32, y: i32, extended: bool, done: impl FnOnce(Outcome) + 'static) {
    let mut pt = POINT { x, y };
    unsafe {
        let _ = ClientToScreen(hwnd, &mut pt);
    }
    PENDING.with(|p| *p.borrow_mut() = Some(Request { hwnd, items, background, pt, extended, done: Box::new(done) }));
    unsafe {
        // a moment for our own menu to disappear from the screen first
        SetTimer(None, 0, 80, Some(timer_proc));
    }
}

unsafe extern "system" fn timer_proc(_h: HWND, _m: u32, id: usize, _t: u32) {
    unsafe {
        let _ = KillTimer(None, id);
    }
    let Some(r) = PENDING.with(|p| p.borrow_mut().take()) else { return };
    let outcome = unsafe { run(&r) }.unwrap_or(Outcome::Nothing);
    (r.done)(outcome);
}

unsafe fn context_menu(hwnd: HWND, items: &[PathBuf], background: Option<&Path>) -> windows::core::Result<IContextMenu> {
    unsafe {
        if items.is_empty() {
            let dir = background.ok_or(windows::core::Error::from(windows::Win32::Foundation::E_FAIL))?;
            let w = wide(dir);
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None)?;
            let r = SHGetDesktopFolder().and_then(|desk| {
                let folder: IShellFolder = desk.BindToObject(pidl, None)?;
                folder.CreateViewObject(hwnd)
            });
            ILFree(Some(pidl));
            return r;
        }
        // all items must share the first one's folder
        let first_parent = items[0].parent().map(|p| p.to_path_buf());
        let mut abs: Vec<*mut ITEMIDLIST> = Vec::new();
        for it in items.iter().filter(|p| p.parent().map(|x| x.to_path_buf()) == first_parent) {
            let w = wide(it);
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None).is_ok() && !pidl.is_null() {
                abs.push(pidl);
            }
        }
        if abs.is_empty() {
            return Err(windows::Win32::Foundation::E_FAIL.into());
        }
        let mut children: Vec<*const ITEMIDLIST> = Vec::new();
        let mut folder: Option<IShellFolder> = None;
        for &p in &abs {
            let mut last: *mut ITEMIDLIST = std::ptr::null_mut();
            if let Ok(f) = SHBindToParent::<IShellFolder>(p, Some(&mut last)) {
                if folder.is_none() {
                    folder = Some(f);
                }
                children.push(last as *const ITEMIDLIST);
            }
        }
        let r = match &folder {
            Some(f) => f.GetUIObjectOf(hwnd, &children, None),
            None => Err(windows::Win32::Foundation::E_FAIL.into()),
        };
        for p in abs {
            ILFree(Some(p));
        }
        r
    }
}

unsafe extern "system" fn subclass_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if matches!(msg, WM_INITMENUPOPUP | WM_DRAWITEM | WM_MEASUREITEM | WM_MENUCHAR) {
        // submenus like "Send to" and "Open with" are filled in by the shell on demand
        let handled = MENU3.with(|m| {
            m.borrow().as_ref().map(|cm| {
                let mut res = LRESULT(0);
                unsafe { cm.HandleMenuMsg2(msg, wparam, lparam, Some(&mut res)) }.ok().map(|_| res)
            })
        });
        if let Some(Some(res)) = handled {
            return res;
        }
        let handled2 = MENU2.with(|m| m.borrow().as_ref().map(|cm| unsafe { cm.HandleMenuMsg(msg, wparam, lparam) }.is_ok()));
        if handled2 == Some(true) {
            return LRESULT(0);
        }
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

unsafe fn run(r: &Request) -> windows::core::Result<Outcome> {
    unsafe {
        let cm = context_menu(r.hwnd, &r.items, r.background.as_deref())?;
        let menu = CreatePopupMenu()?;
        let mut flags = CMF_NORMAL | CMF_EXPLORE | CMF_CANRENAME;
        if r.extended {
            flags |= CMF_EXTENDEDVERBS;
        }
        let hr = cm.QueryContextMenu(menu, 0, 1, 0x7FFF, flags);
        if hr.is_err() {
            let _ = DestroyMenu(menu);
            return Err(hr.into());
        }
        MENU3.with(|m| *m.borrow_mut() = cm.cast::<IContextMenu3>().ok());
        MENU2.with(|m| *m.borrow_mut() = cm.cast::<IContextMenu2>().ok());
        let _ = SetWindowSubclass(r.hwnd, Some(subclass_proc), 0x534B4A, 0);
        let _ = SetForegroundWindow(r.hwnd);
        let cmd = TrackPopupMenuEx(menu, (TPM_RETURNCMD | TPM_RIGHTBUTTON).0, r.pt.x, r.pt.y, r.hwnd, None).0;
        let _ = RemoveWindowSubclass(r.hwnd, Some(subclass_proc), 0x534B4A);
        MENU3.with(|m| *m.borrow_mut() = None);
        MENU2.with(|m| *m.borrow_mut() = None);
        let _ = DestroyMenu(menu);
        if cmd <= 0 {
            return Ok(Outcome::Nothing);
        }
        let offset = (cmd - 1) as usize;
        // the shell's "Rename" would edit inside Explorer's view — do our own in-place rename
        let mut verb = [0u8; 128];
        if cm.GetCommandString(offset, GCS_VERBW, None, PSTR(verb.as_mut_ptr()), 64).is_ok() {
            let w: Vec<u16> = verb.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&c| c != 0).collect();
            if String::from_utf16_lossy(&w).eq_ignore_ascii_case("rename") {
                return Ok(Outcome::Rename);
            }
        }
        let info = CMINVOKECOMMANDINFOEX {
            cbSize: std::mem::size_of::<CMINVOKECOMMANDINFOEX>() as u32,
            fMask: CMIC_MASK_UNICODE | CMIC_MASK_PTINVOKE,
            hwnd: r.hwnd,
            lpVerb: PCSTR(offset as *const u8),
            lpVerbW: PCWSTR(offset as *const u16),
            nShow: SW_SHOWNORMAL.0,
            ptInvoke: r.pt,
            ..Default::default()
        };
        cm.InvokeCommand(&info as *const CMINVOKECOMMANDINFOEX as *const CMINVOKECOMMANDINFO)?;
        Ok(Outcome::Done)
    }
}
