use crate::runtime::{AgentRuntime, AgentState, ToolActivityInput, TrellisAction, WaitingReason};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct HookEvent {
    pub session_id: String,
    pub agent_kind: String,
    pub project: String,
    pub task_id: Option<String>,
    pub event_name: String,
    pub tool_name: Option<String>,
    #[serde(default)]
    pub tool_input: Option<ToolActivityInput>,
    pub activity: Option<String>,
    pub action: Option<TrellisAction>,
    pub timestamp: i64,
}

#[cfg(test)]
impl HookEvent {
    fn new(
        session_id: &str,
        project: &str,
        task_id: Option<&str>,
        event_name: &str,
        timestamp: i64,
    ) -> Self {
        Self {
            session_id: session_id.into(),
            agent_kind: "unknown".into(),
            project: project.into(),
            task_id: task_id.map(str::to_owned),
            event_name: event_name.into(),
            tool_name: None,
            tool_input: None,
            activity: None,
            action: None,
            timestamp,
        }
    }

    pub fn working(session_id: &str, project: &str, task_id: Option<&str>, timestamp: i64) -> Self {
        Self::new(session_id, project, task_id, "PreToolUse", timestamp)
    }

    pub fn permission(session_id: &str, project: &str, task_id: &str, timestamp: i64) -> Self {
        Self::new(
            session_id,
            project,
            Some(task_id),
            "PermissionRequest",
            timestamp,
        )
    }

    pub fn stop(session_id: &str, project: &str, task_id: Option<&str>, timestamp: i64) -> Self {
        Self::new(session_id, project, task_id, "Stop", timestamp)
    }
}

pub struct SessionRegistry {
    sessions: HashMap<String, AgentRuntime>,
    active_timeout: i64,
    registered_timeout: i64,
}

impl Default for SessionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionRegistry {
    pub fn new() -> Self {
        Self::with_timeouts(300, 90)
    }

    pub fn with_timeouts(active_timeout: i64, registered_timeout: i64) -> Self {
        Self {
            sessions: HashMap::new(),
            active_timeout,
            registered_timeout,
        }
    }

    pub fn apply(&mut self, event: HookEvent) -> bool {
        let state = event.event_name.to_ascii_lowercase();
        let terminal = state == "stop"
            || state == "sessionend"
            || state == "session_end"
            || state.ends_with(".idle")
            || state.ends_with("_idle")
            || state.ends_with("idle")
            || state.ends_with(".completed")
            || state.ends_with("_completed")
            || state.ends_with("completed")
            || state.ends_with(".done")
            || state.ends_with("_done")
            || state.ends_with("done")
            || state.ends_with(".ended")
            || state.ends_with("_ended")
            || state.ends_with("ended");
        let (agent_state, waiting_reason) = if state.contains("permission") {
            (AgentState::Waiting, Some(WaitingReason::Permission))
        } else if state.contains("question") || state.contains("askuser") {
            (AgentState::Waiting, Some(WaitingReason::Question))
        } else if terminal {
            (AgentState::Done, None)
        } else if state == "sessionstart" || state == "session_start" {
            (AgentState::None, None)
        } else if state.contains("error") || state.contains("fail") {
            (AgentState::Failed, None)
        } else {
            (AgentState::Working, None)
        };

        let entry = self
            .sessions
            .entry(event.session_id.clone())
            .or_insert_with(|| AgentRuntime {
                session_id: event.session_id.clone(),
                agent_kind: event.agent_kind.clone(),
                project: event.project.clone(),
                task_id: event.task_id.clone(),
                event_name: event.event_name.clone(),
                state: AgentState::None,
                waiting_reason: None,
                tool_name: None,
                tool_input: None,
                activity: None,
                started_at: event.timestamp,
                updated_at: event.timestamp,
            });
        if event.timestamp < entry.updated_at {
            return false;
        }
        if entry.agent_kind == "unknown" && event.agent_kind != "unknown" {
            entry.agent_kind = event.agent_kind;
        }
        if !event.project.is_empty() {
            entry.project = event.project;
        }
        if event.task_id.is_some() {
            entry.task_id = event.task_id;
        }
        entry.state = agent_state;
        entry.waiting_reason = waiting_reason;
        entry.event_name = event.event_name;
        entry.tool_name = event.tool_name;
        entry.tool_input = event.tool_input;
        entry.activity = event.activity;
        entry.updated_at = event.timestamp;
        true
    }

