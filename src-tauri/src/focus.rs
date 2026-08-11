use crate::runtime::{
    AgentRuntime, AgentState, AttentionLevel, Confidence, DisplayState, TaskRuntimeView,
    TrellisAction,
};
use crate::scan::Task;

fn score_for(view: &TaskRuntimeView) -> i32 {
    let state_score = match view.display_state {
        DisplayState::WaitingPermission | DisplayState::WaitingQuestion => 1_000,
        DisplayState::Blocked => 800,
        DisplayState::Failed => 850,
        DisplayState::Working => 500,
        DisplayState::Reviewing => 350,
        DisplayState::TurnDone => 300,
        DisplayState::Completed => 250,
        DisplayState::Stale => 100,
        DisplayState::Planning => 80,
        DisplayState::Idle => 0,
    };
    let action_score = view.action.map(|_| 150).unwrap_or(0);
    let confidence_penalty = match view.confidence {
        Confidence::High => 0,
        Confidence::Medium => 20,
        Confidence::Low => 1_000,
    };
    state_score + action_score + view.focus_score - confidence_penalty
}

fn view_key(view: &TaskRuntimeView) -> Option<String> {
    let task_id = view.task_id.as_ref()?;
    if view.project.is_empty() {
        Some(task_id.clone())
    } else {
        Some(format!("{}::{task_id}", view.project))
    }
}

pub fn fuse_task(
    task: &Task,
    session: Option<&AgentRuntime>,
    action: Option<TrellisAction>,
) -> TaskRuntimeView {
    let agent_state = session.map(|s| s.state).unwrap_or(AgentState::None);
    let waiting_reason = session.and_then(|s| s.waiting_reason);
    let display_state = DisplayState::derive(&task.status, agent_state, waiting_reason);
    let confidence = match session {
        Some(s)
            if matches!(
                s.task_id.as_deref(),
                Some(id) if id == task.id.as_str() || id == task.dir.as_str()
            ) =>
        {
            Confidence::High
        }
        Some(_) => Confidence::Medium,
        None => Confidence::High,
    };
    let task_mtime = if task.mtime > 100_000_000_000 {
        task.mtime / 1_000
    } else {
        task.mtime
    };
    let last_changed_at = session
        .map(|s| s.updated_at.max(task_mtime))
        .unwrap_or(task_mtime);
    let activity = session.and_then(|s| s.activity.clone().or_else(|| s.tool_name.clone()));
    let base_score = match display_state {
        DisplayState::WaitingPermission | DisplayState::WaitingQuestion => 1_000,
        DisplayState::Blocked => 800,
        DisplayState::Failed => 850,
        DisplayState::Working => 500,
        DisplayState::Reviewing => 350,
        DisplayState::TurnDone => 300,
        DisplayState::Completed => 250,
        DisplayState::Stale => 100,
        DisplayState::Planning => 80,
        DisplayState::Idle => 0,
    };
    TaskRuntimeView {
        project: session.map(|s| s.project.clone()).unwrap_or_default(),
        task_id: Some(task.id.clone()),
        task_status: task.status.clone(),
        phase: Some(task.phase.id.clone()),
        display_state,
        attention: AttentionLevel::for_display(display_state),
        confidence,
        action,
        agent: session.cloned(),
        activity,
        focus_score: base_score + i32::from(task.phase.warn) * 50,
        last_changed_at,
    }
}

pub fn choose_focus(
    views: &[TaskRuntimeView],
    previous: Option<&str>,
    _now: i64,
) -> Option<String> {
    if let Some(previous) = previous {
        if let Some(current) = views
            .iter()
            .find(|v| view_key(v).as_deref() == Some(previous))
        {
            let candidate = views
                .iter()
                .filter(|v| view_key(v).as_deref() != Some(previous))
                .filter(|v| v.confidence != Confidence::Low)
                .max_by_key(|view| score_for(view));
            if candidate.is_none() {
                return Some(previous.to_string());
            }
            if let Some(candidate) = candidate {
                if should_hold_focus(current, candidate, _now) {
                    return Some(previous.to_string());
                }
            }
        }
    }
    // 无 previous 焦点（启动/首次）：按最近变化主导，分数次级。
    // 避免打开应用停在一个很久以前完成/闲置的任务上——选「最近有变化」的任务，
    // 分数只在时间相当时作为次级键。
    views
        .iter()
        .filter(|v| v.task_id.is_some())
        .filter(|v| v.confidence != Confidence::Low)
        .max_by_key(|v| (v.last_changed_at, score_for(v)))
        .and_then(view_key)
}

