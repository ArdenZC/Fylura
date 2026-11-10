//! Standard CF_HDROP / Preferred DropEffect, with OS-owned memory lifetime.
use super::ClipboardFiles;
use std::{mem::size_of, ptr};
use windows_sys::Win32::{
    Foundation::{GlobalFree, HGLOBAL, HWND, POINT},
    System::{
        DataExchange::{
            CloseClipboard, EmptyClipboard, GetClipboardData, OpenClipboard,
            RegisterClipboardFormatW, SetClipboardData,
        },
        Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
    },
    UI::Shell::{DragQueryFileW, DROPFILES},
};
const CF_HDROP: u32 = 15;

struct ClipboardGuard;
impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            CloseClipboard();
        }
    }
}
fn open(owner: Option<usize>) -> Result<ClipboardGuard, String> {
    let owner = owner
        .filter(|owner| *owner != 0)
        .ok_or("workspace_clipboard_unavailable")? as HWND;
    for _ in 0..5 {
        if unsafe { OpenClipboard(owner) } != 0 {
            return Ok(ClipboardGuard);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err("workspace_clipboard_busy".into())
}
fn effect_format() -> Result<u32, String> {
    let name: Vec<u16> = "Preferred DropEffect\0".encode_utf16().collect();
    let format = unsafe { RegisterClipboardFormatW(name.as_ptr()) };
    if format == 0 {
        Err("workspace_clipboard_unavailable".into())
    } else {
        Ok(format)
    }
}

pub(super) fn read(owner: Option<usize>) -> Result<ClipboardFiles, String> {
    let _clipboard = open(owner)?;
    let drop = unsafe { GetClipboardData(CF_HDROP) };
    if drop.is_null() {
        return Ok(ClipboardFiles::default());
    }
    let count = unsafe { DragQueryFileW(drop, u32::MAX, ptr::null_mut(), 0) };
    if count as usize > super::MAX_PATHS {
        return Err("workspace_clipboard_invalid_files".into());
    }
    let mut paths = Vec::with_capacity(count as usize);
    for index in 0..count {
        let length = unsafe { DragQueryFileW(drop, index, ptr::null_mut(), 0) } as usize;
        if length == 0 || length > super::MAX_PATH_LENGTH {
            return Err("workspace_clipboard_invalid_files".into());
        }
        let mut path = vec![0u16; length + 1];
        let read =
            unsafe { DragQueryFileW(drop, index, path.as_mut_ptr(), path.len() as u32) } as usize;
        if read != length {
            return Err("workspace_clipboard_invalid_files".into());
        }
        paths.push(
            String::from_utf16(&path[..length]).map_err(|_| "workspace_clipboard_invalid_files")?,
        );
    }
    let effect = unsafe { GetClipboardData(effect_format()?) };
    let mut move_files = false;
    if !effect.is_null() && unsafe { GlobalSize(effect) } >= size_of::<u32>() {
        let bytes = unsafe { GlobalLock(effect) };
        if !bytes.is_null() {
            move_files = unsafe { ptr::read_unaligned(bytes.cast::<u32>()) } & 2 != 0;
            unsafe {
                GlobalUnlock(effect);
            }
        }
    }
    Ok(ClipboardFiles { paths, move_files })
}

struct Memory(HGLOBAL);
impl Drop for Memory {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                GlobalFree(self.0);
            }
        }
    }
}
impl Memory {
    fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let memory = Self(unsafe { GlobalAlloc(GMEM_MOVEABLE, bytes.len()) });
        if memory.0.is_null() {
            return Err("workspace_clipboard_unavailable".into());
        }
        let target = unsafe { GlobalLock(memory.0) };
        if target.is_null() {
            return Err("workspace_clipboard_unavailable".into());
        }
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), target.cast(), bytes.len());
            GlobalUnlock(memory.0);
        }
        Ok(memory)
    }
    fn publish(&mut self, format: u32) -> Result<(), String> {
        if unsafe { SetClipboardData(format, self.0) }.is_null() {
            return Err("workspace_clipboard_unavailable".into());
        }
        self.0 = ptr::null_mut(); // Ownership transfers to Windows only on success.
        Ok(())
    }
}

pub(super) fn write(files: &ClipboardFiles, owner: Option<usize>) -> Result<(), String> {
    let mut names = Vec::<u16>::new();
    for path in &files.paths {
        names.extend(path.replace('/', "\\").encode_utf16());
        names.push(0);
    }
    names.push(0);
    let header = DROPFILES {
        pFiles: size_of::<DROPFILES>() as u32,
        pt: POINT { x: 0, y: 0 },
        fNC: 0,
        fWide: 1,
    };
    let mut bytes = vec![0u8; size_of::<DROPFILES>() + names.len() * size_of::<u16>()];
    unsafe {
        ptr::write_unaligned(bytes.as_mut_ptr().cast::<DROPFILES>(), header);
        ptr::copy_nonoverlapping(
            names.as_ptr().cast::<u8>(),
            bytes.as_mut_ptr().add(size_of::<DROPFILES>()),
            names.len() * size_of::<u16>(),
        );
    }
    let mut drop = Memory::from_bytes(&bytes)?;
    let mut effect =
        Memory::from_bytes(&(if files.move_files { 2u32 } else { 1u32 }).to_le_bytes())?;
    let format = effect_format()?;
    let _clipboard = open(owner)?;
    if unsafe { EmptyClipboard() } == 0 {
        return Err("workspace_clipboard_unavailable".into());
    }
    drop.publish(CF_HDROP)?;
    effect.publish(format)
}
