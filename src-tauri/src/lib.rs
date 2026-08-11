mod activity;
mod config;
mod focus;
mod hook_cli;
mod hook_install;
mod ipc;
mod platform;
mod progress;
mod runtime;
mod scan;
mod session;
mod watch;

use config::AppConfig;
use runtime::{ActionRecord, AgentRuntime, TaskRuntimeView, TrellisAction};
use serde::Serialize;
use session::{HookEvent, SessionRegistry};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

pub struct AppState {
    config: Mutex<AppConfig>,
    runtime: Mutex<RuntimeStore>,
    /* runtime 快照 build/flush 协调器：全触发源共用，串行 + debounce */
    runtime_coord: RuntimeCoordinator,
}

#[derive(Default)]
struct RuntimeStore {
    sessions: SessionRegistry,
    actions: HashMap<String, ActionRecord>,
    focus_key: Option<String>,
    /* 完成迁移生命周期：上次扫描到的真实 task status（key = '项目::任务id'），
    canonical 事实源，只记录任务真实状态，不伪造。 */
    task_statuses: HashMap<String, String>,
    /* 待 emit 的完成迁移事件（仅真实 unfinished -> completed/done 一次） */
    pending_completions: Vec<TaskCompletedEvent>,
    /* 上次已 emit 的快照指纹：无变化时不重复 emit agent-state-changed，
    避免 10s reconcile 等定时触发造成无谓的 IPC + 前端 render。 */
    last_emitted_fingerprint: Option<String>,
    /* 最近一次 coordinator flush 构建的快照缓存：get_runtime_snapshot 读它而非直接
    build，避免与 coordinator 并发扫描 / 旧快照覆盖新状态。首次（缓存空）由调用方直建。 */
    last_snapshot: Option<RuntimeSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskCompletedEvent {
    project: String,
    task_id: String,
    completed_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeSnapshot {
    pub tasks: Vec<TaskRuntimeView>,
    pub project_activities: Vec<AgentRuntime>,
    pub errors: Vec<String>,
    pub focus_key: Option<String>,
    pub generated_at: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HookTasksChangedEvent {
    dynamic_project_added: bool,
    action: Option<TrellisAction>,
    project: Option<String>,
}

// ---------- 输出结构 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ConfigOut {
    roots: Vec<String>,
    always_on_top: bool,
    configured: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RootsOut {
    roots: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TasksPayload {
    version: String,
    tasks: Vec<scan::Task>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArchivedPayload {
    /* archive 独立 version（archive 内最大 mtime），不参与活跃任务的 version 比对 */
    version: String,
    tasks: Vec<scan::Task>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DocOut {
    name: String,  // 文件名（research 内为 research/xx.md）
    label: String, // 展示标签：PRD / DESIGN / IMPLEMENT / 报告 N / 调研·xx / 文件名
    content: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskDetail {
    #[serde(flatten)]
    task: scan::Task,
    docs: Vec<DocOut>,
}

// ---------- 任务关联视图 ----------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TaskRelation {
    id: String,
    dir: String,
    title: String,
    status: String,
    phase: scan::Phase,
    priority: Option<String>,
    dev_type: Option<String>,
    scope: Option<String>,
    parent: Option<String>,
    children: Vec<String>,
    spec_refs: Vec<String>,
    file_refs: Vec<String>,
    prd_refs: Vec<String>,
    subtasks: Vec<scan::Subtask>,
    sessions: Vec<scan::SessionInfo>,
    progress: f64,
    kind: String,
    archived: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecGroup {
    path: String,
    count: usize,
    task_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrdGroup {
    // 被引用的任务 id（官方 FAQ Q25 跨任务内容引用）
    ref_task_id: String,
    // 引用它的任务 id 列表
    task_ids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RelationsPayload {
    tasks: Vec<TaskRelation>,
    spec_groups: Vec<SpecGroup>,
    prd_groups: Vec<PrdGroup>,
}

// 项目规范地图条目：规范路径、显示名、分类、填写状态、被引用次数、正文
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SpecOut {
    path: String,
    name: String,
    category: String,
    filled: bool,
    line_count: usize,
    referenced_by: usize,
    content: String,
}

// ---------- 辅助 ----------

fn expand(p: &str) -> PathBuf {
    config::expand_home(p)
}

pub(crate) fn discover_all(cfg: &AppConfig) -> Vec<PathBuf> {
    let mut out = discover_all_roots(cfg);
    let mut seen: std::collections::HashSet<PathBuf> = out.iter().cloned().collect();
    for project in &cfg.dynamic_projects {
        let dir = expand(project);
        // 路径比较用大小写不敏感（Windows 文件系统）；去重用 os 原生 PathBuf
        let dup = seen.iter().any(|seen_path| {
            crate::platform::path_eq_ignore_case(
                &seen_path.to_string_lossy(),
                &dir.to_string_lossy(),
            )
        });
        if dir.join(".trellis").join("tasks").is_dir() && !dup {
            seen.insert(dir.clone());
            out.push(dir);
        }
    }
    out
}

fn project_dir_for_path(path: &Path) -> Option<PathBuf> {
    let mut current = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    loop {
        if current.join(".trellis").join("tasks").is_dir() {
            return Some(platform::normalize_path(&current));
        }
        if !current.pop() {
            return None;
        }
    }
}

/* Hook 发现的项目不受 roots 限制，但仍必须是真正的 Trellis 项目目录。 */
fn register_dynamic_project(state: &AppState, path: &str) -> Option<(PathBuf, bool)> {
    let project = project_dir_for_path(Path::new(path))?;
    let mut cfg = state.config.lock().unwrap();
    /* A root can itself be a Trellis project. In that case the scanner
    intentionally stops descending, so a child project must still be
    registered when a hook proves that it is active. */
    if is_discovered_project(&cfg, &project) {
        return Some((project, false));
    }
    if cfg
        .dynamic_projects
        .iter()
        .map(|stored| expand(stored))
        .any(|stored| {
            crate::platform::path_eq_ignore_case(
                &stored.to_string_lossy(),
                &project.to_string_lossy(),
            )
        })
    {
        return Some((project, false));
    }
    let stored = platform::strip_device_prefix(&project.to_string_lossy());
    cfg.dynamic_projects.push(stored);
    let snapshot = cfg.clone();
    drop(cfg);
    if let Err(error) = config::save(&snapshot) {
        eprintln!("[projects] 保存动态项目失败: {error}");
    }
    Some((project, true))
}

fn discover_all_roots(cfg: &AppConfig) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for root in &cfg.roots {
        for dir in scan::discover_projects(&expand(root), 3) {
            if seen.insert(dir.clone()) {
                out.push(dir);
            }
        }
    }
    out
}

fn is_discovered_project(cfg: &AppConfig, project: &Path) -> bool {
    discover_all_roots(cfg).iter().any(|found| {
        crate::platform::path_eq_ignore_case(&found.to_string_lossy(), &project.to_string_lossy())
    })
}

// 允许访问 roots 扫描到的项目，以及 Hook 已发现的动态项目。
fn is_allowed_project(state: &AppState, project: &str) -> bool {
    let cfg = state.config.lock().unwrap();
    let target = expand(project);
    discover_all(&cfg).contains(&target)
}

fn save_state(state: &AppState) -> Result<(), String> {
    let cfg = state.config.lock().unwrap().clone();
    config::save(&cfg)
}

fn now_seconds() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or_default()
}

// 按平台候选探测可用的 Python 命令（对齐上游 init.ts 的 PYTHON_CANDIDATES）。
// 返回可执行的命令；全部探测失败时退回第一个候选（由调用方拿到 Command 错误）。
fn resolve_python_command() -> &'static str {
    for candidate in platform::python_candidates() {
        if candidate == "py -3" {
            // py 启动器需带参数探测，这里仅检查命令存在
            if std::process::Command::new("py")
                .arg("-3")
                .arg("--version")
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
            {
                return candidate;
            }
            continue;
        }
        if std::process::Command::new(candidate)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
        {
            return candidate;
        }
    }
    // 全部失败：退回平台默认，让 Command 的 spawn 错误自然暴露给调用方
    platform::python_candidates()[0]
}

fn task_key(project: &str, task_id: &str) -> String {
    format!("{project}::{task_id}")
}

fn is_done_status(status: &str) -> bool {
    matches!(status, "completed" | "done")
}

/* 完成迁移判定：仅「上次已知状态存在且非完成、本次为完成」的真实迁移返回 true。
初次扫描（prev=None）、无变化、completed->completed、blocked->blocked 均不迁移。 */
fn is_completion_transition(prev: Option<&str>, cur: &str) -> bool {
    match prev {
        Some(prev) => !is_done_status(prev) && is_done_status(cur),
        None => false,
    }
}

fn build_runtime_snapshot(state: &AppState) -> RuntimeSnapshot {
    let now = now_seconds();
    let cfg = state.config.lock().unwrap().clone();
    let projects = discover_all(&cfg);
    let mut runtime = state.runtime.lock().unwrap();
    runtime.sessions.prune(now);
    let mut views = Vec::new();
    let mut activities = Vec::new();
    let mut errors = Vec::new();
    for project_dir in projects {
        let project = platform::strip_device_prefix(&project_dir.to_string_lossy());
        let (tasks, task_errors) = scan::scan_tasks(&project_dir);
        errors.extend(task_errors);
        for task in tasks {
            let key = task_key(&project, &task.id);
            /* 完成迁移 reducer：仅 unfinished -> completed/done 的真实迁移记入 pending。
            初次扫描（无上次记录）、无变化、重复 completed 均不生成。 */
            let prev_status = runtime.task_statuses.get(&key).cloned();
            if is_completion_transition(prev_status.as_deref(), &task.status) {
                runtime.pending_completions.push(TaskCompletedEvent {
                    project: project.clone(),
                    task_id: task.id.clone(),
                    completed_at: task
                        .completed_at
                        .as_deref()
                        .and_then(|s| s.parse::<i64>().ok())
                        .unwrap_or(now),
                });
            }
            runtime.task_statuses.insert(key, task.status.clone());
            let session = runtime
                .sessions
                .latest_for_task(&project, &task.id)
                .or_else(|| runtime.sessions.latest_for_task(&project, &task.dir));
            let action = runtime
                .actions
                .get(&task_key(&project, &task.id))
                .and_then(|r| r.is_fresh(now).then_some(r.action))
                .or_else(|| {
                    runtime
                        .actions
                        .get(&task_key(&project, &task.dir))
                        .and_then(|r| r.is_fresh(now).then_some(r.action))
                });
            let mut view = focus::fuse_task(&task, session, action);
            view.project = project.clone();
            views.push(view);
        }
    }
    activities.extend(runtime.sessions.all_activity());
    let previous = runtime.focus_key.as_deref();
    let next_focus = focus::choose_focus(&views, previous, now);
    runtime.focus_key = next_focus.clone();
    RuntimeSnapshot {
        tasks: views,
        project_activities: activities,
        errors,
        focus_key: next_focus,
        generated_at: now,
    }
}

/* 快照语义指纹：反映「前端可见状态」是否变化。覆盖所有实际展示字段——
任务级（phase/displayState/attention/confidence/action/activity/lastChangedAt/
agent 状态与工具）+ 项目级活动 + 焦点。任务集/项目变化会改变指纹（key 含 project）。
缺字段会导致该字段变化时前端不重渲染（活动/阶段展示卡住）。 */
fn snapshot_fingerprint(snapshot: &RuntimeSnapshot) -> String {
    let mut tasks: Vec<String> = snapshot
        .tasks
        .iter()
        .map(|v| {
            let agent = v.agent.as_ref();
            let tool_input = agent
                .and_then(|a| a.tool_input.as_ref())
                .and_then(|t| serde_json::to_string(t).ok())
                .unwrap_or_default();
            format!(
                "{}:{}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{:?}:{}:{}:{}:{:?}:{:?}:{}",
                v.project,
                v.task_id.as_deref().unwrap_or(""),
                v.phase,
                v.display_state,
                v.attention,
                v.confidence,
                v.action,
                agent.map(|a| &a.state),
                v.activity.as_deref().unwrap_or(""),
                v.last_changed_at,
                agent.map(|a| a.agent_kind.as_str()).unwrap_or(""),
                agent
                    .map(|a| a.tool_name.as_deref().unwrap_or(""))
                    .unwrap_or(""),
                agent.map(|a| a.event_name.as_str()).unwrap_or(""),
                agent.map(|a| &a.waiting_reason),
                agent
                    .and_then(|a| a.tool_input.as_ref())
                    .map(|_| tool_input.as_str()),
                agent.map(|a| a.updated_at).unwrap_or(0)
            )
        })
        .collect();
    tasks.sort();
    let mut activities: Vec<String> = snapshot
        .project_activities
        .iter()
        .map(|a| {
            let tool_input = a
                .tool_input
                .as_ref()
                .and_then(|t| serde_json::to_string(t).ok())
                .unwrap_or_default();
            format!(
                "{}:{}:{:?}:{}:{}:{}:{}:{}:{:?}:{:?}",
                a.project,
                a.session_id,
                a.state,
                a.agent_kind.as_str(),
                a.tool_name.as_deref().unwrap_or(""),
                a.activity.as_deref().unwrap_or(""),
                a.event_name.as_str(),
                a.updated_at,
                a.waiting_reason,
                a.tool_input.as_ref().map(|_| tool_input.as_str())
            )
        })
        .collect();
    /* project_activities 来自 HashMap，顺序不稳定；排序保证指纹在无真实变化时稳定。 */
    activities.sort();
    format!(
        "{}|{:?}|{:?}",
        snapshot.focus_key.as_deref().unwrap_or(""),
        tasks,
        activities
    )
}

/* 单一 flush 路径：build(reduce) + 条件 emit snapshot/completions。
reducer 副作用（task_statuses / pending_completions 写入）在 build_runtime_snapshot 内完成；
此处只负责按序 emit。顺序不变量：focus-task-changed → agent-state-changed → task-completed。
所有 flush 必须经 RuntimeCoordinator 串行，避免并发 build 交错扫描 / 快照顺序反转。
「无变化不重渲染」：指纹未变且无 pending 时跳过 emit，避免 10s reconcile 等定时触发
造成无谓的 IPC + 前端 render。pending_completions 非空时必须 emit（completion 事件是
前端唯一一次性触发入口，不可因「快照未变」被跳过）。 */
fn flush_snapshot(app: &AppHandle) {
    let state = app.state::<AppState>();
    let previous = state.runtime.lock().unwrap().focus_key.clone();
    let snapshot = build_runtime_snapshot(&state);
    /* 缓存最近构建的快照：get_runtime_snapshot 读缓存而非直接 build，避免并发扫描 */
    state.runtime.lock().unwrap().last_snapshot = Some(snapshot.clone());
    let fp = snapshot_fingerprint(&snapshot);
    let (completions, unchanged) = {
        let mut runtime = state.runtime.lock().unwrap();
        let unchanged = runtime.last_emitted_fingerprint.as_deref() == Some(fp.as_str())
            && runtime.pending_completions.is_empty();
        (std::mem::take(&mut runtime.pending_completions), unchanged)
    };
    if unchanged {
        return;
    }
    if previous != snapshot.focus_key {
        let _ = app.emit("focus-task-changed", &snapshot.focus_key);
    }
    let _ = app.emit("agent-state-changed", &snapshot);
    state.runtime.lock().unwrap().last_emitted_fingerprint = Some(fp);
    for completed in completions {
        let _ = app.emit("task-completed", &completed);
    }
}

/* RuntimeCoordinator：runtime 快照 build/flush 的唯一入口，trailing debounce + 串行。
- 触发源（hook 事件、10s reconcile、get_runtime_snapshot）都调用 request()，经
  channel 信号唤醒调度线程；
- 调度线程对信号做 trailing debounce（窗口内新信号重置计时），窗口结束执行一次
  flush_snapshot；单线程串行保证并发触发不会交错扫描或颠倒快照顺序。 */
struct RuntimeCoordinator {
    tx: std::sync::mpsc::Sender<()>,
}

const RUNTIME_DEBOUNCE_MS: u64 = 250;

impl RuntimeCoordinator {
    /* 仅建 channel、不 spawn 线程：供测试无 AppHandle 时构造。 */
    #[cfg(test)]
    fn new() -> Self {
        let (tx, _rx) = std::sync::mpsc::channel::<()>();
        RuntimeCoordinator { tx }
    }

    /* 启动调度线程（生产路径）：trailing debounce + 单线程串行。 */
    fn spawn(app: AppHandle) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let coord = RuntimeCoordinator { tx };
        std::thread::spawn(move || {
            loop {
                /* 等待至少一个触发信号；上游全部断开则退出。 */
                if rx.recv().is_err() {
                    return;
                }
                /* trailing debounce：继续收，直到 debounce 窗口内无新信号。 */
                loop {
                    match rx.recv_timeout(std::time::Duration::from_millis(RUNTIME_DEBOUNCE_MS)) {
                        Ok(()) => continue,
                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => break,
                        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                flush_snapshot(&app);
            }
        });
        coord
    }

    fn request(&self) {
        /* 发送失败（线程已退出）时静默降级：flush 不可用比阻塞更安全。 */
        let _ = self.tx.send(());
    }
}

fn apply_hook_events<I>(app: &AppHandle, events: I)
where
    I: IntoIterator<Item = HookEvent>,
{
    let state = app.state::<AppState>();
    let mut dynamic_project_added = false;
    let mut refresh_action = None;
    let mut project_hint = None;
    let mut prepared_events = Vec::new();
    for mut event in events {
        /* 全局 Hook 会收到所有 Agent 项目的事件，只接收真正的 Trellis 项目。 */
        let Some((project, added)) = register_dynamic_project(&state, &event.project) else {
            continue;
        };
        event.project = platform::strip_device_prefix(&project.to_string_lossy());
        dynamic_project_added |= added;
        if event.action == Some(TrellisAction::Create) || project_hint.is_none() {
            project_hint = Some(event.project.clone());
        }
        if event.action == Some(TrellisAction::Create) || refresh_action.is_none() {
            refresh_action = event.action;
        }
        prepared_events.push(event);
    }
    if prepared_events.is_empty() {
        return;
    }
    {
        let mut runtime = state.runtime.lock().unwrap();
        for event in prepared_events {
            let project = event.project.clone();
            let session_id = event.session_id.clone();
            let event_task_id = event.task_id.clone();
            let action = event.action;
            let timestamp = event.timestamp;
            if !runtime.sessions.apply(event) {
                continue;
            }
            let task_id = event_task_id.or_else(|| {
                runtime
                    .sessions
                    .task_id_for_session(&session_id)
                    .map(str::to_owned)
            });
            if let (Some(task_id), Some(action)) = (task_id.as_deref(), action) {
                let key = task_key(&project, task_id);
                let should_update = runtime
                    .actions
                    .get(&key)
                    .map(|record| timestamp >= record.updated_at)
                    .unwrap_or(true);
                if should_update {
                    runtime.actions.insert(
                        key,
                        ActionRecord {
                            action,
                            updated_at: timestamp,
                        },
                    );
                }
            }
        }
    }
    /* hook 事件经 coordinator 串行 flush：debounce 内多个事件只 build/flush 一次，
    避免每次 hook 立即全项目 scan。hook-tasks-changed 同步 emit（前端 refresh 走 coordinator）。 */
    state.runtime_coord.request();
    if dynamic_project_added || refresh_action.is_some() {
        let _ = app.emit(
            "hook-tasks-changed",
            HookTasksChangedEvent {
                dynamic_project_added,
                action: refresh_action,
                project: project_hint,
            },
        );
    }
}

fn apply_hook_event(app: &AppHandle, event: HookEvent) {
    apply_hook_events(app, std::iter::once(event));
}

fn drain_runtime_queue(app: &AppHandle) {
    let events = ipc::drain_queue(&config::inbox_dir());
    if !events.is_empty() {
        apply_hook_events(app, events);
    }
}

fn start_runtime_workers(app: &AppHandle) {
    let socket = config::socket_path();
    let app_for_socket = app.clone();
    ipc::start_server(&socket, move |event| {
        let app = app_for_socket.clone();
        tauri::async_runtime::spawn(async move {
            apply_hook_event(&app, event);
        });
    });

    let app_for_queue = app.clone();
    std::thread::spawn(move || loop {
        drain_runtime_queue(&app_for_queue);
        std::thread::sleep(std::time::Duration::from_millis(250));
    });

    /* 10s reconcile：覆盖 session 过期 → Stale/Idle 等无文件、无 hook 的状态迁移。
    经 coordinator 串行 flush（不与 hook 事件并发）。 */
    let app_for_timer = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(10));
        let state = app_for_timer.state::<AppState>();
        state.runtime_coord.request();
    });
}

#[tauri::command]
fn get_runtime_snapshot(state: State<AppState>) -> RuntimeSnapshot {
    /* 读 coordinator 最近 flush 的缓存快照，而非直接 build：
    - 有缓存：返回（可能旧 250ms，但由事件/10s reconcile 补齐），并触发一次 coordinator
      request 保持新鲜（异步，不阻塞本命令，避免与 flush 并发 build）；
    - 无缓存（首次启动）：直接 build 一次初始化（reducer 副作用幂等，仅初始化）。
    这保证所有「有事件活动」期间的 build 都经 coordinator 串行，消除重复扫描和
    旧快照覆盖新状态。 */
    let cached = state.runtime.lock().unwrap().last_snapshot.clone();
    match cached {
        Some(snapshot) => {
            state.runtime_coord.request();
            snapshot
        }
        None => {
            let snapshot = build_runtime_snapshot(&state);
            state.runtime.lock().unwrap().last_snapshot = Some(snapshot.clone());
            snapshot
        }
    }
}

#[tauri::command]
fn get_hook_statuses() -> Result<Vec<hook_install::HookStatus>, String> {
    hook_install::statuses()
}

#[tauri::command]
fn configure_hook(agent: String, enabled: bool) -> Result<hook_install::HookStatus, String> {
    let agent = agent.to_ascii_lowercase();
    hook_install::install_hooks(&agent, !enabled)?;
    hook_install::status(&agent)
}

// ---------- 命令 ----------

#[tauri::command]
fn get_config(state: State<AppState>) -> ConfigOut {
    let cfg = state.config.lock().unwrap();
    ConfigOut {
        configured: cfg.initialized || !cfg.roots.is_empty() || !cfg.dynamic_projects.is_empty(),
        roots: cfg.roots.clone(),
        always_on_top: cfg.always_on_top,
    }
}

#[tauri::command]
fn complete_setup(state: State<AppState>) -> Result<(), String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.initialized = true;
    drop(cfg);
    save_state(&state)
}

// 必须 async：同步命令跑在主线程，blocking 弹窗会冻住整个应用
#[tauri::command]
async fn pick_folder(app: AppHandle) -> Option<String> {
    use tauri_plugin_dialog::DialogExt;
    let (tx, rx) = std::sync::mpsc::channel();
    app.dialog().file().pick_folder(move |p| {
        let _ = tx.send(p);
    });
    rx.recv().ok().flatten().map(|p| p.to_string())
}

#[tauri::command]
fn add_root(state: State<AppState>, path: String) -> Result<RootsOut, String> {
    let dir = expand(&path);
    if !dir.is_dir() {
        return Err("目录不存在或不可读".into());
    }
    let canonical = platform::strip_device_prefix(&dir.to_string_lossy());
    let mut cfg = state.config.lock().unwrap();
    cfg.initialized = true;
    if !cfg.roots.contains(&canonical) {
        cfg.roots.push(canonical);
    }
    let roots = cfg.roots.clone();
    drop(cfg);
    save_state(&state)?;
    Ok(RootsOut { roots })
}

#[tauri::command]
fn remove_root(state: State<AppState>, path: String) -> Result<RootsOut, String> {
    let mut cfg = state.config.lock().unwrap();
    cfg.roots.retain(|r| r != &path);
    let roots = cfg.roots.clone();
    drop(cfg);
    save_state(&state)?;
    Ok(RootsOut { roots })
}

/* 当前应用版本（更新检查用）。来自 Cargo.toml version，编译时注入。 */
#[tauri::command]
fn get_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/* 用系统默认浏览器打开 URL（跨平台：macOS open / Windows start / Linux xdg-open）。 */
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    open::that(&url).map_err(|error| format!("打开链接失败: {error}"))
}

#[tauri::command]
fn list_projects(state: State<AppState>) -> Vec<scan::ProjectInfo> {
    let cfg = state.config.lock().unwrap();
    discover_all(&cfg)
        .iter()
        .map(|dir| scan::project_info(dir))
        .collect()
}

#[tauri::command]
fn list_tasks(state: State<AppState>, project: String) -> Result<TasksPayload, String> {
    if !is_allowed_project(&state, &project) {
        return Err("项目未在扫描目录中".into());
    }
    /* 默认只扫活跃任务：常规轮询不碰 archive/，避免全项目扫描被归档目录拖慢。
    已归档任务由 list_archived 懒加载（前端勾选「显示已归档」/聚焦归档任务时调用）。 */
    let (tasks, errors) = scan::scan_tasks(&expand(&project));
    Ok(TasksPayload {
        version: scan::tasks_version(&tasks),
        tasks,
        errors,
    })
}

/* 归档任务懒加载：只扫 archive/，返回独立 archivedVersion。
前端合并回同一 project bucket 时，不得用 archivedVersion 覆盖活跃任务的 version。 */
#[tauri::command]
fn list_archived(state: State<AppState>, project: String) -> Result<ArchivedPayload, String> {
    if !is_allowed_project(&state, &project) {
        return Err("项目未在扫描目录中".into());
    }
    let (tasks, errors) = scan::scan_archived(&expand(&project));
    Ok(ArchivedPayload {
        version: scan::tasks_version(&tasks),
        tasks,
        errors,
    })
}

/* 任务关联视图数据：每个任务的 spec 引用 + dev_type/scope + 父子，以及被 ≥2 个
活跃任务引用的共享 spec（共享地盘）。只统计活跃任务，归档不参与，避免过期 spec 污染。 */
#[tauri::command]
fn list_relations(state: State<AppState>, project: String) -> Result<RelationsPayload, String> {
    if !is_allowed_project(&state, &project) {
        return Err("项目未在扫描目录中".into());
    }
    /* 关联视图纳入归档任务（spec 共享 / 父子 / 内容树都覆盖归档），
       归档任务的 spec_refs 可能引用已归档的 spec，但正是历史「地盘」的真实痕迹。
       归档任务用 archived + dir 标记，前端 keyOf 用 dir 区分身份。 */
    let (tasks, _) = scan::scan_tasks_with_archived(&expand(&project));
    let task_relations: Vec<TaskRelation> = tasks
        .iter()
        .map(|t| TaskRelation {
            id: t.id.clone(),
            dir: t.dir.clone(),
            title: t.title.clone(),
            status: t.status.clone(),
            phase: t.phase.clone(),
            priority: Some(t.priority.clone()),
            dev_type: t.dev_type.clone(),
            scope: t.scope.clone(),
            parent: t.parent.clone(),
            children: t.children.clone(),
            spec_refs: t.spec_refs.clone(),
            file_refs: t.file_refs.clone(),
            prd_refs: t.prd_refs.clone(),
            subtasks: t.subtasks.clone(),
            sessions: t.sessions.clone(),
            progress: t.progress,
            kind: t.kind.clone(),
            archived: t.archived,
        })
        .collect();
    let spec_groups = compute_spec_groups(&task_relations);
    let prd_groups = compute_prd_groups(&task_relations);
    Ok(RelationsPayload {
        tasks: task_relations,
        spec_groups,
        prd_groups,
    })
}

// 项目通用规范地图：扫描 .trellis/spec/ 返回每个规范的路径、填写状态、
// 被任务引用次数（trellis-update-spec 沉淀的团队知识资产）。
#[tauri::command]
fn list_specs(state: State<AppState>, project: String) -> Result<Vec<SpecOut>, String> {
    if !is_allowed_project(&state, &project) {
        return Err("项目未在扫描目录中".into());
    }
    let spec_root = expand(&project).join(".trellis").join("spec");
    let mut specs = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&spec_root) {
        for e in entries.flatten() {
            let dir = e.path();
            if !dir.is_dir() || dir.file_name().map(|n| n.to_string_lossy().starts_with('.')).unwrap_or(false) {
                continue;
            }
            let category = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            if let Ok(inner) = std::fs::read_dir(&dir) {
                for f in inner.flatten() {
                    let p = f.path();
                    if p.extension().map(|x| x == "md").unwrap_or(false) {
                        let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                        let content = std::fs::read_to_string(&p).unwrap_or_default();
                        /* 填写状态：有效行数（非空、非注释）超过阈值算「已沉淀经验」 */
                        let real_lines = content.lines().filter(|l| {
                            let t = l.trim();
                            !t.is_empty() && !t.starts_with("<!--") && !t.contains("To be filled") && !t.contains("To fill")
                        }).count();
                        let rel_path = format!("{}/{}", category, name);
                        specs.push(SpecOut {
                            path: rel_path,
                            name: if name == "index.md" { category.clone() } else { name.trim_end_matches(".md").to_string() },
                            category: category.clone(),
                            filled: real_lines >= 40,
                            line_count: real_lines,
                            referenced_by: 0,
                            content,
                        });
                    }
                }
            }
        }
    }
    specs.sort_by(|a, b| a.category.cmp(&b.category).then(a.path.cmp(&b.path)));
    Ok(specs)
}

// prd 显式跨任务引用（官方 FAQ Q25）：被引用的任务 → 引用它的任务列表。
// 只统计指向「本项目存在任务」的引用，避免悬空路径。
fn compute_prd_groups(relations: &[TaskRelation]) -> Vec<PrdGroup> {
    let valid_ids: std::collections::HashSet<&str> =
        relations.iter().map(|t| t.id.as_str()).collect();
    let mut ref_map: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for t in relations {
        for ref_id in &t.prd_refs {
            if valid_ids.contains(ref_id.as_str()) && ref_id != &t.id {
                ref_map.entry(ref_id.clone()).or_default().push(t.id.clone());
            }
        }
    }
    ref_map
        .into_iter()
        .map(|(ref_task_id, task_ids)| PrdGroup { ref_task_id, task_ids })
        .collect()
}

// 共享地盘：被 ≥2 个任务引用的 spec（path → 引用任务 id 列表）。
fn compute_spec_groups(relations: &[TaskRelation]) -> Vec<SpecGroup> {
    let mut ref_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for t in relations {
        for s in &t.spec_refs {
            *ref_count.entry(s.clone()).or_insert(0) += 1;
        }
    }
    ref_count
        .into_iter()
        .filter(|(_, count)| *count >= 2)
        .map(|(path, count)| SpecGroup {
            path: path.clone(),
            count,
            task_ids: relations
                .iter()
                .filter(|t| t.spec_refs.contains(&path))
                .map(|t| t.id.clone())
                .collect(),
        })
        .collect()
}

// 收集任务目录下的所有 markdown：根目录 *.md（含 acceptance-report-*）+ research/*.md
fn collect_docs(task_dir: &Path) -> Vec<DocOut> {
    const MAX_CHARS: usize = 120_000; // 单文档截断保护
    let mut names: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(task_dir) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && name.ends_with(".md") {
                names.push(name);
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(task_dir.join("research")) {
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if e.path().is_file() && name.ends_with(".md") {
                names.push(format!("research/{}", name));
            }
        }
    }
    // 固定顺序：prd/design/implement 优先，报告其次，调研再其次，其余按名字
    let rank = |n: &str| match n {
        "prd.md" => 0,
        "design.md" => 1,
        "implement.md" => 2,
        _ if n.starts_with("acceptance-report") => 3,
        _ if n.starts_with("research/") => 4,
        _ => 5,
    };
    names.sort_by(|a, b| rank(a).cmp(&rank(b)).then(a.cmp(b)));

    let label = |n: &str| -> String {
        match n {
            "prd.md" => "PRD".into(),
            "design.md" => "DESIGN".into(),
            "implement.md" => "IMPLEMENT".into(),
            _ if n.starts_with("acceptance-report") => {
                let stem = n.trim_end_matches(".md");
                let num = stem
                    .trim_start_matches("acceptance-report")
                    .trim_matches('-');
                if num.is_empty() {
                    "验收报告".into()
                } else {
                    format!("报告 {}", num)
                }
            }
            _ if n.starts_with("research/") => {
                format!(
                    "调研·{}",
                    n.trim_start_matches("research/").trim_end_matches(".md")
                )
            }
            _ => n.trim_end_matches(".md").to_string(),
        }
    };

    names
        .into_iter()
        .take(24) // 文档数量保护
        .filter_map(|n| {
            let content = std::fs::read_to_string(task_dir.join(&n)).ok()?;
            let content = if content.chars().count() > MAX_CHARS {
                let cut: String = content.chars().take(MAX_CHARS).collect();
                format!("{}\n\n…（文档过长，已截断）", cut)
            } else {
                content
            };
            Some(DocOut {
                label: label(&n),
                name: n,
                content,
            })
        })
        .collect()
}

#[tauri::command]
fn get_task(state: State<AppState>, project: String, id: String) -> Result<TaskDetail, String> {
    if !is_allowed_project(&state, &project) {
        return Err("项目未在扫描目录中".into());
    }
    let project_dir = expand(&project);
    let (tasks, _) = scan::scan_tasks_with_archived(&project_dir);
    let task = tasks
        .into_iter()
        .find(|t| t.id == id || t.dir == id)
        .ok_or_else(|| "任务不存在".to_string())?;
    let docs = collect_docs(&project_dir.join(".trellis").join("tasks").join(&task.dir));
    Ok(TaskDetail { task, docs })
}

// 校验一个任务是否可归档：项目允许、任务是活跃任务（在 .trellis/tasks/ 下、非 archive/）、
// task.json 存在、项目带 task.py 脚本。返回 (project_dir, task_dir, script) 或错误原因。
fn resolve_archivable_task(
    state: &AppState,
    project: &str,
    task: &str,
) -> Result<(PathBuf, PathBuf), String> {
    if !is_allowed_project(state, project) {
        return Err("项目未在扫描目录中".into());
    }
    let project_dir = expand(project);
    let task_dir = project_dir.join(".trellis").join("tasks").join(task);
    if !task_dir.is_dir() {
        return Err(format!("任务不存在: {task}"));
    }
    if !task_dir.join("task.json").is_file() {
        return Err(format!("任务缺少 task.json: {task}"));
    }
    /* 只允许归档活跃任务：archive/ 下已是归档，避免重复移动。 */
    let script = project_dir.join(".trellis").join("scripts").join("task.py");
    if !script.is_file() {
        return Err(format!("项目缺少 Trellis 脚本: {}", script.display()));
    }
    Ok((project_dir, script))
}

// 把活跃任务归档到 .trellis/tasks/archive/<YYYY-MM>/：执行项目自带的 task.py archive。
// 成功后 emit tasks-changed / runtime-reconciliation-needed 触发前端刷新（目录移动
// 不一定落在 watch 的相关文件路径上，主动通知保证列表即时收敛）。
#[tauri::command]
fn archive_task(
    app: AppHandle,
    state: State<AppState>,
    project: String,
    task: String,
) -> Result<bool, String> {
    let (project_dir, script) = resolve_archivable_task(&state, &project, &task)?;
    // 按平台候选探测可用的 Python 命令（win32: python/python3/py -3）
    let python = resolve_python_command();
    let output = std::process::Command::new(python)
        .arg(script)
        .args(["archive", &task])
        .current_dir(&project_dir)
        .output()
        .map_err(|e| format!("执行 task.py archive 失败: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!(
            "task.py archive 失败: {}{}",
            if stdout.is_empty() { "" } else { &stdout },
            if stderr.is_empty() { "" } else { &stderr }
        ));
    }
    let _ = app.emit("tasks-changed", ());
    let _ = app.emit("runtime-reconciliation-needed", ());
    Ok(true)
}

