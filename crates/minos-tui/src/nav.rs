use minos_domain::AgentName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavLevel {
    Projects,
    Sessions {
        project_id: String,
    },
    Session {
        project_id: String,
        thread_id: String,
    },
    AgentDetail {
        project_id: String,
        thread_id: String,
        agent: AgentName,
    },
}

impl NavLevel {
    pub fn go_up(&self) -> NavLevel {
        match self {
            NavLevel::Projects => NavLevel::Projects,
            NavLevel::Sessions { .. } => NavLevel::Projects,
            NavLevel::Session { project_id, .. } => {
                NavLevel::Sessions { project_id: project_id.clone() }
            }
            NavLevel::AgentDetail { project_id, thread_id, .. } => NavLevel::Session {
                project_id: project_id.clone(),
                thread_id: thread_id.clone(),
            },
        }
    }

    pub fn project_id(&self) -> Option<&str> {
        match self {
            NavLevel::Projects => None,
            NavLevel::Sessions { project_id }
            | NavLevel::Session { project_id, .. }
            | NavLevel::AgentDetail { project_id, .. } => Some(project_id),
        }
    }

    pub fn thread_id(&self) -> Option<&str> {
        match self {
            NavLevel::Projects | NavLevel::Sessions { .. } => None,
            NavLevel::Session { thread_id, .. }
            | NavLevel::AgentDetail { thread_id, .. } => Some(thread_id),
        }
    }

    pub fn esc_quits(&self) -> bool {
        matches!(self, NavLevel::Projects)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavAction {
    Downlevel,
    Uplevel,
    SelectNext,
    SelectPrev,
    OpenCreateProject,
    ConfirmCreateProject,
    CancelDialog,
    SwitchField,
    TypeChar(char),
    Backspace,
    DismissStartupPrompt,
    AcceptStartupPrompt,
    SubmitSessionInput,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_up_from_sessions_to_projects() {
        let level = NavLevel::Sessions { project_id: "p1".into() };
        assert_eq!(level.go_up(), NavLevel::Projects);
    }

    #[test]
    fn go_up_from_session_to_sessions() {
        let level = NavLevel::Session {
            project_id: "p1".into(),
            thread_id: "t1".into(),
        };
        assert_eq!(
            level.go_up(),
            NavLevel::Sessions { project_id: "p1".into() }
        );
    }

    #[test]
    fn go_up_at_projects_stays() {
        assert_eq!(NavLevel::Projects.go_up(), NavLevel::Projects);
    }

    #[test]
    fn esc_quits_only_at_projects() {
        assert!(NavLevel::Projects.esc_quits());
        assert!(!NavLevel::Sessions { project_id: "p".into() }.esc_quits());
    }
}