    pub fn prune(&mut self, now: i64) {
        let active_timeout = self.active_timeout;
        let registered_timeout = self.registered_timeout;
        self.sessions.retain(|_, session| {
            let age = now.saturating_sub(session.updated_at);
            match session.state {
                AgentState::Working | AgentState::Waiting | AgentState::Failed => {
                    if age > active_timeout {
                        session.state = AgentState::Stale;
                    }
                    true
                }
                AgentState::None | AgentState::Done | AgentState::Stale => {
                    age <= registered_timeout
                }
            }
        });
    }

    pub fn latest_for_task(&self, project: &str, task_id: &str) -> Option<&AgentRuntime> {
        self.sessions
            .values()
            .filter(|s| s.project == project && s.task_id.as_deref() == Some(task_id))
            .max_by_key(|s| s.updated_at)
    }

    pub fn task_id_for_session(&self, session_id: &str) -> Option<&str> {
        self.sessions
            .get(session_id)
            .and_then(|session| session.task_id.as_deref())
    }

    pub fn all_activity(&self) -> Vec<AgentRuntime> {
        self.sessions.values().cloned().collect()
    }
}

#[cfg(test)]
impl SessionRegistry {
    fn get(&self, session_id: &str) -> Option<&AgentRuntime> {
        self.sessions.get(session_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{AgentState, WaitingReason};

    #[test]
    fn permission_request_waits_without_completing_task() {
        let mut registry = SessionRegistry::new();
        registry.apply(HookEvent::permission("s1", "/repo", "07-demo", 10));
        let s = registry.get("s1").unwrap();
        assert_eq!(s.state, AgentState::Waiting);
        assert_eq!(s.waiting_reason, Some(WaitingReason::Permission));
    }

    #[test]
    fn stop_marks_agent_done_without_changing_task_status() {
        let mut registry = SessionRegistry::new();
        registry.apply(HookEvent::working("s1", "/repo", Some("07-demo"), 10));
        registry.apply(HookEvent::stop("s1", "/repo", Some("07-demo"), 20));
        assert_eq!(registry.get("s1").unwrap().state, AgentState::Done);
        assert_eq!(
            registry.get("s1").unwrap().task_id.as_deref(),
            Some("07-demo")
        );
    }

    #[test]
    fn opencode_idle_event_marks_agent_done() {
        let mut registry = SessionRegistry::new();
        registry.apply(HookEvent::working("s1", "/repo", Some("07-demo"), 10));
        let mut event = HookEvent::stop("s1", "/repo", Some("07-demo"), 20);
        event.event_name = "session.idle".into();
        registry.apply(event);
        assert_eq!(registry.get("s1").unwrap().state, AgentState::Done);
    }

    #[test]
    fn stale_pruning_drops_quiet_active_sessions() {
        let mut registry = SessionRegistry::with_timeouts(300, 90);
        registry.apply(HookEvent::working("s1", "/repo", Some("07-demo"), 10));
        registry.prune(311);
        assert_eq!(registry.get("s1").unwrap().state, AgentState::Stale);
    }

    #[test]
    fn unknown_task_is_project_level() {
        let mut registry = SessionRegistry::new();
        registry.apply(HookEvent::working("s1", "/repo", None, 10));
        assert_eq!(registry.get("s1").unwrap().task_id, None);
    }

    #[test]
    fn session_task_can_be_reused_by_followup_events() {
        let mut registry = SessionRegistry::new();
        registry.apply(HookEvent::working("s1", "/repo", Some("07-demo"), 10));
        assert_eq!(registry.task_id_for_session("s1"), Some("07-demo"));
        assert_eq!(registry.task_id_for_session("missing"), None);
    }
}
