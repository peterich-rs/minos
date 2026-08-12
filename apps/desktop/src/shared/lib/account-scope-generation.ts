/**
 * Process-local generation for account-scoped async writers.
 * Bumped on leave/switch so mid-flight work can no-op at setState time.
 */

let accountScopeGeneration = 0;

export function getAccountScopeGeneration(): number {
  return accountScopeGeneration;
}

export function bumpAccountScopeGeneration(): number {
  accountScopeGeneration += 1;
  return accountScopeGeneration;
}
