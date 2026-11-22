#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(target_os = "windows")]
fn main() {
	let args: Vec<String> = std::env::args().collect();

	// 提权执行操作的临时进程必须绕过主程序互斥锁。
	if args.iter().any(|arg| arg == "--elevate") {
		ipswitcher_lib::run();
		return;
	}

	let wait_for_previous = args.iter().any(|arg| arg == "--elevated-restart");
	match ipswitcher_lib::single_instance::acquire(wait_for_previous) {
		Ok(_instance_guard) => ipswitcher_lib::run(),
		Err(ipswitcher_lib::single_instance::AcquireError::AlreadyRunning) => {
			ipswitcher_lib::single_instance::focus_existing_main_window();
		}
		Err(ipswitcher_lib::single_instance::AcquireError::System(error)) => {
			eprintln!("无法检查 IPSwitcher 是否已运行：{error}");
		}
	}
}

#[cfg(not(target_os = "windows"))]
fn main() {
    ipswitcher_lib::run()
}
