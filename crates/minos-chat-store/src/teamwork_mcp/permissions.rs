#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeamworkMcpPermission {
    ListRoomMessages,
    DelegateToAgent,
    GetDelegationStatus,
    CancelDelegation,
    AskUserQuestion,
    CheckUserFeedback,
    PostRoomUpdate,
    ReactToMessage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TeamworkMcpPermissions {
    pub list_room_messages: bool,
    pub delegate_to_agent: bool,
    pub get_delegation_status: bool,
    pub cancel_delegation: bool,
    pub ask_user_question: bool,
    pub check_user_feedback: bool,
    pub post_room_update: bool,
    pub react_to_message: bool,
}

impl TeamworkMcpPermissions {
    pub fn allows(self, permission: TeamworkMcpPermission) -> bool {
        match permission {
            TeamworkMcpPermission::ListRoomMessages => self.list_room_messages,
            TeamworkMcpPermission::DelegateToAgent => self.delegate_to_agent,
            TeamworkMcpPermission::GetDelegationStatus => self.get_delegation_status,
            TeamworkMcpPermission::CancelDelegation => self.cancel_delegation,
            TeamworkMcpPermission::AskUserQuestion => self.ask_user_question,
            TeamworkMcpPermission::CheckUserFeedback => self.check_user_feedback,
            TeamworkMcpPermission::PostRoomUpdate => self.post_room_update,
            TeamworkMcpPermission::ReactToMessage => self.react_to_message,
        }
    }
}

impl Default for TeamworkMcpPermissions {
    fn default() -> Self {
        Self {
            list_room_messages: true,
            delegate_to_agent: true,
            get_delegation_status: true,
            cancel_delegation: true,
            ask_user_question: true,
            check_user_feedback: true,
            post_room_update: true,
            react_to_message: true,
        }
    }
}
