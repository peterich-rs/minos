import type { TranscriptItem } from "@/shared/lib/daemon";

export type UserAction =
  | { type: "decision"; decision: string }
  | { type: "answers"; answers: string[][] }
  | { type: "cancel" };

export async function handleUserAction(
  sessionId: string,
  item: TranscriptItem,
  action: UserAction,
  apis: {
    resolveApproval: (
      sessionId: string,
      requestId: string,
      decision: string | Record<string, unknown>,
    ) => Promise<void>;
    respondOpencodePermission: (
      sessionId: string,
      permissionId: string,
      response: string,
    ) => Promise<void>;
    respondOpencodeQuestion: (
      sessionId: string,
      questionId: string,
      answers: string[][],
    ) => Promise<void>;
  },
) {
  const requestId = item.requestId;
  if (!requestId) return;
  const method = item.approvalMethod ?? "";

  if (method === "opencode/permission") {
    const token =
      action.type === "decision" &&
      (action.decision === "approve" ||
        action.decision === "allow" ||
        action.decision === "yes")
        ? (item.approveResponse ?? "accept")
        : (item.declineResponse ?? "reject");
    await apis.respondOpencodePermission(sessionId, requestId, token);
    return;
  }

  if (method === "opencode/question") {
    if (action.type === "cancel") {
      await apis.respondOpencodeQuestion(sessionId, requestId, [[]]);
      return;
    }
    if (action.type === "answers") {
      await apis.respondOpencodeQuestion(sessionId, requestId, action.answers);
      return;
    }
    if (action.type === "decision") {
      await apis.respondOpencodeQuestion(sessionId, requestId, [
        [action.decision],
      ]);
    }
    return;
  }

  if (method === "x.ai/ask_user_question") {
    if (action.type === "cancel") {
      await apis.resolveApproval(sessionId, requestId, {
        outcome: "cancelled",
      });
      return;
    }
    if (action.type === "answers") {
      const map: Record<string, string[]> = {};
      action.answers.forEach((a, i) => {
        if (a.length) map[String(i)] = a;
      });
      await apis.resolveApproval(sessionId, requestId, {
        outcome: "accepted",
        answers: map,
      });
      return;
    }
    if (action.type === "decision") {
      await apis.resolveApproval(sessionId, requestId, {
        outcome: "accepted",
        answers: { "0": [action.decision] },
      });
    }
    return;
  }

  // Plan / ACP permission / generic approval.
  if (action.type === "decision") {
    await apis.resolveApproval(sessionId, requestId, action.decision);
  } else if (action.type === "cancel") {
    await apis.resolveApproval(sessionId, requestId, "deny");
  }
}
