import { create } from "zustand";

export type ProjectView = "conversations" | "sessions" | "board";
export type PrimaryNav = "work" | "attention" | "agents" | "host";

type UiState = {
  primaryNav: PrimaryNav;
  projectId: string;
  conversationId: string | null;
  projectView: ProjectView;
  /** Selected agent session (for inspector and Sessions tab). */
  selectedSessionId: string | null;
  detailsOpen: boolean;
  conversationListCollapsed: boolean;
  sessionsListCollapsed: boolean;
  /** Composer draft keyed by conversation (not shared across conversations). */
  draftByConversationId: Record<string, string>;
  /** Last conversation selected per project (restore on project switch). */
  lastConversationByProject: Record<string, string>;

  setPrimaryNav: (nav: PrimaryNav) => void;
  selectProject: (projectId: string) => void;
  selectConversation: (conversationId: string | null) => void;
  setProjectView: (view: ProjectView) => void;
  selectSession: (sessionId: string | null) => void;
  /** Jump to Sessions tab and open transcript for session. */
  openSessionTranscript: (sessionId: string, conversationId?: string | null) => void;
  /** Jump back to Conversations tab for a conversation. */
  openConversation: (conversationId: string) => void;
  toggleDetails: () => void;
  toggleConversationList: () => void;
  toggleSessionsList: () => void;
  setDraft: (conversationId: string, value: string) => void;
};

export const useUiStore = create<UiState>((set) => ({
  primaryNav: "work",
  projectId: "",
  conversationId: null,
  projectView: "conversations",
  selectedSessionId: null,
  detailsOpen: true,
  conversationListCollapsed: false,
  sessionsListCollapsed: false,
  draftByConversationId: {},
  lastConversationByProject: {},

  setPrimaryNav: (nav) => set({ primaryNav: nav }),
  selectProject: (projectId) => {
    set((s) => {
      const restored = s.lastConversationByProject[projectId] ?? null;
      return {
        projectId,
        conversationId: restored,
        selectedSessionId: null,
        projectView: "conversations",
        primaryNav: "work",
        conversationListCollapsed: false,
        sessionsListCollapsed: false,
      };
    });
  },
  selectConversation: (conversationId) => {
    set((s) => ({
      conversationId,
      selectedSessionId: null,
      projectView: "conversations",
      primaryNav: "work",
      lastConversationByProject:
        conversationId && s.projectId
          ? {
              ...s.lastConversationByProject,
              [s.projectId]: conversationId,
            }
          : s.lastConversationByProject,
    }));
  },
  setProjectView: (projectView) =>
    set({
      projectView,
    }),
  selectSession: (selectedSessionId) => set({ selectedSessionId }),
  openSessionTranscript: (sessionId, conversationId) =>
    set((s) => ({
      primaryNav: "work",
      projectView: "sessions",
      selectedSessionId: sessionId,
      conversationId: conversationId ?? s.conversationId,
      sessionsListCollapsed: false,
    })),
  openConversation: (conversationId) =>
    set((s) => ({
      primaryNav: "work",
      projectView: "conversations",
      conversationId,
      selectedSessionId: null,
      conversationListCollapsed: false,
      lastConversationByProject: s.projectId
        ? {
            ...s.lastConversationByProject,
            [s.projectId]: conversationId,
          }
        : s.lastConversationByProject,
    })),
  toggleDetails: () => set((s) => ({ detailsOpen: !s.detailsOpen })),
  toggleConversationList: () =>
    set((s) => ({ conversationListCollapsed: !s.conversationListCollapsed })),
  toggleSessionsList: () =>
    set((s) => ({ sessionsListCollapsed: !s.sessionsListCollapsed })),
  setDraft: (conversationId, draft) =>
    set((s) => ({
      draftByConversationId: {
        ...s.draftByConversationId,
        [conversationId]: draft,
      },
    })),
}));
