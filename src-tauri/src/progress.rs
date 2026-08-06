// 进度、生长阶段和工作流 lane 计算。
use crate::scan::Subtask;

// status → 无 subtasks 时的默认 progress
fn status_progress(status: &str) -> f64 {
    match status {
        "planning" => 0.1,
        "in_progress" => 0.5,
        "review" => 0.85,
        "completed" => 1.0,
        _ => 0.1,
    }
}

// status → 生长阶段
pub fn growth_stage(status: &str) -> &'static str {
    match status {
        "planning" => "seed",
        "in_progress" => "sprout",
        "review" => "bud",
        "completed" => "bloom",
        _ => "seed",
    }
}

fn is_done(s: &Subtask) -> bool {
    matches!(s.status.as_deref(), Some("completed") | Some("done"))
}

pub fn compute_progress(status: &str, subtasks: &[Subtask]) -> f64 {
    if status == "completed" {
        return 1.0;
    }
    if !subtasks.is_empty() {
        let done = subtasks.iter().filter(|s| is_done(s)).count();
        return done as f64 / subtasks.len() as f64;
    }
    status_progress(status)
}

// status → (lane 0-3, kind)
pub fn lane_model(status: &str) -> (u8, &'static str) {
    match status {
        "completed" | "done" => (3, "done"),
        "review" => (2, "wrap"),
        "in_progress" => (1, "work"),
        "planning" => (0, "plan"),
        "blocked" => (1, "halt"),
        _ => (1, "work"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(name: &str, status: &str) -> Subtask {
        Subtask {
            name: name.into(),
            status: Some(status.into()),
        }
    }

    #[test]
    fn completed_always_one() {
        assert_eq!(compute_progress("completed", &[]), 1.0);
        assert_eq!(compute_progress("completed", &[st("x", "pending")]), 1.0);
    }

    #[test]
    fn subtasks_ratio_counts_completed_and_done() {
        let subs = [
            st("a", "completed"),
            st("b", "done"),
            st("c", "pending"),
            st("d", "in_progress"),
        ];
        assert_eq!(compute_progress("in_progress", &subs), 0.5);
    }

    #[test]
    fn status_mapping_without_subtasks() {
        assert_eq!(compute_progress("planning", &[]), 0.1);
        assert_eq!(compute_progress("in_progress", &[]), 0.5);
        assert_eq!(compute_progress("review", &[]), 0.85);
        assert_eq!(compute_progress("weird", &[]), 0.1);
    }

    #[test]
    fn growth_stage_mapping() {
        assert_eq!(growth_stage("planning"), "seed");
        assert_eq!(growth_stage("in_progress"), "sprout");
        assert_eq!(growth_stage("review"), "bud");
        assert_eq!(growth_stage("completed"), "bloom");
        assert_eq!(growth_stage("weird"), "seed");
    }

    #[test]
    fn lane_mapping() {
        assert_eq!(lane_model("planning"), (0, "plan"));
        assert_eq!(lane_model("in_progress"), (1, "work"));
        assert_eq!(lane_model("review"), (2, "wrap"));
        assert_eq!(lane_model("completed"), (3, "done"));
        assert_eq!(lane_model("blocked"), (1, "halt"));
        assert_eq!(lane_model("weird"), (1, "work"));
    }
}