pub fn should_hold_focus(current: &TaskRuntimeView, candidate: &TaskRuntimeView, now: i64) -> bool {
    if view_key(current) == view_key(candidate) {
        return true;
    }
    if candidate.confidence == Confidence::Low {
        return true;
    }
    let age = now.saturating_sub(current.last_changed_at);
    match current.display_state {
        DisplayState::WaitingPermission | DisplayState::WaitingQuestion => age <= 300,
        DisplayState::Completed => age <= 3,
        _ => age < 8,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{AttentionLevel, Confidence, DisplayState, TaskRuntimeView};
    use crate::scan::{Artifacts, Phase, Task};

    fn view(
        key: &str,
        state: DisplayState,
        score: i32,
        confidence: Confidence,
        changed: i64,
    ) -> TaskRuntimeView {
        TaskRuntimeView {
            project: "/repo".into(),
            task_id: Some(key.into()),
            task_status: "in_progress".into(),
            phase: Some("implement".into()),
            display_state: state,
            attention: if matches!(
                state,
                DisplayState::WaitingPermission | DisplayState::WaitingQuestion
            ) {
                AttentionLevel::Critical
            } else {
                AttentionLevel::Informative
            },
            confidence,
            action: None,
            agent: None,
            activity: None,
            focus_score: score,
            last_changed_at: changed,
        }
    }

    #[test]
    fn waiting_beats_working_when_scoring_focus() {
        let views = vec![
            view("working", DisplayState::Working, 500, Confidence::High, 10),
            view(
                "waiting",
                DisplayState::WaitingPermission,
                900,
                Confidence::High,
                11,
            ),
        ];
        assert_eq!(
            choose_focus(&views, None, 20).as_deref(),
            Some("/repo::waiting")
        );
    }

    #[test]
    fn ordinary_focus_is_held_for_eight_seconds() {
        let current = view("current", DisplayState::Working, 400, Confidence::High, 100);
        let candidate = view(
            "candidate",
            DisplayState::Working,
            900,
            Confidence::High,
            101,
        );
        assert!(should_hold_focus(&current, &candidate, 107));
        assert!(!should_hold_focus(&current, &candidate, 109));
    }

    #[test]
    fn low_confidence_event_cannot_steal_task_focus() {
        let views = vec![
            view("current", DisplayState::Working, 400, Confidence::High, 100),
            view("inferred", DisplayState::Working, 900, Confidence::Low, 101),
        ];
        assert_eq!(
            choose_focus(&views, Some("/repo::current"), 110).as_deref(),
            Some("/repo::current")
        );
    }

    #[test]
    fn startup_prefers_recently_changed_over_high_score() {
        /* 启动（无 previous）：老任务分数高但 24 天前变化，新任务分数低但刚变化，
        应按 last_changed_at 选新任务，而不是停在旧任务上。 */
        let views = vec![
            view(
                "stale-old",
                DisplayState::Completed,
                250,
                Confidence::High,
                100,
            ),
            view("fresh-new", DisplayState::Idle, 0, Confidence::High, 9_000),
        ];
        assert_eq!(
            choose_focus(&views, None, 9_100).as_deref(),
            Some("/repo::fresh-new")
        );
    }

    #[test]
    fn startup_skips_low_confidence_candidates() {
        let views = vec![
            view("fresh", DisplayState::Working, 500, Confidence::High, 9_000),
            view(
                "inferred",
                DisplayState::Completed,
                250,
                Confidence::Low,
                9_001,
            ),
        ];
        assert_eq!(
            choose_focus(&views, None, 9_100).as_deref(),
            Some("/repo::fresh")
        );
    }

    #[test]
    fn task_directory_alias_keeps_runtime_confidence_high() {
        let task = Task {
            id: "cpa-cpamp-apk".into(),
            title: "CPA".into(),
            description: String::new(),
            status: "planning".into(),
            priority: "P2".into(),
            dev_type: None,
            scope: None,
            package: None,
            branch: None,
            parent: None,
            children: Vec::new(),
            subtasks: Vec::new(),
            created_at: None,
            completed_at: None,
            mtime: 1,
            progress: 0.0,
            stage: "规划".into(),
            lane: 0,
            partial: 0.0,
            kind: "task".into(),
            archived: false,
            sessions: Vec::new(),
            excerpt: String::new(),
            artifacts: Artifacts::default(),
            spec_refs: Vec::new(),
            file_refs: Vec::new(),
            prd_refs: Vec::new(),
            phase: Phase {
                id: "explore".into(),
                label: "规划".into(),
                warn: false,
            },
            dir: "07-26-cpa-cpamp-apk".into(),
        };
        let session = AgentRuntime {
            session_id: "s1".into(),
            agent_kind: "codex".into(),
            project: "/repo".into(),
            task_id: Some("07-26-cpa-cpamp-apk".into()),
            event_name: "PreToolUse".into(),
            state: AgentState::Working,
            waiting_reason: None,
            tool_name: Some("trellis-implement".into()),
            tool_input: None,
            activity: None,
            started_at: 10,
            updated_at: 20,
        };
        let view = fuse_task(&task, Some(&session), None);
        assert_eq!(view.confidence, Confidence::High);
        assert_eq!(view.display_state, DisplayState::Working);
    }
}
