import { spawn } from "node:child_process";
import readline from "node:readline";

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Message = { id?: number | string; method?: string; params?: Json; result?: Json; error?: { code: number; message: string; data?: Json } };

class CodexAppServerClient {
  private proc = spawn("codex", ["app-server"], { stdio: ["pipe", "pipe", "inherit"] });
  private rl = readline.createInterface({ input: this.proc.stdout });
  private nextId = 1;
  private pending = new Map<number, { resolve: (value: Message) => void; reject: (error: Error) => void }>();

  constructor() {
    this.rl.on("line", (line) => this.onLine(line));
    this.proc.on("exit", (code, signal) => {
      const err = new Error(`codex app-server exited code=${code} signal=${signal}`);
      for (const { reject } of this.pending.values()) reject(err);
      this.pending.clear();
    });
  }

  async initialize() {
    await this.request("initialize", {
      clientInfo: { name: "my_client", title: "My Client", version: "0.1.0" },
    });
    this.notify("initialized", {});
  }

  request(method: string, params: Json = {}): Promise<Message> {
    const id = this.nextId++;
    this.send({ method, id, params });
    return new Promise((resolve, reject) => this.pending.set(id, { resolve, reject }));
  }

  notify(method: string, params: Json = {}) {
    this.send({ method, params });
  }

  private send(message: Message) {
    // Do not include a "jsonrpc" field. App-server omits the JSON-RPC 2.0 header on the wire.
    this.proc.stdin.write(`${JSON.stringify(message)}\n`);
  }

  private onLine(line: string) {
    const msg = JSON.parse(line) as Message;
    if (msg.id !== undefined && (msg.result !== undefined || msg.error !== undefined)) {
      const id = Number(msg.id);
      const pending = this.pending.get(id);
      if (pending) {
        this.pending.delete(id);
        msg.error ? pending.reject(new Error(msg.error.message)) : pending.resolve(msg);
      }
      return;
    }
    if (msg.method) this.onNotification(msg.method, msg.params);
  }

  private onNotification(method: string, params: Json | undefined) {
    // Route turn/*, item/*, serverRequest/resolved, approvals, and fs/changed here.
    console.log("notification", method, params);
  }
}

async function main() {
  const client = new CodexAppServerClient();
  await client.initialize();
  const threadResp = await client.request("thread/start", { model: "gpt-5.4" });
  const thread = (threadResp.result as any).thread;
  await client.request("turn/start", {
    threadId: thread.id,
    input: [{ type: "text", text: "Summarize this repo." }],
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
