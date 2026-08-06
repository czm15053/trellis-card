use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    None,
    Working,
    Waiting,
    Done,
    Stale,
    Failed,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WaitingReason {
    Permission,
    Question,
    Review,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrellisAction {
    Create,
    Brainstorm,
    Research,
    Prd,
    Context,
    Activate,
    Implement,
    Check,
    Rollback,
    BreakLoop,
    UpdateSpec,
    Archive,
}

pub const ACTION_TTL_SECONDS: i64 = 300;

/// Agent hook 的结构化工具输入，保留 AgentPet formatter 需要的字段。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolActivityInput {
    pub file_path: Option<String>,
    pub command: Option<String>,
    pub description: Option<String>,
    pub pattern: Option<String>,
    pub query: Option<String>,
    pub url: Option<String>,
    pub prompt: Option<String>,
    pub subagent_type: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ActionRecord {
    pub action: TrellisAction,
    pub updated_at: i64,
}

impl ActionRecord {
    pub fn is_fresh(&self, now: i64) -> bool {
        now.saturating_sub(self.updated_at) <= ACTION_TTL_SECONDS
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DisplayState {
    Planning,
    Working,
    WaitingPermission,
    WaitingQuestion,
    Reviewing,
    TurnDone,
    Blocked,
    Stale,
    Failed,
    Completed,
    Idle,
}

impl DisplayState {
    pub fn derive(
        task_status: &str,
        agent_state: AgentState,
        waiting_reason: Option<WaitingReason>,
    ) -> Self {
        if matches!(task_status, "completed" | "done") {
            return Self::Completed;
        }
        if agent_state == AgentState::Waiting {
            return match waiting_reason {
                Some(WaitingReason::Permission) => Self::WaitingPermission,
                Some(WaitingReason::Question) => Self::WaitingQuestion,
                Some(WaitingReason::Review) | None => Self::Reviewing,
            };
        }
        if task_status == "blocked" {
            return Self::Blocked;
        }
        /* failed 优先于 Working/Done：failed task 即使 Agent 正在工作或本轮结束也显示执行失败 */
        if task_status == "failed" || agent_state == AgentState::Failed {
            return Self::Failed;
        }
        if agent_state == AgentState::Working {
            return Self::Working;
        }
        if task_status == "review" {
            return Self::Reviewing;
        }
        if agent_state == AgentState::Done {
            return Self::TurnDone;
        }
        if agent_state == AgentState::Stale {
            return Self::Stale;
        }
        if task_status == "planning" {
            return Self::Planning;
        }
        Self::Idle
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AttentionLevel {
    Normal,
    Informative,
    Required,
    Critical,
}

impl AttentionLevel {
    pub fn for_display(state: DisplayState) -> Self {
        match state {
            DisplayState::WaitingPermission | DisplayState::WaitingQuestion => Self::Critical,
            DisplayState::Blocked | DisplayState::Failed => Self::Required,
            DisplayState::Working | DisplayState::Reviewing | DisplayState::TurnDone => {
                Self::Informative
            }
            _ => Self::Normal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentRuntime {
    pub session_id: String,
    pub agent_kind: String,
    pub project: String,
    pub task_id: Option<String>,
    pub event_name: String,
    pub state: AgentState,
    pub waiting_reason: Option<WaitingReason>,
    pub tool_name: Option<String>,
    pub tool_input: Option<ToolActivityInput>,
    pub activity: Option<String>,
    pub started_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskRuntimeView {
    pub project: String,
    pub task_id: Option<String>,
    pub task_status: String,
    pub phase: Option<String>,
    pub display_state: DisplayState,
    pub attention: AttentionLevel,
    pub confidence: Confidence,
    pub action: Option<TrellisAction>,
    pub agent: Option<AgentRuntime>,
    pub activity: Option<String>,
    pub focus_score: i32,
    pub last_changed_at: i64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completed_task_wins_over_agent_done() {
        let view = DisplayState::derive("completed", AgentState::Done, None);
        assert_eq!(view, DisplayState::Completed);
    }

    #[test]
    fn waiting_reason_becomes_specific_display_state() {
        assert_eq!(
            DisplayState::derive(
                "in_progress",
                AgentState::Waiting,
                Some(WaitingReason::Permission),
            ),
            DisplayState::WaitingPermission
        );
    }

    #[test]
    fn stop_does_not_complete_an_in_progress_task() {
        assert_eq!(
            DisplayState::derive("in_progress", AgentState::Done, None),
            DisplayState::TurnDone
        );
    }

    #[test]
    fn public_names_use_snake_case() {
        let json = serde_json::to_string(&WaitingReason::Permission).unwrap();
        assert_eq!(json, "\"permission\"");
    }

    #[test]
    fn action_record_is_fresh_inside_ttl() {
        let record = ActionRecord {
            action: TrellisAction::Implement,
            updated_at: 100,
        };
        assert!(record.is_fresh(100 + ACTION_TTL_SECONDS));
    }

    #[test]
    fn action_record_expires_after_ttl() {
        let record = ActionRecord {
            action: TrellisAction::Check,
            updated_at: 100,
        };
        assert!(!record.is_fresh(100 + ACTION_TTL_SECONDS + 1));
    }

    #[test]
    fn failed_agent_state_becomes_failed_display() {
        assert_eq!(
            DisplayState::derive("in_progress", AgentState::Failed, None),
            DisplayState::Failed
        );
    }

    #[test]
    fn failed_task_status_becomes_failed_display() {
        assert_eq!(
            DisplayState::derive("failed", AgentState::None, None),
            DisplayState::Failed
        );
    }

    #[test]
    fn failed_task_wins_over_agent_working() {
        assert_eq!(
            DisplayState::derive("failed", AgentState::Working, None),
            DisplayState::Failed
        );
    }

    #[test]
    fn failed_task_wins_over_agent_done() {
        assert_eq!(
            DisplayState::derive("failed", AgentState::Done, None),
            DisplayState::Failed
        );
    }
}
