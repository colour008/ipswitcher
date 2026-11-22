use std::{
	ptr,
	thread,
	time::Duration,
};

use windows_sys::Win32::{
	Foundation::{
		CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_ABANDONED, WAIT_FAILED,
		WAIT_OBJECT_0,
	},
	System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject},
	UI::WindowsAndMessaging::{
		FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
	},
};

const INSTANCE_MUTEX_NAME: &str = "Local\\com_jamhoo_ipswitcher_single_instance";
const MAIN_WINDOW_TITLE: &str = "IPSwitcher";
const ELEVATED_RESTART_WAIT_MS: u32 = 30_000;
const WINDOW_LOOKUP_ATTEMPTS: usize = 40;

pub enum AcquireError {
	AlreadyRunning,
	System(String),
}

pub struct InstanceGuard {
	handle: HANDLE,
}

impl Drop for InstanceGuard {
	fn drop(&mut self) {
	unsafe {
		let _ = ReleaseMutex(self.handle);
		let _ = CloseHandle(self.handle);
	}
}
}

/// 获取当前 Windows 会话内的单实例互斥锁。
/// 提权重启时等待旧进程释放锁，再由新进程接管；普通二次启动则返回 AlreadyRunning。
pub fn acquire(wait_for_previous: bool) -> Result<InstanceGuard, AcquireError> {
	let mutex_name: Vec<u16> = INSTANCE_MUTEX_NAME.encode_utf16().chain(Some(0)).collect();
	let handle = unsafe { CreateMutexW(ptr::null(), 1, mutex_name.as_ptr()) };
	if handle.is_null() {
		return Err(AcquireError::System(format!("CreateMutexW 错误码 {}", unsafe {
			GetLastError()
		})));
	}

	if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
		return Ok(InstanceGuard { handle });
	}

	if !wait_for_previous {
		unsafe {
			let _ = CloseHandle(handle);
		}
		return Err(AcquireError::AlreadyRunning);
	}

	match unsafe { WaitForSingleObject(handle, ELEVATED_RESTART_WAIT_MS) } {
		WAIT_OBJECT_0 | WAIT_ABANDONED => Ok(InstanceGuard { handle }),
		WAIT_FAILED => {
			let error = unsafe { GetLastError() };
			unsafe {
				let _ = CloseHandle(handle);
			}
			Err(AcquireError::System(format!("等待旧进程退出失败，错误码 {error}")))
		}
		_ => {
			unsafe {
				let _ = CloseHandle(handle);
			}
			Err(AcquireError::System("等待旧进程退出超时".to_string()))
		}
	}
}

/// 让已运行的主窗口显示并置前；隐藏到托盘和最小化状态都可以恢复。
pub fn focus_existing_main_window() {
	let title: Vec<u16> = MAIN_WINDOW_TITLE.encode_utf16().chain(Some(0)).collect();
	for _ in 0..WINDOW_LOOKUP_ATTEMPTS {
		let window = unsafe { FindWindowW(ptr::null(), title.as_ptr()) };
		if !window.is_null() {
			unsafe {
				if IsIconic(window) != 0 {
					let _ = ShowWindow(window, SW_RESTORE);
				} else {
					let _ = ShowWindow(window, SW_SHOW);
				}
				let _ = SetForegroundWindow(window);
			}
			return;
		}
		thread::sleep(Duration::from_millis(50));
	}
}
