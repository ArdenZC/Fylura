"""Native file handles only; no application, services, registry or system policy."""
import ctypes
import os
import platform
from pathlib import Path

WINDOWS = platform.system() == 'Windows'
if WINDOWS:
    from ctypes import wintypes as w
    k = ctypes.WinDLL('kernel32', use_last_error=True)
    k.CreateFileW.argtypes = [w.LPCWSTR, w.DWORD, w.DWORD, ctypes.c_void_p, w.DWORD, w.DWORD, w.HANDLE]
    k.CreateFileW.restype = w.HANDLE
    k.CloseHandle.argtypes = [w.HANDLE]
    k.CloseHandle.restype = w.BOOL
    class Info(ctypes.Structure):
        _fields_ = [('attributes', w.DWORD), ('created', w.FILETIME), ('accessed', w.FILETIME),
                    ('written', w.FILETIME), ('volume', w.DWORD), ('size_hi', w.DWORD),
                    ('size_lo', w.DWORD), ('links', w.DWORD), ('index_hi', w.DWORD), ('index_lo', w.DWORD)]
    k.GetFileInformationByHandle.argtypes = [w.HANDLE, ctypes.POINTER(Info)]
    k.GetFileInformationByHandle.restype = w.BOOL
    k.GetFinalPathNameByHandleW.argtypes = [w.HANDLE, w.LPWSTR, w.DWORD, w.DWORD]
    k.GetFinalPathNameByHandleW.restype = w.DWORD


def win_open(path, access=0x80000000, share=7):
    h = k.CreateFileW(str(Path(path).absolute()), access, share, None, 3, 0x80, None)
    err = ctypes.get_last_error() if h == ctypes.c_void_p(-1).value else 0
    result = dict(api='CreateFileW', path=str(path), access=access, share=share,
                  disposition='OPEN_EXISTING', handle=h, winerror=err, ok=not err)
    if not err:
        result['identity'] = win_info(h)
    return h if not err else None, result


def win_info(h):
    info = Info()
    ok = k.GetFileInformationByHandle(h, ctypes.byref(info))
    buf = ctypes.create_unicode_buffer(32768)
    n = k.GetFinalPathNameByHandleW(h, buf, len(buf), 0)
    return dict(api='GetFileInformationByHandle', ok=bool(ok),
                winerror=0 if ok else ctypes.get_last_error(), volume_serial=info.volume,
                file_index=(info.index_hi << 32) | info.index_lo, links=info.links,
                attributes=info.attributes, final_path=buf.value if n else None)


def identity(path):
    p = Path(path)
    if WINDOWS:
        h, result = win_open(p, access=0, share=7)
        if h is not None:
            result['CloseHandle'] = bool(k.CloseHandle(h))
        return result
    try:
        fd = os.open(p, os.O_RDONLY)
        s = os.fstat(fd)
        result = dict(api='open/fstat', path=str(p), fd=fd, device=s.st_dev, inode=s.st_ino,
                      links=s.st_nlink, mode=s.st_mode, size=s.st_size, realpath=os.path.realpath(p))
        os.close(fd)
        return result
    except OSError as e:
        return dict(path=str(p), errno=e.errno, error=str(e))


def identities(path):
    return {suffix or 'main': identity(str(path) + suffix) for suffix in ('', '-wal', '-shm')}
