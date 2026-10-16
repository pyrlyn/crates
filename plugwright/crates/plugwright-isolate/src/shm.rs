//! A fixed-size shared memory region between the host and one runner.
//!
//! The host creates the region and hands the runner a token for it in HELLO.
//! The runner maps it before LOAD applies the sandbox and then drops its own
//! descriptor, and the host forgets the name once the runner has answered: from
//! then on the two mappings are the only references, so nothing the plugin does
//! can resize the region under the host (a shrunk POSIX object would fault the
//! host with SIGBUS on its next read).
//!
//! The host never forms a reference into the region: the runner may write it at
//! any time, so bytes are copied out with raw pointers after the range is checked
//! against the host's own size.

use std::io;
use std::process::Child;
use std::ptr::NonNull;

pub(crate) struct Region {
    ptr: NonNull<u8>,
    len: usize,
    #[cfg(unix)]
    name: Option<std::ffi::CString>,
    #[cfg(windows)]
    section: windows_sys::Win32::Foundation::HANDLE,
}

// SAFETY: the mapping is process-wide and owned by the `Region`; access goes
// through `&self` methods that copy with raw pointers.
unsafe impl Send for Region {}

impl Region {
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    /// Copy `data` to the start of the region; `false` when it does not fit.
    pub(crate) fn write(&self, data: &[u8]) -> bool {
        if data.len() > self.len {
            return false;
        }
        // SAFETY: `data.len()` bytes fit in the mapping; `data` is process memory, not the region.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), self.ptr.as_ptr(), data.len()) };
        true
    }

    /// Copy out `len` bytes at `offset`, or `None` when the range leaves the region.
    pub(crate) fn read(&self, offset: u64, len: u64) -> Option<Vec<u8>> {
        let (offset, len) = (usize::try_from(offset).ok()?, usize::try_from(len).ok()?);
        if offset.checked_add(len)? > self.len {
            return None;
        }
        let mut out = Vec::with_capacity(len);
        // SAFETY: the range was checked against the mapping; `out` has `len` bytes of capacity.
        unsafe {
            std::ptr::copy_nonoverlapping(self.ptr.as_ptr().add(offset), out.as_mut_ptr(), len);
            out.set_len(len);
        }
        Some(out)
    }

    /// The first `len` bytes, borrowed. Only for the runner, whose peer (the host)
    /// leaves the region alone while a call is in flight.
    pub(crate) fn prefix(&self, len: u64) -> Option<&[u8]> {
        let len = usize::try_from(len).ok().filter(|&l| l <= self.len)?;
        // SAFETY: in bounds; the host writes the region only between calls.
        Some(unsafe { std::slice::from_raw_parts(self.ptr.as_ptr(), len) })
    }
}

#[cfg(unix)]
impl Region {
    /// Host side: a new POSIX shared memory object of `len` bytes, mapped.
    pub(crate) fn create(len: usize) -> io::Result<Region> {
        use std::hash::{BuildHasher, RandomState};
        if len == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        // macOS limits names to 31 bytes; this one is at most 25.
        let name = format!("/pw{}-{:x}", std::process::id(), RandomState::new().hash_one(len));
        let name = std::ffi::CString::new(name).expect("no NUL");
        // SAFETY: plain syscalls on a NUL-terminated name and a descriptor this function owns.
        unsafe {
            let fd = libc::shm_open(name.as_ptr(), libc::O_RDWR | libc::O_CREAT | libc::O_EXCL, 0o600 as libc::c_uint);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            let mut region = map_fd(fd, len, libc::ftruncate(fd, len as libc::off_t) == 0);
            if let Ok(r) = &mut region {
                r.name = Some(name);
            } else {
                libc::shm_unlink(name.as_ptr());
            }
            region
        }
    }

    /// What the runner needs to open the region: its name.
    pub(crate) fn token(&self, _runner: &Child) -> io::Result<Vec<u8>> {
        Some(self.name_token()).filter(|t| !t.is_empty()).ok_or_else(|| io::Error::other("region already unlinked"))
    }

    pub(crate) fn name_token(&self) -> Vec<u8> {
        self.name.as_ref().map(|n| n.as_bytes().to_vec()).unwrap_or_default()
    }