#[tauri::command]
fn set_always_on_top(app: AppHandle, state: State<AppState>, flag: bool) -> Result<bool, String> {
    if let Some(window) = app.get_webview_window("main") {
        window.set_always_on_top(flag).map_err(|e| e.to_string())?;
    }
    state.config.lock().unwrap().always_on_top = flag;
    save_state(&state)?;
    Ok(flag)
}

#[tauri::command]
fn set_window_mode(app: AppHandle, mode: String) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    let position = window.outer_position().ok();
    if mode == "capsule" {
        /* 胶囊是紧凑交互态，保持固定尺寸（拉伸会破坏紧凑布局）。 */
        let _ = window.set_resizable(false);
        window
            .set_size(tauri::LogicalSize::new(360.0, 136.0))
            .map_err(|e| e.to_string())?;
    } else {
        /* card 模式：恢复可拉伸，设初始尺寸，用户可自由调整。 */
        let _ = window.set_resizable(true);
        window
            .set_size(tauri::LogicalSize::new(380.0, 640.0))
            .map_err(|e| e.to_string())?;
    }
    if let Some(position) = position {
        window.set_position(position).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// 临时调整窗口尺寸（关联画布大展开用）；macOS 无边框窗口需先置可调整。
#[tauri::command]
fn set_window_size(app: AppHandle, width: f64, height: f64) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    let position = window.outer_position().ok();
    if !cfg!(windows) {
        let _ = window.set_resizable(true);
    }
    if let Err(e) = window.set_size(tauri::LogicalSize::new(width, height)) {
        if !cfg!(windows) {
            let _ = window.set_resizable(false);
        }
        return Err(e.to_string());
    }
    if let Some(position) = position {
        window.set_position(position).map_err(|e| e.to_string())?;
    }
    /* macOS 延迟恢复不可调整状态 */
    let delayed = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(180));
        if !cfg!(windows) {
            let _ = delayed.set_resizable(false);
        }
    });
    Ok(())
}

