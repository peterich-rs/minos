import assert from "node:assert/strict";
import { describe, it } from "node:test";
import type { TranscriptItem } from "../../../shared/lib/daemon.ts";
import { handleUserAction } from "./user-action.ts";

function item(
  partial: Partial<TranscriptItem> & Pick<TranscriptItem, "id">,
): TranscriptItem {
  return {
    kind: "approval",
    role: null,
    text: "",
    tsMs: 0,
    seq: 1,
    ...partial,
  };
}

function mockApis() {
  const resolveApproval: Array<
    [string, string, string | Record<string, unknown>]
  > = [];
  const respondOpencodePermission: Array<[string, string, string]> = [];
  const respondOpencodeQuestion: Array<[string, string, string[][]]> = [];
  return {
    calls: {
      resolveApproval,
      respondOpencodePermission,
      respondOpencodeQuestion,
    },
    apis: {
      resolveApproval: async (
        sessionId: string,
        requestId: string,
        decision: string | Record<string, unknown>,
      ) => {
        resolveApproval.push([sessionId, requestId, decision]);
      },
      respondOpencodePermission: async (
        sessionId: string,
        permissionId: string,
        response: string,
      ) => {
        respondOpencodePermission.push([sessionId, permissionId, response]);
      },
      respondOpencodeQuestion: async (
        sessionId: string,
        questionId: string,
        answers: string[][],
      ) => {
        respondOpencodeQuestion.push([sessionId, questionId, answers]);
      },
    },
  };
}

describe("handleUserAction", () => {
  it("no-ops without requestId", async () => {
    const { apis, calls } = mockApis();
    await handleUserAction(
      "sess",
      item({ id: "t1", requestId: null }),
      { type: "decision", decision: "approve" },
      apis,
    );
    assert.deepEqual(calls.resolveApproval, []);
    assert.deepEqual(calls.respondOpencodePermission, []);
    assert.deepEqual(calls.respondOpencodeQuestion, []);
  });

  it("maps opencode/permission approve-like decisions to approveResponse", async () => {
    const { apis, calls } = mockApis();
    for (const decision of ["approve", "allow", "yes"] as const) {
      calls.respondOpencodePermission.length = 0;
      await handleUserAction(
        "sess",
        item({
          id: "t1",
          requestId: "req-1",
          approvalMethod: "opencode/permission",
          approveResponse: "accept",
          declineResponse: "reject",
        }),
        { type: "decision", decision },
        apis,
      );
      assert.deepEqual(calls.respondOpencodePermission, [
        ["sess", "req-1", "accept"],
      ]);
    }
  });

  it("maps opencode/permission other actions to declineResponse", async () => {
    const { apis, calls } = mockApis();
    await handleUserAction(
      "sess",
      item({
        id: "t1",
        requestId: "req-1",
        approvalMethod: "opencode/permission",
        approveResponse: "accept",
        declineResponse: "reject",
      }),
      { type: "decision", decision: "deny" },
      apis,
    );
    assert.deepEqual(calls.respondOpencodePermission, [
      ["sess", "req-1", "reject"],
    ]);

    calls.respondOpencodePermission.length = 0;
    await handleUserAction(
      "sess",
      item({
        id: "t1",
        requestId: "req-1",
        approvalMethod: "opencode/permission",
      }),
      { type: "cancel" },
      apis,
    );
    assert.deepEqual(calls.respondOpencodePermission, [
      ["sess", "req-1", "reject"],
    ]);
  });

  it("handles opencode/question cancel / answers / decision shapes", async () => {
    const { apis, calls } = mockApis();
    const base = item({
      id: "t1",
      requestId: "q-1",
      approvalMethod: "opencode/question",
    });

    await handleUserAction("sess", base, { type: "cancel" }, apis);
    assert.deepEqual(calls.respondOpencodeQuestion, [["sess", "q-1", [[]]]]);

    calls.respondOpencodeQuestion.length = 0;
    await handleUserAction(
      "sess",
      base,
      { type: "answers", answers: [["a"], ["b", "c"]] },
      apis,
    );
    assert.deepEqual(calls.respondOpencodeQuestion, [
      ["sess", "q-1", [["a"], ["b", "c"]]],
    ]);

    calls.respondOpencodeQuestion.length = 0;
    await handleUserAction(
      "sess",
      base,
      { type: "decision", decision: "option-1" },
      apis,
    );
    assert.deepEqual(calls.respondOpencodeQuestion, [
      ["sess", "q-1", [["option-1"]]],
    ]);
  });

  it("handles x.ai/ask_user_question cancel / answers / decision shapes", async () => {
    const { apis, calls } = mockApis();
    const base = item({
      id: "t1",
      requestId: "q-2",
      approvalMethod: "x.ai/ask_user_question",
    });

    await handleUserAction("sess", base, { type: "cancel" }, apis);
    assert.deepEqual(calls.resolveApproval, [
      ["sess", "q-2", { outcome: "cancelled" }],
    ]);

    calls.resolveApproval.length = 0;
    await handleUserAction(
      "sess",
      base,
      { type: "answers", answers: [["yes"], [], ["no"]] },
      apis,
    );
    assert.deepEqual(calls.resolveApproval, [
      [
        "sess",
        "q-2",
        {
          outcome: "accepted",
          answers: { "0": ["yes"], "2": ["no"] },
        },
      ],
    ]);

    calls.resolveApproval.length = 0;
    await handleUserAction(
      "sess",
      base,
      { type: "decision", decision: "ship-it" },
      apis,
    );
    assert.deepEqual(calls.resolveApproval, [
      [
        "sess",
        "q-2",
        {
          outcome: "accepted",
          answers: { "0": ["ship-it"] },
        },
      ],
    ]);
  });

  it("routes generic decision/cancel through resolveApproval", async () => {
    const { apis, calls } = mockApis();
    const base = item({
      id: "t1",
      requestId: "req-g",
      approvalMethod: "session/request_permission",
    });

    await handleUserAction(
      "sess",
      base,
      { type: "decision", decision: "allow-once" },
      apis,
    );
    assert.deepEqual(calls.resolveApproval, [["sess", "req-g", "allow-once"]]);

    calls.resolveApproval.length = 0;
    await handleUserAction("sess", base, { type: "cancel" }, apis);
    assert.deepEqual(calls.resolveApproval, [["sess", "req-g", "deny"]]);
  });
});
