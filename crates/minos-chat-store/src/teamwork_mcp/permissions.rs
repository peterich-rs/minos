#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamworkMcpPermission {
    ListConversationMessages,
    ListConversationRoster,
    DelegateToAgent,
    GetDelegationStatus,
    WaitDelegation,
    CancelDelegation,
    PostConversationUpdate,
    PostGitUpdate,
    ReactToMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamworkMcpPermissions {
    pub list_conversation_messages: bool,
    pub list_conversation_roster: bool,
    pub delegate_to_agent: bool,
    pub get_delegation_status: bool,
    pub wait_delegation: bool,
    pub cancel_delegation: bool,
    pub post_conversation_update: bool,
    pub post_git_update: bool,
    pub react_to_message: bool,
}

impl TeamworkMcpPermissions {
    pub fn allows(self, permission: TeamworkMcpPermission) -> bool {
        match permission {
            TeamworkMcpPermission::ListConversationMessages => self.list_conversation_messages,
            TeamworkMcpPermission::ListConversationRoster => self.list_conversation_roster,
            TeamworkMcpPermission::DelegateToAgent => self.delegate_to_agent,
            TeamworkMcpPermission::GetDelegationStatus => self.get_delegation_status,
            TeamworkMcpPermission::WaitDelegation => self.wait_delegation,
            TeamworkMcpPermission::CancelDelegation => self.cancel_delegation,
            TeamworkMcpPermission::PostConversationUpdate => self.post_conversation_update,
            TeamworkMcpPermission::PostGitUpdate => self.post_git_update,
            TeamworkMcpPermission::ReactToMessage => self.react_to_message,
        }
    }
}

impl Default for TeamworkMcpPermissions {
    fn default() -> Self {
        Self {
            list_conversation_messages: true,
            list_conversation_roster: true,
            delegate_to_agent: true,
            get_delegation_status: true,
            wait_delegation: true,
            cancel_delegation: true,
            post_conversation_update: true,
            post_git_update: true,
            react_to_message: true,
        }
    }
}
