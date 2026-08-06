// 文件系统监听：项目 .trellis/tasks/**/task.json 变化时向前端 emit "tasks-changed"。
// 与 Agent 活动采集（全局 Hook）分工：本模块只负责任务文件变化。
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tauri::Emitter;

const DEBOUNCE: Duration = Duration::from_millis(300);
const REDISCOVER: Duration = Duration::from_secs(60);
const EVENT: &str = "tasks-changed";
const RUNTIME_EVENT: &str = "runtime-reconciliation-needed";

pub fn is_relevant_task_event(path: &Path) -> bool {
    let comps: Vec<String> = path
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    let Some(index) = comps
        .windows(3)
        .position(|parts| parts[0] == ".trellis" && parts[1] == "tasks" && !parts[2].is_empty())
    else {
        return false;
    };
    let rest = &comps[index + 3..];
    if rest.len() == 1 {
        let file = rest[0].as_str();
        return matches!(
            file,
            "task.json"
                | "prd.md"
                | "design.md"
                | "implement.md"
                | "implement.jsonl"
                | "check.jsonl"
        ) || (file.starts_with("acceptance-report-") && file.ends_with(".md"));
    }
    rest.len() == 2 && rest[0] == "research" && rest[1].ends_with(".md")
}

// 当前所有 roots 下发现的项目的 .trellis 目录集合
fn discover_trellis_dirs() -> Vec<PathBuf> {
    let cfg = crate::config::load();
    crate::discover_all(&cfg)
        .iter()
        .map(|p| p.join(".trellis"))
        .filter(|p| p.is_dir())
        .collect()
}

// 对齐监听集合与发现结果；返回集合是否有增删
fn sync_watches(watcher: &mut RecommendedWatcher, watched: &mut Vec<PathBuf>) -> bool {
    let desired = discover_trellis_dirs();
    let mut changed = false;
    // 摘除已消失的项目
    watched.retain(|p| {
        if desired.contains(p) {
            true
        } else {
            let _ = watcher.unwatch(p);
            changed = true;
            false
        }
    });
    // 监听新出现的项目
    for dir in desired {
        if !watched.contains(&dir) {
            match watcher.watch(&dir, RecursiveMode::Recursive) {
                Ok(()) => {
                    watched.push(dir);
                    changed = true;
                }
                Err(e) => eprintln!("[watch] 监听失败 {}: {}", dir.display(), e),
            }
        }
    }
    changed
}

pub fn spawn_watcher(app: tauri::AppHandle) {
    std::thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let mut watcher = match RecommendedWatcher::new(
            move |res| {
                let _ = tx.send(res);
            },
            notify::Config::default(),
        ) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("[watch] 创建 watcher 失败: {}", e);
                return;
            }
        };
        let mut watched: Vec<PathBuf> = Vec::new();
        sync_watches(&mut watcher, &mut watched);
        println!("[watch] 已启动，监听 {} 个 .trellis 目录", watched.len());

        let mut dirty_at: Option<Instant> = None;
        let mut last_tick = Instant::now();
        loop {
            match rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Ok(event)) => {
                    if event.paths.iter().any(|p| is_relevant_task_event(p)) {
                        dirty_at = Some(Instant::now());
                    }
                }
                Ok(Err(e)) => eprintln!("[watch] 事件错误: {}", e),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
            // 去抖：300ms 无新事件后合并 emit 一次
            if let Some(t) = dirty_at {
                if t.elapsed() >= DEBOUNCE {
                    dirty_at = None;
                    let _ = app.emit(EVENT, ());
                    let _ = app.emit(RUNTIME_EVENT, ());
                }
            }
            // 每 60s 重新发现项目：增删时更新监听并通知前端（roots 变化靠这里收敛）
            if last_tick.elapsed() >= REDISCOVER {
                last_tick = Instant::now();
                if sync_watches(&mut watcher, &mut watched) {
                    println!(
                        "[watch] 项目集合变化（{} 个），emit tasks-changed",
                        watched.len()
                    );
                    let _ = app.emit(EVENT, ());
                    let _ = app.emit(RUNTIME_EVENT, ());
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_relevant_task_artifacts_and_rejects_unrelated_files() {
        assert!(is_relevant_task_event(Path::new(
            "/x/proj/.trellis/tasks/01-a/task.json"
        )));
        assert!(is_relevant_task_event(Path::new(
            "/x/proj/.trellis/tasks/01-a/prd.md"
        )));
        assert!(is_relevant_task_event(Path::new(
            "/x/proj/.trellis/tasks/01-a/research/source.md"
        )));
        assert!(is_relevant_task_event(Path::new(
            "/x/proj/.trellis/tasks/01-a/acceptance-report-final.md"
        )));
        assert!(!is_relevant_task_event(Path::new(
            "/x/proj/.trellis/journal/prd.md"
        )));
        assert!(!is_relevant_task_event(Path::new(
            "/x/proj/.trellis/tasks/01-a/notes.txt"
        )));
        assert!(!is_relevant_task_event(Path::new(
            "/x/proj/.trellis/journal/task.json"
        )));
        assert!(!is_relevant_task_event(Path::new(
            "/x/proj/.trellis/tasks/01-a"
        )));
        assert!(!is_relevant_task_event(Path::new("/x/proj/task.json")));
    }
}
