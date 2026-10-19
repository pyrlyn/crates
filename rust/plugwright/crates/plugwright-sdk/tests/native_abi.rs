//! The native C ABI emitted by `#[plugin]` + `export_plugin!`, driven the way
//! the host loader drives it — through the declaration table, without dlopen.
#![cfg(not(target_arch = "wasm32"))]

use std::ffi::c_void;
use std::sync::Mutex;

use plugwright_sdk::abi::{
    ABI_VERSION, LOG_INFO, PwBuf, PwHostApi, PwPluginDecl, PwStr, STATUS_ERROR, STATUS_OK, STATUS_PANIC,
};
use plugwright_sdk::{SDK_VERSION, log, plugin};

struct Probe {
    prefix: String,
}

#[plugin]
impl Probe {
    fn new(prefix: &str) -> Result<Self, String> {
        if prefix == "bad" { Err("refusing config `bad`".into()) } else { Ok(Probe { prefix: prefix.to_owned() }) }
    }

    fn tag(&self, s: &str) -> String {
        format!("{}{s}", self.prefix)
    }

    fn boom(&self) {
        panic!("boom from the plugin")
    }

    fn say(&self, msg: &str) {
        log::info(msg);
    }
}

unsafe extern "C" {
    fn plugwright_entry_v1() -> *const PwPluginDecl;
}

static LOGS: Mutex<Vec<(u32, String)>> = Mutex::new(Vec::new());

unsafe extern "C" fn host_log(_ctx: *mut c_void, level: u32, msg: PwStr) {
    let text = String::from_utf8_lossy(unsafe { msg.as_bytes() }).into_owned();
    LOGS.lock().unwrap().push((level, text));
}

/// The SDK copies the table on first `create`, so a temporary is enough.
fn host() -> PwHostApi {
    PwHostApi { ctx: std::ptr::null_mut(), log: Some(host_log) }
}

fn decl() -> &'static PwPluginDecl {
    unsafe { &*plugwright_entry_v1() }
}

fn text(s: &PwStr) -> String {
    String::from_utf8(unsafe { s.as_bytes() }.to_vec()).unwrap()
}

/// Copy a plugin-owned buffer and hand it back to the plugin's allocator.
fn take(buf: PwBuf) -> String {
    let bytes =
        if buf.ptr.is_null() { Vec::new() } else { unsafe { std::slice::from_raw_parts(buf.ptr, buf.len) }.to_vec() };
    unsafe { decl().free_buf.unwrap()(buf) };
    String::from_utf8(bytes).unwrap()
}

fn create(config: &str) -> (u32, *mut c_void, String) {
    let mut inst: *mut c_void = std::ptr::null_mut();
    let mut err = PwBuf::EMPTY;
    let status = unsafe { decl().create.unwrap()(&host(), PwStr::from_bytes(config.as_bytes()), &mut inst, &mut err) };
    (status, inst, take(err))
}

fn call(inst: *mut c_void, method: &[u8], input: &[u8]) -> (u32, String) {
    let mut out = PwBuf::EMPTY;
    let status = unsafe { decl().call.unwrap()(inst, PwStr::from_bytes(method), PwStr::from_bytes(input), &mut out) };
    (status, take(out))
}

#[test]
fn declaration_table_describes_the_plugin() {
    let d = decl();
    assert_eq!(d.abi_version, ABI_VERSION);
    assert_eq!(d.struct_size as usize, size_of::<PwPluginDecl>());
    assert_eq!(text(&d.sdk_version), SDK_VERSION);
    assert_eq!(text(&d.name), env!("CARGO_PKG_NAME"));
    assert_eq!(text(&d.version), env!("CARGO_PKG_VERSION"));
    assert!(d.create.is_some() && d.call.is_some() && d.destroy.is_some() && d.free_buf.is_some());
}

#[test]
fn create_call_destroy_round_trip() {
    let (status, inst, err) = create("p:");
    assert_eq!((status, err.as_str()), (STATUS_OK, ""));
    assert!(!inst.is_null());
    assert_eq!(call(inst, b"tag", b"x"), (STATUS_OK, "p:x".to_owned()));
    let (status, msg) = call(inst, b"missing", b"");
    assert_eq!(status, STATUS_ERROR);
    assert!(msg.starts_with("unknown method `missing`"), "{msg}");
    unsafe { decl().destroy.unwrap()(inst) };
}

#[test]
fn constructor_errors_are_reported_without_an_instance() {
    let (status, inst, err) = create("bad");
    assert_eq!(status, STATUS_ERROR);
    assert_eq!(err, "refusing config `bad`");
    assert!(inst.is_null());
    let (status, _, err) = create("\u{0}\u{ff}");
    assert_eq!(status, STATUS_OK, "any UTF-8 is a valid &str config: {err}");
}

#[test]
fn panics_are_caught_and_the_instance_survives() {
    let (_, inst, _) = create("s:");
    let (status, msg) = call(inst, b"boom", b"");
    assert_eq!(status, STATUS_PANIC);
    assert_eq!(msg, "boom from the plugin");
    assert_eq!(call(inst, b"tag", b"after"), (STATUS_OK, "s:after".to_owned()));
    unsafe { decl().destroy.unwrap()(inst) };
}

#[test]
fn guest_logs_reach_the_host_callback() {
    let (_, inst, _) = create("");
    assert_eq!(call(inst, b"say", b"hello host").0, STATUS_OK);
    assert!(LOGS.lock().unwrap().contains(&(LOG_INFO, "hello host".to_owned())));
    unsafe { decl().destroy.unwrap()(inst) };
}

#[test]
fn hostile_arguments_are_errors_not_crashes() {
    let d = decl();
    let (_, inst, _) = create("");
    let (status, msg) = call(inst, &[0xff, 0xfe], b"");
    assert_eq!((status, msg.as_str()), (STATUS_ERROR, "method name is not UTF-8"));
    let (status, msg) = call(inst, b"tag", &[0xc3]);
    assert_eq!(status, STATUS_ERROR);
    assert!(msg.contains("must be UTF-8"), "{msg}");

    let (status, msg) = call(std::ptr::null_mut(), b"tag", b"");
    assert_eq!((status, msg.as_str()), (STATUS_ERROR, "null instance"));
    let mut err = PwBuf::EMPTY;
    let status = unsafe { d.create.unwrap()(&host(), PwStr::EMPTY, std::ptr::null_mut(), &mut err) };
    assert_eq!((status, take(err).as_str()), (STATUS_ERROR, "null out_instance"));
    // A null `out` only loses the payload; the status still tells the story.
    let status = unsafe { d.call.unwrap()(inst, PwStr::from_static("tag"), PwStr::EMPTY, std::ptr::null_mut()) };
    assert_eq!(status, STATUS_OK);
    unsafe {
        d.destroy.unwrap()(std::ptr::null_mut());
        d.free_buf.unwrap()(PwBuf::EMPTY);
        d.destroy.unwrap()(inst);
    }
}
