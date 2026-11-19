// 控制台输出解码：优先按 UTF-8 严格解码，失败时回退 GBK(936)，
// 兼容中文 Windows 下 netsh / ipconfig 等命令按 OEM 代码页输出的字节流。

#[cfg(windows)]
extern "system" {
    fn MultiByteToWideChar(
        CodePage: u32,
        dwFlags: u32,
        lpMultiByteStr: *const u8,
        cbMultiByte: i32,
        lpWideCharStr: *mut u16,
        cchWideChar: i32,
    ) -> i32;
}

#[cfg(windows)]
fn decode_gbk(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    unsafe {
        let len = MultiByteToWideChar(936, 0, bytes.as_ptr(), bytes.len() as i32, std::ptr::null_mut(), 0);
        if len <= 0 {
            return String::from_utf8_lossy(bytes).into_owned();
        }
        let mut buf = vec![0u16; len as usize];
        MultiByteToWideChar(
            936,
            0,
            bytes.as_ptr(),
            bytes.len() as i32,
            buf.as_mut_ptr(),
            len,
        );
        String::from_utf16_lossy(&buf)
    }
}

#[cfg(not(windows))]
fn decode_gbk(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// 解码子进程字节流：UTF-8 优先，否则按 GBK 回退。
pub fn decode_bytes(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => decode_gbk(bytes),
    }
}
