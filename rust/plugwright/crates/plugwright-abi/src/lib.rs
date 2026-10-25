//! Binary contract shared by the plugwright host and plugins.
//!
//! Two ABIs live here:
//!
//! * **Native** — a `#[repr(C)]` declaration table returned by the exported
//!   symbol [`NATIVE_ENTRY`]. Only C types cross the boundary, so host and
//!   plugin may be built by different `rustc` versions.
//! * **Wasm** — a core-module ABI (no component model) made of a handful of
//!   `i32`/`i64` exports, so any engine (wasmtime, wasmi, wasmer, …) can run it.
//!
//! Every change to a layout or a signature in this crate bumps [`ABI_VERSION`].
#![no_std]

use core::ffi::c_void;

/// Version of the binary contract. Host and plugin must match exactly.
pub const ABI_VERSION: u32 = 1;

/// Name of the native entry symbol (without the trailing NUL).
pub const NATIVE_ENTRY: &str = "plugwright_entry_v1";

/// Upper bound for strings in the declaration table (names, versions).
pub const MAX_DECL_STR: usize = 256;

/// Status codes returned across the native boundary. A plugin may return any
/// `u32`; the host treats unknown values as errors instead of transmuting them.
pub type PwStatus = u32;
pub const STATUS_OK: PwStatus = 0;
/// The plugin returned an error; the output buffer holds a UTF-8 message.
pub const STATUS_ERROR: PwStatus = 1;
/// The plugin panicked; the output buffer holds the panic message.
pub const STATUS_PANIC: PwStatus = 2;

/// Log levels used by both ABIs (match `tracing::Level` ordering).
pub const LOG_ERROR: u32 = 1;
pub const LOG_WARN: u32 = 2;
pub const LOG_INFO: u32 = 3;
pub const LOG_DEBUG: u32 = 4;
pub const LOG_TRACE: u32 = 5;

/// Borrowed byte string. Valid only for the duration of the call it is passed to.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PwStr {
    pub ptr: *const u8,
    pub len: usize,
}

impl PwStr {
    pub const EMPTY: PwStr = PwStr { ptr: core::ptr::null(), len: 0 };

    pub const fn from_static(s: &'static str) -> Self {
        PwStr { ptr: s.as_ptr(), len: s.len() }
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        PwStr { ptr: b.as_ptr(), len: b.len() }
    }

    /// # Safety
    /// `ptr` must point to `len` readable bytes that outlive `'a` (or be null with `len == 0`).
    pub unsafe fn as_bytes<'a>(&self) -> &'a [u8] {
        if self.ptr.is_null() || self.len == 0 {
            &[]
        } else {
            unsafe { core::slice::from_raw_parts(self.ptr, self.len) }
        }
    }
}

/// Owned buffer allocated by the plugin. The host copies it and hands it back
/// through [`PwPluginDecl::free_buf`], so it is always freed by the allocator
/// that created it.
#[repr(C)]
pub struct PwBuf {
    pub ptr: *mut u8,
    pub len: usize,
    pub cap: usize,
}

impl PwBuf {
    pub const EMPTY: PwBuf = PwBuf { ptr: core::ptr::null_mut(), len: 0, cap: 0 };
}

/// Services the host offers to a native plugin.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PwHostApi {
    /// Opaque host context; valid for the whole lifetime of the loaded library.
    pub ctx: *mut c_void,
    pub log: Option<unsafe extern "C" fn(ctx: *mut c_void, level: u32, msg: PwStr)>,
}

pub type CreateFn = unsafe extern "C" fn(
    host: *const PwHostApi,
    config: PwStr,
    out_instance: *mut *mut c_void,
    out_err: *mut PwBuf,
) -> PwStatus;
pub type CallFn = unsafe extern "C" fn(instance: *mut c_void, method: PwStr, input: PwStr, out: *mut PwBuf) -> PwStatus;
pub type DestroyFn = unsafe extern "C" fn(instance: *mut c_void);
pub type FreeBufFn = unsafe extern "C" fn(buf: PwBuf);

