use std::ffi::CStr;
use std::os::raw::c_char;

unsafe extern "C" {
    fn beebeeb_fp_status(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_visible_url(
        url_buffer: *mut c_char,
        url_buffer_len: usize,
        error_buffer: *mut c_char,
        error_buffer_len: usize,
    ) -> i32;
    fn beebeeb_fp_install(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
    fn beebeeb_fp_remove(error_buffer: *mut c_char, error_buffer_len: usize) -> i32;
}

pub fn status() -> Result<bool, String> {
    Ok(call_bridge(beebeeb_fp_status)? == 1)
}

pub fn visible_url() -> Result<Option<String>, String> {
    let mut url_buffer = [0i8; 2048];
    let mut error_buffer = [0i8; 1024];
    let code = unsafe {
        beebeeb_fp_visible_url(
            url_buffer.as_mut_ptr(),
            url_buffer.len(),
            error_buffer.as_mut_ptr(),
            error_buffer.len(),
        )
    };
    if code < 0 {
        return Err(
            buffer_to_string(&error_buffer).unwrap_or_else(|| "Resolve File Provider location failed".to_string())
        );
    }
    if code == 0 {
        return Ok(None);
    }
    Ok(buffer_to_string(&url_buffer))
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

fn buffer_to_string(buffer: &[i8]) -> Option<String> {
    let message = unsafe { CStr::from_ptr(buffer.as_ptr()) }
        .to_string_lossy()
        .trim()
        .to_string();
    if message.is_empty() { None } else { Some(message) }
}
