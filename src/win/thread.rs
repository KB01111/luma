use windows::Win32::System::Threading::{
    GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
};

/// Keep indexing and shell-scanning work below the UI thread's priority so
/// the launcher can paint and respond while background data loads.
pub fn lower_current_thread_priority() {
    unsafe {
        let _ = SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}