/// Declaration table a native plugin exposes through [`NATIVE_ENTRY`].
///
/// `abi_version` is the first field so the host can reject a foreign layout
/// after reading only four bytes.
#[repr(C)]
pub struct PwPluginDecl {
    pub abi_version: u32,
    /// `size_of::<PwPluginDecl>()` as seen by the plugin.
    pub struct_size: u32,
    pub sdk_version: PwStr,
    pub name: PwStr,
    pub version: PwStr,
    pub create: Option<CreateFn>,
    pub call: Option<CallFn>,
    pub destroy: Option<DestroyFn>,
    pub free_buf: Option<FreeBufFn>,
}

// The table is an immutable static that only holds pointers to static data.
unsafe impl Sync for PwPluginDecl {}

/// Signature of the native entry symbol.
pub type EntryFn = unsafe extern "C" fn() -> *const PwPluginDecl;

/// Core-wasm ABI. Pointers are `i32` offsets into the exported `memory`;
/// `(ptr, len)` pairs are packed into one `i64` by [`wasm::pack`].
///
/// Exports a module must provide:
///
/// | export           | signature                       | meaning                                   |
/// |------------------|---------------------------------|-------------------------------------------|
/// | `memory`         | memory                          | linear memory                             |
/// | `pw_abi_version` | `() -> i32`                     | must equal [`ABI_VERSION`]                |
/// | `pw_alloc`       | `(len: i32) -> i32`             | allocate `len` bytes for the host         |
/// | `pw_free`        | `(ptr: i32, len: i32)`          | free a buffer returned by the guest       |
/// | `pw_meta`        | `() -> i64`                     | `name\0version\0sdk_version`              |
/// | `pw_init`        | `(ptr, len) -> i64`             | create the instance from config bytes     |
/// | `pw_call`        | `(mptr, mlen, iptr, ilen) -> i64` | call a method                           |
///
/// `pw_init`/`pw_call` take ownership of the buffers the host allocated with
/// `pw_alloc` and return a buffer whose first byte is a status code
/// ([`STATUS_OK`], [`STATUS_ERROR`]) followed by the payload.
///
/// Imports, all in module `plugwright`; a module may use any subset:
///
/// | import      | signature                                       | meaning                                  |
/// |-------------|-------------------------------------------------|------------------------------------------|
/// | `log`       | `(level: i32, ptr: i32, len: i32)`              | log a UTF-8 line                         |
/// | `host_call` | `(nptr, nlen, iptr, ilen) -> i32`               | run the host function named by `nptr`    |
/// | `host_take` | `(ptr: i32, len: i32) -> i32`                   | copy the pending `host_call` result      |
///
/// `host_call` runs a function the embedder registered under that name on the
/// input bytes and returns the length of its result, a status byte followed
/// by the output or the error message, or `-1` when the name or input lies
/// outside memory. The guest allocates that many bytes and calls
/// `host_take`, which copies the result there and returns `0`, or `-1` when no
/// result of that length is pending. Two calls, because the host cannot
/// allocate in the guest while the guest is running.
pub mod wasm {
    pub const EXPORT_MEMORY: &str = "memory";
    pub const EXPORT_ABI_VERSION: &str = "pw_abi_version";
    pub const EXPORT_ALLOC: &str = "pw_alloc";
    pub const EXPORT_FREE: &str = "pw_free";
    pub const EXPORT_META: &str = "pw_meta";
    pub const EXPORT_INIT: &str = "pw_init";
    pub const EXPORT_CALL: &str = "pw_call";
    pub const IMPORT_MODULE: &str = "plugwright";
    pub const IMPORT_LOG: &str = "log";
    pub const IMPORT_HOST_CALL: &str = "host_call";
    pub const IMPORT_HOST_TAKE: &str = "host_take";
    /// Longest host function name `host_call` reads.
    pub const MAX_HOST_NAME: u32 = 256;

    pub const fn pack(ptr: u32, len: u32) -> i64 {
        (((ptr as u64) << 32) | len as u64) as i64
    }

    pub const fn unpack(v: i64) -> (u32, u32) {
        let v = v as u64;
        ((v >> 32) as u32, v as u32)
    }
}

#[cfg(test)]
mod tests {
    use core::mem::{offset_of, size_of};
    use core::ptr::NonNull;

    use super::wasm::{pack, unpack};
    use super::*;

