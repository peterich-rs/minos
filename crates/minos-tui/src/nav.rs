use minos_domain::AgentName;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavLevel {
    Projects,
    Conversations {
        project_id: String,
    },
    Conversation {
        project_id: String,
        conversation_id: String,
    },
    AgentDetail {
        project_id: String,
        conversation_id: String,
        thread_id: String,
        agent: AgentName,
    },
}

impl NavLevel {
    pub fn project_id(&self) -> Option<&str> {
        match self {
            NavLevel::Projects => None,
            NavLevel::Conversations { project_id }
            | NavLevel::Conversation { project_id, .. }
            | NavLevel::AgentDetail { project_id, .. } => Some(project_id),
        }
    }

    pub fn conversation_id(&self) -> Option<&str> {
        match self {
            NavLevel::Projects | NavLevel::Conversations { .. } => None,
            NavLevel::Conversation {
                conversation_id, ..
            }
            | NavLevel::AgentDetail {
                conversation_id, ..
            } => Some(conversation_id),
        }
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
    SubmitConversationInput,
    JumpToProjects,
    JumpToConversations,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_variant_exists() {
        let level = NavLevel::Projects;
        assert!(matches!(level, NavLevel::Projects));
    }
}
