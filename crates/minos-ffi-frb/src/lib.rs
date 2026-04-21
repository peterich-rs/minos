//! frb surface for Dart. Plan 03 fills in real exports. This file exists only
//! so the workspace compiles and the crate name is reserved.

#[allow(dead_code)]
fn _link_minos_mobile() {
    // Keep the `minos-mobile` dep used so cargo doesn't drop it from compile.
    let _ = std::any::type_name::<minos_mobile::MobileClient>();
}

#[no_mangle]
pub extern "C" fn minos_ffi_frb_ping() -> i32 {
    42
}
