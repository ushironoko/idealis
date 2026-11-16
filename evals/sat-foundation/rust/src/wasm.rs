use std::cell::RefCell;
thread_local! { static BUFFERS: RefCell<(Vec<u8>, Vec<u8>)> = const { RefCell::new((Vec::new(), Vec::new())) }; }
#[unsafe(no_mangle)]
pub extern "C" fn sat_abi_version() -> u32 {
    1
}
#[unsafe(no_mangle)]
pub extern "C" fn sat_input_resize(length: u32) -> *mut u8 {
    BUFFERS.with_borrow_mut(|buffers| {
        buffers.0.resize(length as usize, 0);
        buffers.0.as_mut_ptr()
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn sat_dispatch() {
    BUFFERS.with_borrow_mut(|buffers| {
        buffers.1 = serde_json::to_vec(&super::dispatch(&buffers.0)).expect("serializable result");
    })
}
#[unsafe(no_mangle)]
pub extern "C" fn sat_output_ptr() -> *const u8 {
    BUFFERS.with_borrow(|buffers| buffers.1.as_ptr())
}
#[unsafe(no_mangle)]
pub extern "C" fn sat_output_len() -> u32 {
    BUFFERS.with_borrow(|buffers| buffers.1.len() as u32)
}