    #[test]
    fn pack_roundtrip() {
        for (p, l) in [(0, 0), (1, 2), (u32::MAX, u32::MAX), (0x8000_0000, 7)] {
            assert_eq!(unpack(pack(p, l)), (p, l));
        }
    }

    #[test]
    fn pack_puts_ptr_in_the_high_word() {
        assert_eq!(pack(0, 0), 0);
        assert_eq!(pack(1, 2), 0x0000_0001_0000_0002);
        assert_eq!(pack(0, u32::MAX), 0xFFFF_FFFF);
        assert_eq!(pack(u32::MAX, 0), 0xFFFF_FFFF_0000_0000_u64 as i64);
    }

    #[test]
    fn pack_crosses_the_i64_sign_bit_losslessly() {
        // Wasm has no unsigned i64: a pointer with its top bit set packs negative.
        assert!(pack(0x8000_0000, 0) < 0);
        assert_eq!(pack(u32::MAX, u32::MAX), -1);
        assert_eq!(unpack(-1), (u32::MAX, u32::MAX));
        assert_eq!(unpack(i64::MIN), (0x8000_0000, 0));
        assert_eq!(unpack(i64::MAX), (0x7FFF_FFFF, u32::MAX));
        // A length with its top bit set does not spill into the pointer.
        assert_eq!(unpack(pack(0, 0x8000_0000)), (0, 0x8000_0000));
    }

    #[test]
    fn pack_and_unpack_work_in_const_context() {
        const PACKED: i64 = pack(7, 9);
        const UNPACKED: (u32, u32) = unpack(PACKED);
        assert_eq!(UNPACKED, (7, 9));
    }

    #[test]
    fn status_codes_are_distinct_and_fit_the_wasm_status_byte() {
        let statuses = [STATUS_OK, STATUS_ERROR, STATUS_PANIC];
        assert_eq!(statuses[0], 0);
        for (i, a) in statuses.iter().enumerate() {
            assert!(u8::try_from(*a).is_ok(), "{a}");
            for b in &statuses[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn log_levels_rise_with_verbosity_like_tracing() {
        assert_eq!([LOG_ERROR, LOG_WARN, LOG_INFO, LOG_DEBUG, LOG_TRACE], [1, 2, 3, 4, 5]);
    }

    #[test]
    fn pwstr_borrows_without_copying() {
        let s = PwStr::from_static("plugwright");
        assert_eq!(unsafe { s.as_bytes() }, b"plugwright");
        let owned = [1u8, 2, 3];
        let b = PwStr::from_bytes(&owned);
        assert_eq!((b.ptr, b.len), (owned.as_ptr(), 3));
        assert_eq!(unsafe { b.as_bytes() }, &owned);
        assert!(unsafe { PwStr::from_bytes(&[]).as_bytes() }.is_empty());
    }

    #[test]
    fn empty_or_null_pwstr_is_never_dereferenced() {
        assert!(unsafe { PwStr::EMPTY.as_bytes() }.is_empty());
        // A null pointer reads as empty whatever length comes with it.
        let null = PwStr { ptr: core::ptr::null(), len: 5 };
        assert!(unsafe { null.as_bytes() }.is_empty());
        let dangling = PwStr { ptr: NonNull::dangling().as_ptr(), len: 0 };
        assert!(unsafe { dangling.as_bytes() }.is_empty());
    }

    #[test]
    fn empty_pwbuf_is_null_with_no_capacity() {
        let b = PwBuf::EMPTY;
        assert!(b.ptr.is_null());
        assert_eq!((b.len, b.cap), (0, 0));
    }

    #[test]
    fn decl_layout_matches_the_c_contract() {
        // The host reads `abi_version` alone before trusting the rest.
        assert_eq!(offset_of!(PwPluginDecl, abi_version), 0);
        assert_eq!(offset_of!(PwPluginDecl, struct_size), 4);
        // Empty slots are null pointers, as C expects.
        assert_eq!(size_of::<Option<CreateFn>>(), size_of::<usize>());
        assert_eq!(size_of::<PwStr>(), 2 * size_of::<usize>());
        assert_eq!(size_of::<PwBuf>(), 3 * size_of::<usize>());
        assert_eq!(size_of::<PwPluginDecl>(), 8 + 3 * size_of::<PwStr>() + 4 * size_of::<usize>());
    }
}