// 窗口尺寸模型（灵动岛）：胶囊紧凑/展开尺寸。
// main 合并后 set_window_mode 内联尺寸，此函数仅测试引用（保留断言）。
#[allow(dead_code)]
fn window_mode_size(mode: &str) -> (f64, f64) {
    if mode == "capsule" {
        capsule_window_size(false)
    } else {
        (380.0, 640.0)
    }
}

fn capsule_window_size(expanded: bool) -> (f64, f64) {
    (360.0, if expanded { 136.0 } else { 62.0 })
}

// 灵动岛展开/收起：原生窗口同步尺寸（消除透明点击死区）。
#[tauri::command]
fn set_capsule_expanded(app: AppHandle, expanded: bool) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    let position = window.outer_position().ok();
    let (w, h) = capsule_window_size(expanded);
    window
        .set_size(tauri::LogicalSize::new(w, h))
        .map_err(|e| e.to_string())?;
    if let Some(position) = position {
        window.set_position(position).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

// 内容自适应：只在渲染稳定后由前端一次性调用（防抖+阈值在前端），不做连续跟随。
// resizable 现在持久为 true（窗口可拉伸），fit 负责「内容高度对齐窗口」（双向：
// 内容更高放大、内容更矮缩小），保证卡片无下方空白。不再临时切 resizable 往返。
//
// 计算 fit 的目标高度：始终返回内容高度（双向），前端在用户手动拉伸时跳过缩小。
fn fit_target_height(content_h: f64) -> f64 {
    content_h.clamp(160.0, 2000.0)
}

#[tauri::command]
fn fit_window_height(app: AppHandle, height: f64) -> Result<(), String> {
    let window = app.get_webview_window("main").ok_or("窗口不存在")?;
    let position = window.outer_position().ok();
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let size = window.inner_size().map_err(|e| e.to_string())?;
    let w = size.width as f64 / scale;
    let mut h = fit_target_height(height);
    if let Ok(Some(monitor)) = window.current_monitor() {
        let max_h = monitor.size().height as f64 / scale - 40.0;
        h = h.min(max_h);
    }
    if let Err(error) = window.set_size(tauri::LogicalSize::new(w, h)) {
        return Err(error.to_string());
    }
    if let Some(position) = position {
        let _ = window.set_position(position);
    }
    Ok(())
}

// ---------- 启动 ----------

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let cfg = config::load();
            let handle = app.handle().clone();
            app.manage(AppState {
                config: Mutex::new(cfg.clone()),
                runtime: Mutex::new(RuntimeStore::default()),
                runtime_coord: RuntimeCoordinator::spawn(handle.clone()),
            });

            start_runtime_workers(app.handle());

            let window = app.get_webview_window("main").expect("main window");
            let _ = window.set_always_on_top(cfg.always_on_top);

            // 关闭窗口 = 隐藏到托盘，不退出
            let w = window.clone();
            window.on_window_event(move |event| {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = w.hide();
                }
            });

            // 托盘：显示 / 退出
            use tauri::menu::{MenuBuilder, MenuItemBuilder};
            use tauri::tray::TrayIconBuilder;
            let show = MenuItemBuilder::with_id("show", "显示 Trellis Card").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;
            let mut tray = TrayIconBuilder::with_id("main-tray")
                .menu(&menu)
                .tooltip("Trellis Card")
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(w) = app.get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;

            watch::spawn_watcher(app.handle().clone());

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_version,
            open_url,
            get_config,
            complete_setup,
            pick_folder,
            add_root,
            remove_root,
            list_projects,
            list_tasks,
            list_archived,
            list_relations,
            list_specs,
            get_task,
            archive_task,
            set_always_on_top,
            set_window_mode,
            set_window_size,
            set_capsule_expanded,
            fit_window_height,
            hide_window,
            get_runtime_snapshot,
            get_hook_statuses,
            configure_hook,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// CLI 入口由二进制入口转发，保持 GUI 与 Hook 命令共用同一 crate。
