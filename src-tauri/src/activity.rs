use crate::runtime::TrellisAction;

const NATIVE_TO_ACTION: &[(&str, TrellisAction)] = &[
    ("trellis-create", TrellisAction::Create),
    ("trellis-brainstorm", TrellisAction::Brainstorm),
    ("trellis-research", TrellisAction::Research),
    ("trellis-prd", TrellisAction::Prd),
    ("trellis-context", TrellisAction::Context),
    ("trellis-implement", TrellisAction::Implement),
    ("trellis-check", TrellisAction::Check),
    ("trellis-rollback", TrellisAction::Rollback),
    ("trellis-break-loop", TrellisAction::BreakLoop),
    ("trellis-update-spec", TrellisAction::UpdateSpec),
    ("trellis-archive", TrellisAction::Archive),
];

const SKILL_TO_ACTION: &[(&str, TrellisAction)] = &[
    ("trellis-brainstorm", TrellisAction::Brainstorm),
    ("trellis-research", TrellisAction::Research),
    ("trellis-implement", TrellisAction::Implement),
    ("trellis-check", TrellisAction::Check),
    ("trellis-break-loop", TrellisAction::BreakLoop),
    ("trellis-update-spec", TrellisAction::UpdateSpec),
];

fn action_for_task_subcommand(subcommand: &str) -> Option<TrellisAction> {
    match subcommand {
        "create" => Some(TrellisAction::Create),
        "start" | "activate" => Some(TrellisAction::Activate),
        "add-context" | "context" => Some(TrellisAction::Context),
        "archive" | "finish-work" => Some(TrellisAction::Archive),
        "rollback" => Some(TrellisAction::Rollback),
        _ => None,
    }
}

fn shell_segments(command: &str) -> impl Iterator<Item = &str> {
    command.split(['&', '|', ';', '\n', '\r'])
}

fn segment_has_task_script(segment: &str) -> bool {
    segment.split_whitespace().any(|token| {
        let token = token.trim_matches(|c: char| c == '\'' || c == '"');
        let token = crate::platform::to_posix(token);
        token == "task.py" || token.ends_with("/task.py")
    })
}

fn segment_has_script(segment: &str, script: &str) -> bool {
    segment.split_whitespace().any(|token| {
        let token = token.trim_matches(|c: char| c == '\'' || c == '"');
        let token = crate::platform::to_posix(token);
        token == script || token.ends_with(&format!("/{script}"))
    })
}

pub fn classify_tool(tool_name: &str, command: &str, skill: &str) -> Option<TrellisAction> {
    let tool = tool_name.trim();
    if let Some((_, action)) = NATIVE_TO_ACTION.iter().find(|(name, _)| *name == tool) {
        return Some(*action);
    }
    if let Some((_, action)) = SKILL_TO_ACTION
        .iter()
        .find(|(name, _)| *name == skill.trim())
    {
        return Some(*action);
    }
    for segment in shell_segments(command) {
        if segment_has_script(segment, "get_context.py") {
            return Some(TrellisAction::Context);
        }
        if !segment_has_task_script(segment) {
            continue;
        }
        let tokens: Vec<String> = segment
            .split_whitespace()
            .map(|token| {
                let token = token.trim_matches(|c: char| c == '\'' || c == '"');
                crate::platform::to_posix(token)
            })
            .collect();
        for (index, token) in tokens.iter().enumerate() {
            if (token == "task.py" || token.ends_with("/task.py")) && index + 1 < tokens.len() {
                if let Some(action) = action_for_task_subcommand(&tokens[index + 1]) {
                    return Some(action);
                }
            }
        }
    }
    None
}

pub fn extract_task_dir(text: &str) -> Option<String> {
    for raw in text.split_whitespace() {
        let token = raw.trim_matches(|c: char| "'\"()[]{}<>,;".contains(c));
        let token = crate::platform::to_posix(token);
        let components: Vec<&str> = token.split('/').filter(|part| !part.is_empty()).collect();
        for parts in components.windows(3) {
            if parts[0] == ".trellis" && parts[1] == "tasks" && !parts[2].is_empty() {
                return Some(parts[2].to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::TrellisAction;

    #[test]
    fn classifies_native_trellis_tools() {
        assert_eq!(
            classify_tool("trellis-implement", "", ""),
            Some(TrellisAction::Implement)
        );
        assert_eq!(
            classify_tool("trellis-check", "", ""),
            Some(TrellisAction::Check)
        );
    }

    #[test]
    fn classifies_task_py_subcommands() {
        assert_eq!(
            classify_tool("Bash", "task.py start 07-demo", ""),
            Some(TrellisAction::Activate)
        );
        assert_eq!(
            classify_tool(
                "Bash",
                "python3 ./.trellis/scripts/task.py add-context x",
                ""
            ),
            Some(TrellisAction::Context)
        );
    }

    #[test]
    fn classifies_get_context_script() {
        assert_eq!(
            classify_tool(
                "Bash",
                "python3 ./.trellis/scripts/get_context.py --mode phase --step 1.1",
                ""
            ),
            Some(TrellisAction::Context)
        );
    }

    #[test]
    fn ignores_incidental_mentions() {
        assert_eq!(classify_tool("Bash", "rg trellis README.md", ""), None);
        assert_eq!(classify_tool("Bash", "echo trellis", ""), None);
    }

    #[test]
    fn recognizes_trellis_task_paths() {
        assert_eq!(
            extract_task_dir("write /repo/.trellis/tasks/07-demo/prd.md"),
            Some("07-demo".into())
        );
    }
}
