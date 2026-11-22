use std::{
	ptr,
	thread,
	time::Duration,
};

use windows_sys::core::BOOL;
use windows_sys::Win32::{
	Foundation::{
        CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, HWND, LPARAM, WAIT_ABANDONED,
        WAIT_FAILED, WAIT_OBJECT_0,
	},
    System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, ReleaseMutex, SetEvent, WaitForSingleObject,
        EVENT_MODIFY_STATE, INFINITE,
    },
	UI::WindowsAndMessaging::{
        BringWindowToTop, EnumWindows, FindWindowW, GetWindowTextW, IsIconic,
        SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
	},
};

const INSTANCE_MUTEX_NAME: &str = "Local\\com_jamhoo_ipswitcher_single_instance";
const ACTIVATE_EVENT_NAME: &str = "Local\\com_jamhoo_ipswitcher_activate";
const MAIN_WINDOW_TITLE: &str = "IPSwitcher";
const ELEVATED_RESTART_WAIT_MS: u32 = 30_000;
const WINDOW_LOOKUP_ATTEMPTS: usize = 40;

struct WindowSearch {
    exact: HWND,
    fallback: HWND,
}

unsafe extern "system" fn find_main_window(window: HWND, data: LPARAM) -> BOOL {
    let search = &mut *(data as *mut WindowSearch);
    let mut title = [0u16; 256];
    let len = GetWindowTextW(window, title.as_mut_ptr(), title.len() as i32);
    if len <= 0 {
        return 1;
    }
    let title = String::from_utf16_lossy(&title[..len as usize]);
    if title == MAIN_WINDOW_TITLE {
        search.exact = window;
        return 0;
    }
    if search.fallback.is_null()
        && title.contains(MAIN_WINDOW_TITLE)
        && !title.contains("挂件")
    {
        search.fallback = window;
    }
    1
}

fn locate_main_window() -> HWND {
    let title: Vec<u16> = MAIN_WINDOW_TITLE.encode_utf16().chain(Some(0)).collect();
    let exact = unsafe { FindWindowW(ptr::null(), title.as_ptr()) };
    if !exact.is_null() {
        return exact;
    }

    let mut search = WindowSearch {
        exact: ptr::null_mut(),
        fallback: ptr::null_mut(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(find_main_window),
            &mut search as *mut WindowSearch as LPARAM,
        );
    }
    if !search.exact.is_null() {
        search.exact
    } else {
        search.fallback
    }
}

/// 主实例监听一个跨完整性级别可访问的 Windows 事件，由主进程自己恢复并聚焦窗口。
/// 这样从普通权限启动器打开已提权的实例时，也不依赖低权限进程直接控制高权限窗口。
pub fn start_activation_listener(app: tauri::AppHandle) {
    let name: Vec<u16> = ACTIVATE_EVENT_NAME.encode_utf16().chain(Some(0)).collect();
    let event = unsafe { CreateEventW(ptr::null(), 0, 0, name.as_ptr()) };
    if event.is_null() {
        return;
    }
    let event = event as usize;

    thread::spawn(move || loop {
        match unsafe { WaitForSingleObject(event as HANDLE, INFINITE) } {
            WAIT_OBJECT_0 => super::show_main_window(&app),
            WAIT_FAILED => break,
            _ => {}
        }
    });
}

fn signal_existing_instance() -> bool {
    let name: Vec<u16> = ACTIVATE_EVENT_NAME.encode_utf16().chain(Some(0)).collect();
    for _ in 0..WINDOW_LOOKUP_ATTEMPTS {
        let event = unsafe { OpenEventW(EVENT_MODIFY_STATE, 0, name.as_ptr()) };
        if !event.is_null() {
            let signaled = unsafe { SetEvent(event) } != 0;
            unsafe {
                let _ = CloseHandle(event);
            }
            return signaled;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

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
    if signal_existing_instance() {
        return;
    }
	for _ in 0..WINDOW_LOOKUP_ATTEMPTS {
        let window = locate_main_window();
		if !window.is_null() {
			unsafe {
				if IsIconic(window) != 0 {
					let _ = ShowWindow(window, SW_RESTORE);
				} else {
					let _ = ShowWindow(window, SW_SHOW);
				}
                let _ = BringWindowToTop(window);
				let _ = SetForegroundWindow(window);
			}
			return;
		}
		thread::sleep(Duration::from_millis(50));
	}
}
