/**
 * Minimal Grok ACP stdio client skeleton (TypeScript / Node).
 * Spawn: grok agent --no-leader stdio
 */
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import * as readline from "node:readline";

type Json = Record<string, unknown>;

export class GrokAcpClient {
  private proc: ChildProcessWithoutNullStreams;
  private rl: readline.Interface;
  private nextId = 1;
  private pending = new Map<
    number,
    { resolve: (v: unknown) => void; reject: (e: Error) => void }
  >();
  private onNotification: (method: string, params: unknown) => void;

  constructor(
    cwd = process.cwd(),
    onNotification: (method: string, params: unknown) => void = () => {},
  ) {
    this.onNotification = onNotification;
    this.proc = spawn("grok", ["agent", "--no-leader", "stdio"], {
      cwd,
      stdio: ["pipe", "pipe", "inherit"],
    });
    this.rl = readline.createInterface({ input: this.proc.stdout });
    this.rl.on("line", (line) => this.onLine(line));
  }

  private onLine(line: string) {
    let msg: Json;
    try {
      msg = JSON.parse(line) as Json;
    } catch {
      return;
    }
    if (typeof msg.id !== "undefined" && (msg.result !== undefined || msg.error)) {
      const id = Number(msg.id);
      const p = this.pending.get(id);
      if (!p) return;
      this.pending.delete(id);
      if (msg.error) p.reject(new Error(JSON.stringify(msg.error)));
      else p.resolve(msg.result);
      return;
    }
    if (typeof msg.method === "string") {
      this.onNotification(msg.method, msg.params);
    }
  }

  private request(method: string, params: unknown): Promise<unknown> {
    const id = this.nextId++;
    const frame = JSON.stringify({ jsonrpc: "2.0", id, method, params });
    return new Promise((resolve, reject) => {
      this.pending.set(id, { resolve, reject });
      this.proc.stdin.write(frame + "\n");
    });
  }

  async initialize() {
    return this.request("initialize", {
      protocolVersion: 1,
      clientCapabilities: {
        fs: { readTextFile: false, writeTextFile: false },
        terminal: false,
      },
      clientInfo: { name: "minos-example", version: "0.0.0" },
    });
  }

  async newSession(cwd: string) {
    return this.request("session/new", {
      cwd,
      mcpServers: [],
    }) as Promise<{ sessionId: string }>;
  }

  async prompt(sessionId: string, text: string) {
    return this.request("session/prompt", {
      sessionId,
      prompt: [{ type: "text", text }],
    });
  }

  close() {
    this.proc.kill();
  }
}

async function main() {
  const client = new GrokAcpClient(process.cwd(), (method, params) => {
    if (method === "session/update") {
      const update = (params as Json)?.update as Json | undefined;
      const kind = update?.sessionUpdate;
      if (kind === "agent_message_chunk") {
        const text = (update?.content as Json)?.text;
        if (typeof text === "string") process.stdout.write(text);
      } else {
        console.error("[update]", kind, JSON.stringify(update));
      }
    } else {
      console.error("[notif]", method, JSON.stringify(params));
    }
  });

  await client.initialize();
  const { sessionId } = await client.newSession(process.cwd());
  await client.prompt(sessionId, "Say hi in one sentence.");
  client.close();
}

if (require.main === module) {
  main().catch((err) => {
    console.error(err);
    process.exit(1);
  });
}
