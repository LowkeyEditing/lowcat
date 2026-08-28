use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::Threading::CreateMutexW,
        UI::WindowsAndMessaging::{FindWindowW, SW_RESTORE, SetForegroundWindow, ShowWindow},
    },
    core::w,
};

pub struct SingleInstance(HANDLE);

impl SingleInstance {
    pub fn acquire_or_activate() -> Option<Self> {
        let mutex =
            unsafe { CreateMutexW(None, false, w!("Local\\Lowcat.SingleInstance")) }.ok()?;
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe {
                if let Ok(window) = FindWindowW(None, w!("Lowcat")) {
                    let _ = ShowWindow(window, SW_RESTORE);
                    let _ = SetForegroundWindow(window);
                }
                let _ = CloseHandle(mutex);
            }
            None
        } else {
            Some(Self(mutex))
        }
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