    /// Runner side: map the region the host named.
    pub(crate) fn open(token: &[u8], len: usize) -> io::Result<Region> {
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, "bad shared memory token");
        if len == 0 || token.first() != Some(&b'/') {
            return Err(bad());
        }
        let name = std::ffi::CString::new(token).map_err(|_| bad())?;
        // SAFETY: plain syscalls on a NUL-terminated name and a descriptor this function owns.
        unsafe {
            let fd = libc::shm_open(name.as_ptr(), libc::O_RDWR, 0 as libc::c_uint);
            if fd < 0 {
                return Err(io::Error::last_os_error());
            }
            // Mapping past the object's end would fault on first touch.
            let mut st: libc::stat = std::mem::zeroed();
            let big_enough = libc::fstat(fd, &mut st) == 0 && st.st_size as u64 >= len as u64;
            map_fd(fd, len, big_enough)
        }
    }

    /// Remove the name; existing mappings stay valid.
    pub(crate) fn unlink(&mut self) {
        if let Some(name) = self.name.take() {
            // SAFETY: NUL-terminated name.
            unsafe { libc::shm_unlink(name.as_ptr()) };
        }
    }
}

/// Map `len` bytes of `fd` if `ok`, then close `fd` either way.
#[cfg(unix)]
unsafe fn map_fd(fd: libc::c_int, len: usize, ok: bool) -> io::Result<Region> {
    let result = if ok {
        // SAFETY: the caller owns `fd`; a failed mmap is reported, not used.
        let p = unsafe {
            libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ | libc::PROT_WRITE, libc::MAP_SHARED, fd, 0)
        };
        match NonNull::new(p.cast::<u8>()).filter(|_| p != libc::MAP_FAILED) {
            Some(ptr) => Ok(Region { ptr, len, name: None }),
            None => Err(io::Error::last_os_error()),
        }
    } else {
        Err(io::Error::other("shared memory object has the wrong size"))
    };
    // SAFETY: the caller hands over ownership of `fd`; the mapping does not need it.
    unsafe { libc::close(fd) };
    result
}

#[cfg(unix)]
impl Drop for Region {
    fn drop(&mut self) {
        self.unlink();
        // SAFETY: `ptr`/`len` describe a live mapping created by `map_fd`.
        unsafe { libc::munmap(self.ptr.as_ptr().cast(), self.len) };
    }
}

#[cfg(windows)]
impl Region {
    /// Host side: a pagefile-backed section of `len` bytes, mapped. Its size is
    /// fixed at creation, so the runner cannot shrink it.
    pub(crate) fn create(len: usize) -> io::Result<Region> {
        use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows_sys::Win32::System::Memory::{CreateFileMappingW, PAGE_READWRITE};
        if len == 0 {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        let size = len as u64;
        // SAFETY: an unnamed section with default security; the handle is owned below.
        let section = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null(),
                PAGE_READWRITE,
                (size >> 32) as u32,
                size as u32,
                std::ptr::null(),
            )
        };
        if section.is_null() {
            return Err(io::Error::last_os_error());
        }
        map_section(section, len).inspect_err(|_| {
            // SAFETY: the section handle is ours and unused elsewhere.
            unsafe { CloseHandle(section) };
        })
    }

    /// What the runner needs to open the region: the section handle duplicated into it.
    pub(crate) fn token(&self, runner: &Child) -> io::Result<Vec<u8>> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE};
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut dup: HANDLE = std::ptr::null_mut();
        // SAFETY: duplicates our own section handle into the runner we spawned; not inheritable.
        let ok = unsafe {
            DuplicateHandle(
                GetCurrentProcess(),
                self.section,
                runner.as_raw_handle(),
                &mut dup,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok((dup as usize as u64).to_le_bytes().to_vec())
    }

    /// Runner side: map the section handle the host duplicated into this process.
    pub(crate) fn open(token: &[u8], len: usize) -> io::Result<Region> {
        use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
        let bad = || io::Error::new(io::ErrorKind::InvalidInput, "bad shared memory token");
        let raw = u64::from_le_bytes(token.try_into().map_err(|_| bad())?);
        if len == 0 || raw == 0 {
            return Err(bad());
        }
        let section = raw as usize as HANDLE;
        // MapViewOfFile fails when `len` exceeds the section, so the size needs no separate check.
        let region = map_section(section, len);
        // The runner keeps only the view: the plugin gets no handle to the section.
        // SAFETY: the host duplicated this handle for us to own.
        unsafe { CloseHandle(section) };
        region.map(|mut r| {
            r.section = std::ptr::null_mut();
            r
        })
    }

    pub(crate) fn unlink(&mut self) {}
}

