/// 子进程创建辅助：在 Windows 下隐藏控制台子进程弹出的 CMD 黑窗。
use std::process::Command;

/// 为命令附加 `CREATE_NO_WINDOW` 标志，避免调起 netsh / powershell / 诊断命令时闪控制台窗口。
pub fn suppress_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
