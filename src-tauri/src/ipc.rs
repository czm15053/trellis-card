use crate::session::HookEvent;
use std::fs;
#[cfg(unix)]
use std::io::{Read, Write};
use std::path::Path;

pub fn encode_event(event: &HookEvent) -> Result<String, serde_json::Error> {
    serde_json::to_string(event).map(|json| format!("{json}\n"))
}

pub fn decode_event(line: &str) -> Result<HookEvent, serde_json::Error> {
    serde_json::from_str(line)
}

fn enqueue(event: &HookEvent, queue: &Path) -> bool {
    let data = match encode_event(event) {
        Ok(data) => data,
        Err(error) => {
            eprintln!("[hook] encode failed: {error}");
            return false;
        }
    };
    if fs::create_dir_all(queue).is_err() {
        return false;
    }
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let name = format!("{stamp}-{}.json", std::process::id());
    let target = queue.join(name);
    let temp = target.with_extension("tmp");
    if fs::write(&temp, data).is_err() || fs::rename(&temp, &target).is_err() {
        let _ = fs::remove_file(&temp);
        return false;
    }
    true
}

// 发送 Hook 事件：优先走实时通道（unix socket / Windows Named Pipe），
// 失败时回退到 inbox 队列（由服务端 250ms 轮询兜底）。
pub fn send_event(event: &HookEvent, socket: &Path, queue: &Path) -> bool {
    let data = match encode_event(event) {
        Ok(data) => data,
        Err(error) => {
            eprintln!("[hook] encode failed: {error}");
            return false;
        }
    };
    #[cfg(unix)]
    {
        if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket) {
            if stream.write_all(data.as_bytes()).is_ok() {
                return true;
            }
        }
    }
    #[cfg(windows)]
    {
        if pipe_client_write(socket, data.as_bytes()) {
            return true;
        }
    }
    enqueue(event, queue)
}

// Windows Named Pipe 客户端：用 CreateFileW 打开管道写入。
// std 的 os::windows::net 在 stable 上没有稳定 Named Pipe 支持，直接用 Win32 API。
#[cfg(windows)]
fn pipe_client_write(pipe_path: &Path, data: &[u8]) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::WriteFile;
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_READ_DATA, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        FILE_WRITE_DATA, OPEN_EXISTING,
    };
    let wide: Vec<u16> = pipe_path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_DATA | FILE_WRITE_DATA,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return false;
    }
    let mut written: u32 = 0;
    let ok = unsafe {
        WriteFile(
            handle,
            data.as_ptr() as *const _,
            data.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        ) != 0
    };
    unsafe {
        CloseHandle(handle);
    }
    ok
}

pub fn drain_queue(dir: &Path) -> Vec<HookEvent> {
    let mut paths: Vec<_> = match fs::read_dir(dir) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .collect(),
        Err(_) => return Vec::new(),
    };
    paths.sort();
    let mut events = Vec::new();
    for path in paths {
        let result = fs::read_to_string(&path)
            .ok()
            .and_then(|data| decode_event(data.trim()).ok());
        if let Some(event) = result {
            events.push(event);
        } else {
            eprintln!("[hook] ignoring malformed queue entry {}", path.display());
        }
        let _ = fs::remove_file(path);
    }
    events
}

// ---- Unix socket 服务端 ----

#[cfg(unix)]
pub fn start_server<F>(path: &Path, on_event: F)
where
    F: Fn(HookEvent) + Send + 'static,
{
    use std::os::unix::net::UnixListener;
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::remove_file(&path);
        let listener = match UnixListener::bind(&path) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("[hook] socket bind failed {}: {error}", path.display());
                return;
            }
        };
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut data = String::new();
            if stream.read_to_string(&mut data).is_err() {
                continue;
            }
            for line in data.lines() {
                match decode_event(line) {
                    Ok(event) => on_event(event),
                    Err(error) => eprintln!("[hook] malformed socket event: {error}"),
                }
            }
        }
        let _ = fs::remove_file(path);
    });
}