#[cfg(windows)]
fn map_section(section: windows_sys::Win32::Foundation::HANDLE, len: usize) -> io::Result<Region> {
    use windows_sys::Win32::System::Memory::{FILE_MAP_ALL_ACCESS, MapViewOfFile};
    // SAFETY: a valid section handle; a null view is reported, not used.
    let view = unsafe { MapViewOfFile(section, FILE_MAP_ALL_ACCESS, 0, 0, len) };
    match NonNull::new(view.Value.cast::<u8>()) {
        Some(ptr) => Ok(Region { ptr, len, section }),
        None => Err(io::Error::last_os_error()),
    }
}

#[cfg(windows)]
impl Drop for Region {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Memory::{MEMORY_MAPPED_VIEW_ADDRESS, UnmapViewOfFile};
        // SAFETY: the view and the section handle (if any) belong to this region.
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS { Value: self.ptr.as_ptr().cast() });
            if !self.section.is_null() {
                CloseHandle(self.section);
            }
        }
    }
}

#[cfg(not(any(unix, windows)))]
impl Region {
    pub(crate) fn create(_: usize) -> io::Result<Region> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(crate) fn token(&self, _: &Child) -> io::Result<Vec<u8>> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(crate) fn open(_: &[u8], _: usize) -> io::Result<Region> {
        Err(io::ErrorKind::Unsupported.into())
    }
    pub(crate) fn unlink(&mut self) {}
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    // Multiples of the largest page size, so the object's size is exactly this.
    const LEN: usize = 1 << 16;

    fn token(r: &Region) -> Vec<u8> {
        let t = r.name_token();
        assert!(!t.is_empty(), "not unlinked");
        t
    }

    #[test]
    fn two_mappings_see_each_others_writes() {
        let host = Region::create(LEN).unwrap();
        let runner = Region::open(&token(&host), LEN).unwrap();
        assert!(runner.name.is_none(), "only the creator unlinks");
        assert!(host.write(b"request"));
        assert_eq!(runner.prefix(7), Some(&b"request"[..]));
        assert!(runner.write(b"reply!"));
        assert_eq!(host.read(0, 6).as_deref(), Some(&b"reply!"[..]));
        assert_eq!(host.read(2, 4).as_deref(), Some(&b"ply!"[..]));
        assert_eq!(host.read(LEN as u64, 0), Some(Vec::new()), "an empty range at the end is in bounds");
    }

    #[test]
    fn ranges_outside_the_region_are_refused() {
        let r = Region::create(LEN).unwrap();
        let len = LEN as u64;
        for (offset, n) in [(len, 1), (0, len + 1), (1, len), (u64::MAX, 2), (2, u64::MAX), (u64::MAX, u64::MAX)] {
            assert_eq!(r.read(offset, n), None, "offset {offset}, length {n}");
        }
        assert_eq!(r.prefix(len + 1), None);
        assert_eq!(r.prefix(u64::MAX), None);
        assert!(!r.write(&vec![1; LEN + 1]), "a payload larger than the region");
        assert!(r.write(&vec![1; LEN]));
    }

    #[test]
    fn open_checks_the_token_and_the_size() {
        let r = Region::create(LEN).unwrap();
        let e = Region::open(&token(&r), 2 * LEN).err().expect("larger than the object");
        assert!(e.to_string().contains("wrong size"), "{e}");
        for bad in [&b""[..], b"no-slash", b"/nul\0inside"] {
            let e = Region::open(bad, LEN).err().expect("bad token");
            assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
        }
        assert_eq!(Region::open(&token(&r), 0).err().unwrap().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(Region::create(0).err().unwrap().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn unlink_and_drop_remove_the_name_but_keep_live_mappings() {
        let mut host = Region::create(LEN).unwrap();
        let name = token(&host);
        let runner = Region::open(&name, LEN).unwrap();
        host.unlink();
        assert_eq!(Region::open(&name, LEN).err().unwrap().kind(), io::ErrorKind::NotFound);
        assert!(host.write(b"still shared"));
        assert_eq!(runner.prefix(12), Some(&b"still shared"[..]));

        let dropped = Region::create(LEN).unwrap();
        let name = token(&dropped);
        drop(dropped);
        assert_eq!(Region::open(&name, LEN).err().unwrap().kind(), io::ErrorKind::NotFound);
    }
}
