//! Isolated installer fixture; never part of the product binaries.
use std::{env, fs::{self, OpenOptions}, io::Write, ptr};
type Handle = *mut std::ffi::c_void;
#[link(name = "kernel32")]
unsafe extern "system" {
    fn OpenMutexW(access: u32, inherit: i32, name: *const u16) -> Handle;
    fn CreateMutexW(security: *const std::ffi::c_void, owner: i32, name: *const u16) -> Handle;
    fn CreateEventW(security: *const std::ffi::c_void, manual: i32, initial: i32, name: *const u16) -> Handle;
    fn WaitForSingleObject(handle: Handle, timeout: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}
fn wide(value: &str) -> Vec<u16> { value.encode_utf16().chain(Some(0)).collect() }
fn main() {
    let executable = env::current_exe().unwrap();
    let directory = executable.parent().unwrap();
    let namespace = env::var("NDM_TEST_INSTALLATION_NAMESPACE").expect("isolated namespace required");
    let guard = unsafe { OpenMutexW(1, 0, wide(&namespace).as_ptr()) };
    let state = if guard.is_null() { "launched" } else { unsafe { CloseHandle(guard); } "guard-held" };
    writeln!(OpenOptions::new().create(true).append(true).open(directory.join("launches.log")).unwrap(), "{state}").unwrap();
    fs::write(directory.join("fixture.pid"), std::process::id().to_string()).unwrap();
    if env::var_os("NDM_TEST_DESKTOP_WAIT").is_some() {
        let base = namespace.strip_suffix("-installing").unwrap();
        unsafe {
            let mutex = CreateMutexW(ptr::null(), 0, wide(&format!("{base}-mutex")).as_ptr());
            let event = if env::var_os("NDM_TEST_DESKTOP_LEGACY").is_some() { ptr::null_mut() } else {
                CreateEventW(ptr::null(), 0, 0, wide(&format!("{base}-shutdown")).as_ptr())
            };
            fs::write(directory.join("fixture.ready"), b"ready").unwrap();
            if event.is_null() { std::thread::sleep(std::time::Duration::from_secs(60)); }
            else {
                if WaitForSingleObject(event, 60_000) == 0 {
                    fs::write(directory.join("fixture.shutdown"), b"graceful").unwrap();
                    if env::var_os("NDM_TEST_DESKTOP_HANG").is_some() { std::thread::sleep(std::time::Duration::from_secs(60)); }
                }
                CloseHandle(event);
            }
            CloseHandle(mutex);
        }
    }
}