pub use hook_cli::run_hook_cli;
pub use hook_install::run_hook_install_cli;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn collect_docs_orders_and_labels() {
        let dir = std::env::temp_dir().join(format!("trellis-card-docs-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("research")).unwrap();
        std::fs::write(dir.join("implement.md"), "impl").unwrap();
        std::fs::write(dir.join("prd.md"), "prd").unwrap();
        std::fs::write(dir.join("acceptance-report-2.md"), "r2").unwrap();
        std::fs::write(dir.join("PEER_BRIEF.md"), "peer").unwrap();
        std::fs::write(dir.join("research").join("竞品.md"), "res").unwrap();
        std::fs::write(dir.join("task.json"), "{}").unwrap(); // 非 md 不收

        let docs = collect_docs(&dir);
        let labels: Vec<&str> = docs.iter().map(|d| d.label.as_str()).collect();
        assert_eq!(
            labels,
            ["PRD", "IMPLEMENT", "报告 2", "调研·竞品", "PEER_BRIEF"]
        );
        assert_eq!(docs[0].content, "prd");
        assert_eq!(docs[3].name, "research/竞品.md");

        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn fp_view(tool_input: Option<runtime::ToolActivityInput>) -> TaskRuntimeView {
        use runtime::{AgentRuntime, AgentState};
        TaskRuntimeView {
            project: "/repo/alpha".into(),
            task_id: Some("07-demo".into()),
            task_status: "in_progress".into(),
            phase: Some("implement".into()),
            display_state: runtime::DisplayState::Working,
            attention: runtime::AttentionLevel::Informative,
            confidence: runtime::Confidence::High,
            action: None,
            agent: Some(AgentRuntime {
                session_id: "s1".into(),
                agent_kind: "claude".into(),
                project: "/repo/alpha".into(),
                task_id: Some("07-demo".into()),
                event_name: "PreToolUse".into(),
                state: AgentState::Working,
                waiting_reason: None,
                tool_name: Some("Read".into()),
                tool_input,
                activity: Some("读取 task.json".into()),
                started_at: 100,
                updated_at: 100,
            }),
            activity: Some("读取 task.json".into()),
            focus_score: 5,
            last_changed_at: 100,
        }
    }

    #[test]
    fn snapshot_fingerprint_stable_for_unchanged() {
        let snapshot = RuntimeSnapshot {
            tasks: vec![fp_view(Some(runtime::ToolActivityInput {
                file_path: Some("task.json".into()),
                ..Default::default()
            }))],
            project_activities: vec![],
            errors: vec![],
            focus_key: Some("/repo/alpha::07-demo".into()),
            generated_at: 100,
        };
        assert_eq!(
            snapshot_fingerprint(&snapshot),
            snapshot_fingerprint(&snapshot)
        );
    }

    #[test]
    fn snapshot_fingerprint_changes_on_tool_input() {
        /* 同一秒内 tool_input 变化（updated_at 相同）：指纹必须变化，前端活动展示才能更新 */
        let a = RuntimeSnapshot {
            tasks: vec![fp_view(Some(runtime::ToolActivityInput {
                file_path: Some("task.json".into()),
                ..Default::default()
            }))],
            project_activities: vec![],
            errors: vec![],
            focus_key: Some("/repo/alpha::07-demo".into()),
            generated_at: 100,
        };
        let b = RuntimeSnapshot {
            tasks: vec![fp_view(Some(runtime::ToolActivityInput {
                command: Some("cargo test".into()),
                ..Default::default()
            }))],
            project_activities: vec![],
            errors: vec![],
            focus_key: Some("/repo/alpha::07-demo".into()),
            generated_at: 100,
        };
        assert_ne!(snapshot_fingerprint(&a), snapshot_fingerprint(&b));
        assert_eq!(snapshot_fingerprint(&a), snapshot_fingerprint(&a));
    }

    #[test]
    fn snapshot_fingerprint_changes_on_agent_kind_same_timestamp() {
        /* 同 updated_at 下 agent_kind 从 unknown -> claude：指纹必须变化（项目级标题/任务归属展示） */
        let mut a = fp_view(Some(runtime::ToolActivityInput {
            file_path: Some("task.json".into()),
            ..Default::default()
        }));
        if let Some(agent) = a.agent.as_mut() {
            agent.agent_kind = "unknown".into();
        }
        let mut b = a.clone();
        if let Some(agent) = b.agent.as_mut() {
            agent.agent_kind = "claude".into();
        }
        let snap_a = RuntimeSnapshot {
            tasks: vec![a],
            project_activities: vec![],
            errors: vec![],
            focus_key: Some("/repo/alpha::07-demo".into()),
            generated_at: 100,
        };
        let snap_b = RuntimeSnapshot {
            tasks: vec![b],
            project_activities: vec![],
            errors: vec![],
            focus_key: Some("/repo/alpha::07-demo".into()),
            generated_at: 100,
        };
        assert_ne!(snapshot_fingerprint(&snap_a), snapshot_fingerprint(&snap_b));
    }

    #[test]
    fn collect_docs_empty_dir() {
        assert!(collect_docs(Path::new("/nonexistent")).is_empty());
    }

    #[test]
    fn spec_groups_only_include_specs_shared_by_two_or_more() {
        let rel = |id: &str, refs: Vec<&str>| TaskRelation {
            id: id.into(),
            dir: id.into(),
            title: id.into(),
            status: "in_progress".into(),
            priority: Some("P2".into()),
            phase: scan::Phase { id: "implement".into(), label: "实现中".into(), warn: false },
            dev_type: None,
            scope: None,
            parent: None,
            children: Vec::new(),
            spec_refs: refs.into_iter().map(String::from).collect(),
            file_refs: Vec::new(),
            prd_refs: Vec::new(),
            subtasks: Vec::new(),
            sessions: Vec::new(),
            progress: 0.5,
            kind: "work".into(),
            archived: false,
        };
        let relations = vec![
            rel("a", vec![".trellis/spec/frontend/index.md"]),
            rel("b", vec![".trellis/spec/frontend/index.md"]),
            rel("c", vec!["research/x.md"]), // 私有引用不产生共享
        ];
        let groups = compute_spec_groups(&relations);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].path, ".trellis/spec/frontend/index.md");
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[0].task_ids, vec!["a", "b"]);
    }

    #[test]
    fn spec_groups_empty_when_no_shared_spec() {
        let rel = |id: &str, refs: Vec<&str>| TaskRelation {
            id: id.into(),
            dir: id.into(),
            title: id.into(),
            status: "in_progress".into(),
            priority: Some("P2".into()),
            phase: scan::Phase { id: "implement".into(), label: "实现中".into(), warn: false },
            dev_type: None,
            scope: None,
            parent: None,
            children: Vec::new(),
            spec_refs: refs.into_iter().map(String::from).collect(),
            file_refs: Vec::new(),
            prd_refs: Vec::new(),
            subtasks: Vec::new(),
            sessions: Vec::new(),
            progress: 0.5,
            kind: "work".into(),
            archived: false,
        };
        let relations = vec![
            rel("a", vec![".trellis/spec/frontend/index.md"]),
            rel("b", vec![".trellis/spec/backend/x.md"]), // 各引各的，无共享
        ];
        assert!(compute_spec_groups(&relations).is_empty());
    }

    #[test]
    fn prd_groups_only_include_existing_task_refs() {
        let rel = |id: &str, prd_refs: Vec<&str>| TaskRelation {
            id: id.into(),
            dir: id.into(),
            title: id.into(),
            status: "in_progress".into(),
            priority: Some("P2".into()),
            phase: scan::Phase { id: "implement".into(), label: "实现中".into(), warn: false },
            dev_type: None,
            scope: None,
            parent: None,
            children: Vec::new(),
            spec_refs: Vec::new(),
            file_refs: Vec::new(),
            prd_refs: prd_refs.into_iter().map(String::from).collect(),
            subtasks: Vec::new(),
            sessions: Vec::new(),
            progress: 0.5,
            kind: "work".into(),
            archived: false,
        };
        let relations = vec![
            rel("a", vec!["b", "ghost"]), // 引 b（存在）+ ghost（不存在，应过滤）
            rel("b", vec![]),
            rel("c", vec!["b"]), // 也引 b → b 被 2 个任务引用
        ];
        let groups = compute_prd_groups(&relations);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].ref_task_id, "b");
        assert_eq!(groups[0].task_ids, vec!["a", "c"]);
    }

    #[test]
    fn capsule_window_uses_compact_and_expanded_heights() {
        assert_eq!(window_mode_size("capsule"), (360.0, 62.0));
        assert_eq!(capsule_window_size(false), (360.0, 62.0));
        assert_eq!(capsule_window_size(true), (360.0, 136.0));
        assert_eq!(window_mode_size("card"), (380.0, 640.0));
    }

    #[test]
    fn discover_all_includes_hook_discovered_project_outside_roots() {
        let project = std::env::temp_dir().join(format!(
            "trellis-card-dynamic-project-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(project.join(".trellis/tasks")).unwrap();
        let cfg = AppConfig {
            roots: vec![],
            dynamic_projects: vec![project.to_string_lossy().into_owned()],
            initialized: false,
            always_on_top: false,
        };

        assert_eq!(discover_all(&cfg), vec![project.clone()]);
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn project_dir_for_path_finds_trellis_ancestor() {
        let project = std::env::temp_dir().join(format!(
            "trellis-card-dynamic-ancestor-{}",
            std::process::id()
        ));
        let nested = project.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::create_dir_all(project.join(".trellis/tasks")).unwrap();

        assert_eq!(
            project_dir_for_path(&nested),
            Some(platform::normalize_path(&project))
        );
        std::fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn project_dir_for_path_rejects_non_trellis_ancestor() {
        let root =
            std::env::temp_dir().join(format!("trellis-card-non-project-{}", std::process::id()));
        let nested = root.join("src/deep");
        std::fs::create_dir_all(&nested).unwrap();

        assert_eq!(project_dir_for_path(&nested), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn child_project_under_trellis_root_is_discovered_by_scan() {
        /* root 和其子目录 child 都是 Trellis 项目：扫描应同时发现两者（嵌套项目可见） */
        let root =
            std::env::temp_dir().join(format!("trellis-card-nested-root-{}", std::process::id()));
        let child = root.join("child");
        std::fs::create_dir_all(root.join(".trellis/tasks")).unwrap();
        std::fs::create_dir_all(child.join(".trellis/tasks")).unwrap();
        let cfg = AppConfig {
            roots: vec![root.to_string_lossy().into_owned()],
            dynamic_projects: vec![],
            initialized: false,
            always_on_top: false,
        };

        assert!(is_discovered_project(&cfg, &child));
        assert!(discover_all(&cfg).contains(&child));
        std::fs::remove_dir_all(root).unwrap();
    }

    /* ---- 完成迁移 reducer（P0-1） ---- */

    #[test]
    fn completion_transition_unfinished_to_completed() {
        assert!(is_completion_transition(Some("in_progress"), "completed"));
        assert!(is_completion_transition(Some("planning"), "done"));
    }

    #[test]
    fn completion_transition_first_scan_does_not_emit() {
        /* 初次扫描：无上次记录，不生成完成事件 */
        assert!(!is_completion_transition(None, "completed"));
        assert!(!is_completion_transition(None, "in_progress"));
    }

    #[test]
    fn completion_transition_no_change_does_not_emit() {
        assert!(!is_completion_transition(
            Some("in_progress"),
            "in_progress"
        ));
        assert!(!is_completion_transition(Some("planning"), "planning"));
    }

    #[test]
    fn completion_transition_completed_to_completed_not_repeated() {
        /* 重复 completed：不重复生成（第二次 completed 不再迁移） */
        assert!(!is_completion_transition(Some("completed"), "completed"));
        assert!(!is_completion_transition(Some("done"), "done"));
    }

    #[test]
    fn completion_transition_blocked_to_blocked_not_emit() {
        assert!(!is_completion_transition(Some("blocked"), "blocked"));
        assert!(!is_completion_transition(Some("failed"), "failed"));
    }

    #[test]
    fn completion_transition_unfinished_to_unfinished_not_emit() {
        assert!(!is_completion_transition(Some("in_progress"), "blocked"));
        assert!(!is_completion_transition(Some("planning"), "review"));
    }

    /* ---- fit_target_height（窗口「只增不减」语义） ---- */

    #[test]
    fn fit_target_height_returns_content_height() {
        /* 双向：内容高度始终为目标（放大或缩小），前端负责「用户拉伸不缩小」 */
        assert_eq!(fit_target_height(500.0), 500.0);
        assert_eq!(fit_target_height(640.0), 640.0);
        assert_eq!(fit_target_height(400.0), 400.0);
        assert_eq!(fit_target_height(300.0), 300.0);
    }

    #[test]
    fn fit_target_height_clamps_to_min() {
        /* 低于最小高度 → clamp 到 160 */
        assert_eq!(fit_target_height(50.0), 160.0);
    }

    #[test]
    fn completion_pending_event_carries_project_task_completed_at() {
        /* 集成：模拟 reducer 写入 pending，验证 payload 字段 */
        let mut store = RuntimeStore::default();
        let key = task_key("/repo/alpha", "07-demo");
        store
            .task_statuses
            .insert(key.clone(), "in_progress".into());
        /* 构造最小 scan::Task 手动走 reducer 逻辑（不经 build_runtime_snapshot） */
        let now = now_seconds();
        if is_completion_transition(
            store.task_statuses.get(&key).map(String::as_str),
            "completed",
        ) {
            store.pending_completions.push(TaskCompletedEvent {
                project: "/repo/alpha".into(),
                task_id: "07-demo".into(),
                completed_at: now,
            });
        }
        store.task_statuses.insert(key.clone(), "completed".into());
        assert_eq!(store.pending_completions.len(), 1);
        let ev = &store.pending_completions[0];
        assert_eq!(ev.project, "/repo/alpha");
        assert_eq!(ev.task_id, "07-demo");
        assert_eq!(ev.completed_at, now);
        /* 第二次扫描（completed->completed）：不新增 pending */
        if is_completion_transition(
            store.task_statuses.get(&key).map(String::as_str),
            "completed",
        ) {
            store.pending_completions.push(TaskCompletedEvent {
                project: "/repo/alpha".into(),
                task_id: "07-demo".into(),
                completed_at: now,
            });
        }
        assert_eq!(store.pending_completions.len(), 1);
    }

    // 构造一个 roots 指向临时项目的 AppState，供归档校验测试使用
    fn state_with_project(project: &Path) -> AppState {
        AppState {
            config: Mutex::new(AppConfig {
                roots: vec![project.to_string_lossy().into_owned()],
                dynamic_projects: vec![],
                initialized: true,
                always_on_top: false,
            }),
            runtime: Mutex::new(RuntimeStore::default()),
            runtime_coord: RuntimeCoordinator::new(),
        }
    }

    fn archive_fixture(tag: &str) -> (std::path::PathBuf, AppState) {
        let root = std::env::temp_dir().join(format!(
            "trellis-card-archive-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("alpha");
        std::fs::create_dir_all(project.join(".trellis/tasks/02-27-user-login")).unwrap();
        std::fs::create_dir_all(project.join(".trellis/scripts")).unwrap();
        std::fs::write(
            project.join(".trellis/scripts/task.py"),
            "#!/usr/bin/env python3\n",
        )
        .unwrap();
        (root, state_with_project(&project))
    }

    #[test]
    fn resolve_archivable_task_accepts_active_task() {
        let (root, state) = archive_fixture("accept");
        let project = root.join("alpha");
        std::fs::write(
            project.join(".trellis/tasks/02-27-user-login/task.json"),
            r#"{"id":"02-27-user-login","status":"completed"}"#,
        )
        .unwrap();

        let (proj, script) =
            resolve_archivable_task(&state, &project.to_string_lossy(), "02-27-user-login")
                .unwrap();
        assert_eq!(proj, project);
        assert_eq!(script, project.join(".trellis/scripts/task.py"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_archivable_task_rejects_missing_task() {
        let (root, state) = archive_fixture("missing");
        let project = root.join("alpha");
        let err = resolve_archivable_task(&state, &project.to_string_lossy(), "no-such-task")
            .unwrap_err();
        assert!(err.contains("任务不存在"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn resolve_archivable_task_rejects_missing_script() {
        let (root, state) = archive_fixture("noscript");
        let project = root.join("alpha");
        std::fs::write(
            project.join(".trellis/tasks/02-27-user-login/task.json"),
            r#"{"id":"02-27-user-login","status":"completed"}"#,
        )
        .unwrap();
        std::fs::remove_file(project.join(".trellis/scripts/task.py")).unwrap();

        let err = resolve_archivable_task(&state, &project.to_string_lossy(), "02-27-user-login")
            .unwrap_err();
        assert!(err.contains("缺少 Trellis 脚本"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    /* ---------- Hook 端到端集成测试 ----------
    打通完整链路：真实 payload → parse_hook_payload → IPC 编解码 → SessionRegistry 应用
    → build_runtime_snapshot 反映到前端可读的运行时快照。 */

    fn hook_fixture(tag: &str) -> (std::path::PathBuf, AppState) {
        let root = std::env::temp_dir().join(format!(
            "trellis-card-hook-e2e-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("alpha");
        std::fs::create_dir_all(project.join(".trellis/tasks/02-27-user-login")).unwrap();
        std::fs::write(
            project.join(".trellis/tasks/02-27-user-login/task.json"),
            r#"{"id":"02-27-user-login","title":"User login","status":"in_progress"}"#,
        )
        .unwrap();
        /* normalize 对齐 parse_hook_payload 的路径输出（/var ↔ /private/var，去 Windows \\?\ 前缀） */
        let project = platform::normalize_path(&project);
        (root, state_with_project(&project))
    }

    #[test]
    fn hook_payload_reaches_runtime_snapshot_end_to_end() {
        let (root, state) = hook_fixture("flow");
        let project = platform::normalize_path(&root.join("alpha"));
        let project_str = project.to_string_lossy().into_owned();
        // Windows 路径含反斜杠，JSON 里要转义，否则 "invalid escape"
        let project_json = project_str.replace('\\', "\\\\");

        /* 1) 真实 Codex 风格 payload → 解析成 HookEvent */
        let payload = format!(
            r#"{{
              "hook_event_name": "PreToolUse",
              "session_id": "sess-1",
              "agent": "codex",
              "cwd": "{project_json}",
              "task_id": "02-27-user-login",
              "tool_name": "Bash",
              "command": "cargo test",
              "timestamp": 1000
            }}"#
        );
        let overrides = crate::hook_cli::HookOverrides {
            agent: None,
            event: None,
            session: None,
            project: None,
            task: None,
        };
        let event =
            crate::hook_cli::parse_hook_payload(&payload, &overrides).expect("payload 应解析成功");
        assert_eq!(event.session_id, "sess-1");
        assert_eq!(event.task_id.as_deref(), Some("02-27-user-login"));
        assert_eq!(event.event_name, "PreToolUse");

        /* 2) IPC 编解码往返：模拟 hook → socket/队列 → 后端 */
        let encoded = ipc::encode_event(&event).unwrap();
        let decoded = ipc::decode_event(encoded.trim()).unwrap();
        assert_eq!(decoded, event);

        /* 3) 应用事件到 SessionRegistry（apply_hook_events 的核心状态迁移；
        不调用 register_dynamic_project——那会写真实配置文件，测试必须隔离） */
        let mut runtime = state.runtime.lock().unwrap();
        assert!(runtime.sessions.apply(decoded.clone()));
        drop(runtime);

        /* 4) 运行时快照应包含该会话活动（前端 agent-state-changed 的数据源）。
        state_with_project 已把临时项目设为 root，build_runtime_snapshot 会扫描到它。 */
        let snapshot = build_runtime_snapshot(&state);
        let expected_key = format!("{}::02-27-user-login", project_str);
        assert_eq!(snapshot.focus_key, Some(expected_key));
        let activity = snapshot
            .project_activities
            .iter()
            .find(|a| a.session_id == "sess-1");
        assert!(activity.is_some(), "快照应包含 hook 会话活动");
        assert_eq!(activity.unwrap().project, project_str);

        std::fs::remove_dir_all(&root).unwrap();
    }
}
