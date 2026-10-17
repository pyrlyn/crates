//! Exports the drift tests generate bindings from. Read by cbindgen and
//! csbindgen as text; never compiled.

/// A point in the fixture's ABI.
#[repr(C)]
pub struct FfiPoint {
    pub x: i32,
    pub y: i32,
}

/// Adds two numbers.
#[no_mangle]
pub extern "C" fn ffi_add(a: i32, b: i32) -> i32 {
    a + b
}

/// The origin.
#[no_mangle]
pub extern "C" fn ffi_origin() -> FfiPoint {
    FfiPoint { x: 0, y: 0 }
}
