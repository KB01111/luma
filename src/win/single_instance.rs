use windows::Win32::{
    Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
    System::Threading::CreateMutexW,
};

pub enum InstanceGuard {
    Primary(PrimaryInstance),
    Secondary,
}

pub struct PrimaryInstance {
    handle: Option<HANDLE>,
}

impl Drop for PrimaryInstance {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                let _ = CloseHandle(handle);
            }
        }
    }
}

pub fn acquire_instance() -> Result<InstanceGuard, String> {
    if std::env::var_os("LUMA_ALLOW_MULTI").is_some() {
        return Ok(InstanceGuard::Primary(PrimaryInstance { handle: None }));
    }

    let handle = unsafe { CreateMutexW(None, false, windows::core::w!("Local\\Luma.Launcher.v1")) }
        .map_err(|error| {
            format!(
                "could not create the single-instance mutex: {}",
                error.message()
            )
        })?;

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            let _ = CloseHandle(handle);
        }
        return Ok(InstanceGuard::Secondary);
    }

    Ok(InstanceGuard::Primary(PrimaryInstance {
        handle: Some(handle),
    }))
}
