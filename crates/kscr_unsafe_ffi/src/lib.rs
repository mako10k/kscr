use std::ffi::CString;
use std::os::raw::{c_char, c_int};

extern "C" {
    fn puts(s: *const c_char) -> c_int;
}

pub fn puts_checked(s: &str) -> Result<i32, &'static str> {
    let cs = CString::new(s).map_err(|_| "string contains NUL")?;
    // SAFETY: `cs` is NUL-terminated and lives for the duration of the call.
    let rc = unsafe { puts(cs.as_ptr()) };
    Ok(rc as i32)
}
