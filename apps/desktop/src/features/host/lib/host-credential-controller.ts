/**
 * Serial host-credential controller.
 *
 * clear / apply must never interleave: a deferred account-A register must not
 * write hit_ after account-B already owns the daemon, and leave must close the
 * authorization boundary before the next account registers.
 */

let credentialGeneration = 0;
let opChain: Promise<void> = Promise.resolve();

export function getHostCredentialGeneration(): number {
  return credentialGeneration;
}

/** Bump generation so in-flight register/apply abort before writing hit_. */
export function bumpHostCredentialGeneration(): number {
  credentialGeneration += 1;
  return credentialGeneration;
}

/**
 * Enqueue a credential mutation. Operations run strictly in order.
 * Callers must re-check generation inside `op` before side effects.
 */
export function enqueueHostCredentialOp<T>(
  op: (generation: number) => Promise<T>,
): Promise<T> {
  const genAtEnqueue = credentialGeneration;
  const run = opChain.catch(() => undefined).then(() => op(genAtEnqueue));
  opChain = run.then(
    () => undefined,
    () => undefined,
  );
  return run;
}

/**
 * Account leave/switch: invalidate in-flight ensure and clear daemon hit_.
 * Clear is serialized so a subsequent register waits for it.
 */
export function revokeHostCredential(clear: () => Promise<void>): void {
  bumpHostCredentialGeneration();
  void enqueueHostCredentialOp(async () => {
    try {
      await clear();
    } catch {
      /* daemon may be offline; next login force-registers */
    }
  });
}

/** True when the captured ensure generation is still current. */
export function isHostCredentialCurrent(generation: number): boolean {
  return generation === credentialGeneration;
}

/** Test helper — reset serial chain + generation. */
export function resetHostCredentialControllerForTests(): void {
  credentialGeneration = 0;
  opChain = Promise.resolve();
}
