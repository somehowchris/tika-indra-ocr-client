"""Launch a scratch-only Docker daemon in a mapped user and mount namespace."""
import ctypes
import os
from pathlib import Path
import sys

libc = ctypes.CDLL(None, use_errno=True)
ready_read, ready_write = os.pipe()
go_read, go_write = os.pipe()
child = os.fork()
if child == 0:
    os.close(ready_read)
    os.close(go_write)
    os.setgroups([])
    if libc.unshare(0x10000000) != 0:  # CLONE_NEWUSER
        raise OSError(ctypes.get_errno(), "unshare user")
    os.write(ready_write, b"1")
    os.close(ready_write)
    if os.read(go_read, 1) != b"1":
        raise RuntimeError("namespace mapping was not installed")
    os.close(go_read)
    if libc.unshare(0x00020000) != 0:  # CLONE_NEWNS
        raise OSError(ctypes.get_errno(), "unshare mount")
    if libc.mount(None, b"/", None, 16384 | (1 << 18), None) != 0:
        raise OSError(ctypes.get_errno(), "make mounts private")
    os.execvpe(sys.argv[1], sys.argv[1:], os.environ)

os.close(ready_write)
os.close(go_read)
if os.read(ready_read, 1) != b"1":
    raise RuntimeError("child namespace creation failed")
os.close(ready_read)
try:
    Path(f"/proc/{child}/uid_map").write_text("0 0 65536\n")
    Path(f"/proc/{child}/gid_map").write_text("0 0 65536\n")
    os.write(go_write, b"1")
finally:
    os.close(go_write)
_, status = os.waitpid(child, 0)
sys.exit(os.waitstatus_to_exitcode(status))
