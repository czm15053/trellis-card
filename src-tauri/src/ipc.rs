use crate::session::HookEvent;
use std::fs;
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
    enqueue(event, queue)
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

#[cfg(not(unix))]
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
