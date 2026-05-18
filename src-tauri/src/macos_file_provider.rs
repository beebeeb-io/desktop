use std::ffi::CStr;
use std::os::raw::c_char;

unsafe extern "C" {
    fn beebeeb_fp_status(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_install(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_remove(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
}

pub fn status() -> Result<bool, String> {
    Ok(call_bridge(beebeeb_fp_status)? == 1)
}

pub fn install() -> Result<(), String> {
    call_bridge(beebeeb_fp_install).map(|_| ())
}

#[allow(dead_code)]
pub fn remove() -> Result<(), String> {
    call_bridge(beebeeb_fp_remove).map(|_| ())
}

fn call_bridge(function: unsafe extern "C" fn(*mut c_char, usize) -> i32) -> Result<i32, String> {
    let mut error_buffer = [0i8; 1024];
    let code = unsafe { function(error_buffer.as_mut_ptr(), error_buffer.len()) };
    if code >= 0 {
        return Ok(code);
    }
    let message = unsafe { CStr::from_ptr(error_buffer.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    Err(if message.is_empty() {
        "File Provider operation failed".to_string()
    } else {
        message
    })
}
