//! BAR0 through a memory-mapped `resource0` file: the real hardware backend.
//!
//! On Linux each PCI device exposes its BAR0 as
//! `/sys/bus/pci/devices/<bdf>/resource0`. Mapping it needs root, and kernel
//! lockdown (Secure Boot) can forbid it.

use std::fs::OpenOptions;
use std::io;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::Path;
use std::ptr::NonNull;

use crate::bar0::{Access, Bar0};
use crate::error::Error;
use crate::regs::{BAR0_SIZE, NO_READ};

pub struct MmapBar0 {
    base: NonNull<u8>,
}

impl MmapBar0 {
    pub fn open(path: &Path, access: Access) -> Result<Self, Error> {
        let writable = access == Access::ReadWrite;
        let file = OpenOptions::new()
            .read(true)
            .write(writable)
            .custom_flags(libc::O_SYNC)
            .open(path)
            .map_err(|e| io_error("open", path, &e))?;
        let len = file
            .metadata()
            .map_err(|e| io_error("stat", path, &e))?
            .len();
        if len < BAR0_SIZE as u64 {
            return Err(Error::Io(format!(
                "{}: {len} bytes, expected a 16 MiB BAR0",
                path.display()
            )));
        }
        let prot = if writable {
            libc::PROT_READ | libc::PROT_WRITE
        } else {
            libc::PROT_READ
        };
        // SAFETY: maps BAR0_SIZE bytes of a file just checked to be at least that long.
        let ptr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                BAR0_SIZE,
                prot,
                libc::MAP_SHARED,
                file.as_raw_fd(),
                0,
            )
        };
        if ptr == libc::MAP_FAILED {
            return Err(io_error("mmap", path, &io::Error::last_os_error()));
        }
        NonNull::new(ptr.cast())
            .map(|base| Self { base })
            .ok_or_else(|| Error::Io(format!("mmap {}: null mapping", path.display())))
    }

    fn at(&self, off: u32) -> *mut u32 {
        assert!(
            off.is_multiple_of(4) && off as usize + 4 <= BAR0_SIZE,
            "BAR0 offset 0x{off:X}"
        );
        // SAFETY: the offset is aligned and inside the BAR0_SIZE mapping (asserted above).
        unsafe { self.base.as_ptr().add(off as usize).cast() }
    }
}

impl Bar0 for MmapBar0 {
    fn read32(&self, off: u32) -> u32 {
        // Regs refuses this window too; this guard holds even if it is bypassed.
        assert!(
            !NO_READ.contains(&off),
            "read of the write-only multicast window at 0x{off:06X}"
        );
        // SAFETY: `at` returns an aligned pointer into the live mapping.
        unsafe { self.at(off).read_volatile() }
    }

    fn write32(&self, off: u32, val: u32) {
        // SAFETY: as above; writes only reach here through a writable Regs.
        unsafe { self.at(off).write_volatile(val) }
    }
}

impl Drop for MmapBar0 {
    fn drop(&mut self) {
        // SAFETY: unmaps exactly the mapping created in `open`.
        unsafe { libc::munmap(self.base.as_ptr().cast(), BAR0_SIZE) };
    }
}

fn io_error(what: &str, path: &Path, err: &io::Error) -> Error {
    let hint = match err.kind() {
        io::ErrorKind::PermissionDenied if what == "mmap" => {
            " (kernel lockdown or Secure Boot can block BAR0 access)"
        }
        io::ErrorKind::PermissionDenied => " (needs root)",
        _ => "",
    };
    Error::Io(format!("{what} {}: {err}{hint}", path.display()))
}