// ---- Windows Named Pipe 服务端 ----
// Named Pipe 是 Windows 原生进程间通道，对应 unix socket 的角色。
// stable Rust 的 std 没有稳定 Named Pipe 支持，直接用 Win32 API：
//  CreateNamedPipeW → ConnectNamedPipe → ReadFile → 断开 → 重建。
// 每次连接后必须重新 CreateNamedPipeW 才能接下一个客户端。

#[cfg(windows)]
pub fn start_server<F>(path: &Path, on_event: F)
where
    F: Fn(HookEvent) + Send + 'static,
{
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{
        CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        ReadFile, FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX,
    };
    use windows_sys::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
        PIPE_TYPE_MESSAGE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    std::thread::spawn(move || {
        const BUF: usize = 4096;
        // 阻塞监听：每个连接在同一个线程内串行处理（Hook 事件频率低，串行足够），
        // 避免 HANDLE（*mut c_void）跨线程移动。
        loop {
            let handle = unsafe {
                CreateNamedPipeW(
                    wide.as_ptr(),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    BUF as u32,
                    BUF as u32,
                    0,
                    std::ptr::null_mut(),
                )
            };
            if handle == INVALID_HANDLE_VALUE {
                // 已有实例占用（应用已运行）：放弃，靠队列兜底
                eprintln!("[hook] named pipe create failed, fallback to queue");
                return;
            }
            unsafe {
                let connected = ConnectNamedPipe(handle, std::ptr::null_mut());
                // ERROR_PIPE_CONNECTED 表示客户端已在等待，视为成功
                let last_error = GetLastError();
                if connected == 0 && last_error != ERROR_PIPE_CONNECTED {
                    // 无客户端连接（超时或错误），重建
                    let _ = DisconnectNamedPipe(handle);
                    CloseHandle(handle);
                    continue;
                }
            }
            // 读该连接的全部数据（Hook 事件是一行 JSON，可能多次到达）
            let mut data_buf = Vec::with_capacity(BUF);
            let mut chunk = vec![0u8; BUF];
            loop {
                let mut read: u32 = 0;
                let ok = unsafe {
                    ReadFile(
                        handle,
                        chunk.as_mut_ptr() as *mut _,
                        BUF as u32,
                        &mut read,
                        std::ptr::null_mut(),
                    ) != 0
                };
                if !ok || read == 0 {
                    break;
                }
                data_buf.extend_from_slice(&chunk[..read as usize]);
            }
            let text = String::from_utf8_lossy(&data_buf);
            for line in text.lines() {
                match decode_event(line) {
                    Ok(event) => on_event(event),
                    Err(error) => eprintln!("[hook] malformed pipe event: {error}"),
                }
            }
            unsafe {
                let _ = DisconnectNamedPipe(handle);
                CloseHandle(handle);
            }
        }
    });
}

#[cfg(all(not(unix), not(windows)))]
pub fn start_server<F>(_path: &Path, _on_event: F)
where
    F: Fn(HookEvent) + Send + 'static,
{
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::HookEvent;

    #[test]
    fn queue_round_trip_preserves_event() {
        let root = std::env::temp_dir().join(format!("trellis-card-ipc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let socket = root.join("missing.sock");
        let queue = root.join("inbox");
        let event = HookEvent::working("s1", "/repo", Some("07-demo"), 10);
        assert!(send_event(&event, &socket, &queue));
        let events = drain_queue(&queue);
        assert_eq!(events, vec![event]);
        assert!(drain_queue(&queue).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn newline_encoding_has_one_json_event_per_line() {
        let event = HookEvent::stop("s1", "/repo", None, 20);
        let encoded = encode_event(&event).unwrap();
        assert!(encoded.ends_with('\n'));
        assert_eq!(encoded.lines().count(), 1);
        assert_eq!(decode_event(encoded.trim()).unwrap(), event);
    }
}
