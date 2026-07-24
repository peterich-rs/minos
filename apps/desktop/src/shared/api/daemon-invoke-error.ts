export class DaemonInvokeError extends Error {
  readonly command: string;
  readonly cause?: unknown;

  constructor(message: string, command: string, cause?: unknown) {
    super(message);
    this.name = "DaemonInvokeError";
    this.command = command;
    this.cause = cause;
  }
}
