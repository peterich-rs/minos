import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

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
  commandPaletteOpen: boolean;
  setCommandPaletteOpen: (open: boolean) => void;
};

export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
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
          if (s.projectId === projectId) {
            // Same project re-click: keep view/session selection.
            return {
              projectId,
              primaryNav: "work" as const,
            };
          }
          const restored = s.lastConversationByProject[projectId] ?? null;
          return {
            projectId,
            conversationId: restored,
            // Always clear session when leaving a project — Sessions tab must not
            // keep rendering the previous project's transcript.
            selectedSessionId: null,
            // Preserve sessions tab if user was already there; otherwise conversations.
            projectView:
              s.projectView === "sessions" || s.projectView === "board"
                ? s.projectView
                : ("conversations" as const),
            primaryNav: "work" as const,
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
      commandPaletteOpen: false,
      setCommandPaletteOpen: (commandPaletteOpen) => set({ commandPaletteOpen }),
    }),
    {
      name: "minos.ui-store.v1",
      storage: createJSONStorage(() => localStorage),
      partialize: (s) => ({
        projectId: s.projectId,
        conversationId: s.conversationId,
        lastConversationByProject: s.lastConversationByProject,
      }),
    },
  ),
);
