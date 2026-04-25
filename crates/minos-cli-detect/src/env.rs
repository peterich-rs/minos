//! User-shell env capture. The macOS app process inherits launchd's minimal
//! `PATH`, so subprocesses spawned by the daemon (detection probes, codex
//! itself) need an env that mirrors what the user sees in their terminal.
//!
//! `capture_user_shell_env()` runs `$SHELL -lic '<dump>'` once at daemon
//! bootstrap. The output is bracketed by control-char sentinels so we can
//! discard rc-script noise; values are NUL-separated via `env -0` so values
//! containing newlines parse correctly.

use std::collections::HashMap;
use std::path::Path;
use std::time::Duration;

use tokio::process::Command;
use tokio::time::timeout;
use tracing::warn;

/// Volatile shell-session keys we strip — they describe the temporary login
/// shell, not anything the user would expect their CLI tools to inherit.
const FILTER: &[&str] = &["_", "SHLVL", "PWD", "OLDPWD"];

const BEGIN: &str = "\x01MINOS_ENV_BEGIN\x01";
const END: &str = "\x01MINOS_ENV_END\x01";

const SHELL_TIMEOUT: Duration = Duration::from_secs(3);
const FALLBACK_SHELL: &str = "/bin/zsh";

/// Shell-side dump script. Brackets `env -0` output with control-char
/// sentinels so the parser can discard rc-script chatter on stdout.
/// `\1` is octal-escape for `\x01`, supported by every printf we care about.
const DUMP_SCRIPT: &str = "printf '\\1MINOS_ENV_BEGIN\\1'; env -0; printf '\\1MINOS_ENV_END\\1'";

/// Pure parser: given the raw stdout of the dump script, slice between
/// sentinels and split into `(key, value)` pairs. Returns an empty map if
/// the sentinels are missing.
pub(crate) fn parse_env_dump(stdout: &str) -> HashMap<String, String> {
    let Some(begin_at) = stdout.find(BEGIN) else {
        return HashMap::new();
    };
    let body_start = begin_at + BEGIN.len();
    let Some(end_offset) = stdout[body_start..].find(END) else {
        return HashMap::new();
    };
    let body = &stdout[body_start..body_start + end_offset];

    body.split('\0')
        .filter_map(|entry| {
            let (k, v) = entry.split_once('=')?;
            if k.is_empty() || FILTER.contains(&k) {
                return None;
            }
            Some((k.to_owned(), v.to_owned()))
        })
        .collect()
}

/// Run `$SHELL -lic '<dump>'` once and return the parsed env map. Any
/// failure (timeout, spawn error, non-zero exit, missing sentinels) logs
/// at `warn` level and returns the current process's env via
/// `std::env::vars()`. This function never panics and never returns Err —
/// daemon bootstrap must not be blocked by a broken user shell rc.
pub async fn capture_user_shell_env() -> HashMap<String, String> {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|p| Path::new(p).is_absolute())
        .unwrap_or_else(|| FALLBACK_SHELL.to_owned());

    let fut = Command::new(&shell)
        .args(["-l", "-i", "-c", DUMP_SCRIPT])
        .output();

    match timeout(SHELL_TIMEOUT, fut).await {
        Ok(Ok(out)) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let map = parse_env_dump(&stdout);
            if map.is_empty() {
                warn!(
                    shell = %shell,
                    "shell env dump produced no parseable entries; falling back to process env"
                );
                std::env::vars().collect()
            } else {
                map
            }
        }
        Ok(Ok(out)) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            warn!(
                shell = %shell,
                exit = ?out.status.code(),
                stderr_first_line = %stderr.lines().next().unwrap_or(""),
                "shell env dump exited non-zero; falling back to process env",
            );
            std::env::vars().collect()
        }
        Ok(Err(e)) => {
            warn!(
                shell = %shell,
                error = %e,
                "shell env dump spawn failed; falling back to process env",
            );
            std::env::vars().collect()
        }
        Err(_) => {
            warn!(
                shell = %shell,
                timeout_secs = SHELL_TIMEOUT.as_secs(),
                "shell env dump timed out; falling back to process env",
            );
            std::env::vars().collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_input_yields_empty_map() {
        assert!(parse_env_dump("").is_empty());
    }

    #[test]
    fn missing_sentinels_yields_empty_map() {
        assert!(parse_env_dump("PATH=/usr/bin\0HOME=/home/u\0").is_empty());
    }

    #[test]
    fn parses_minimal_dump() {
        let s = "\x01MINOS_ENV_BEGIN\x01PATH=/usr/bin\0HOME=/home/u\0\x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(map.get("HOME").map(String::as_str), Some("/home/u"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn rc_script_noise_is_discarded() {
        let s = "Welcome to zsh\nLast login: ...\n\
                 \x01MINOS_ENV_BEGIN\x01PATH=/usr/bin\0\x01MINOS_ENV_END\x01\n\
                 trailing garbage";
        let map = parse_env_dump(s);
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(map.len(), 1);
    }

    #[test]
    fn volatile_keys_are_filtered() {
        let s = "\x01MINOS_ENV_BEGIN\x01\
                 PATH=/usr/bin\0_=/usr/bin/zsh\0SHLVL=2\0PWD=/tmp\0OLDPWD=/home/u\0HOME=/home/u\0\
                 \x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(map.get("HOME").map(String::as_str), Some("/home/u"));
        assert!(!map.contains_key("_"));
        assert!(!map.contains_key("SHLVL"));
        assert!(!map.contains_key("PWD"));
        assert!(!map.contains_key("OLDPWD"));
        assert_eq!(map.len(), 2);
    }

    #[test]
    fn value_with_equals_is_preserved() {
        let s = "\x01MINOS_ENV_BEGIN\x01\
                 RUSTFLAGS=-C link-arg=-Wl,-rpath,/opt/lib\0PATH=/usr/bin\0\
                 \x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(
            map.get("RUSTFLAGS").map(String::as_str),
            Some("-C link-arg=-Wl,-rpath,/opt/lib"),
        );
        assert_eq!(map.get("PATH").map(String::as_str), Some("/usr/bin"));
    }

    #[test]
    fn empty_key_entries_are_dropped() {
        // Trailing NUL → empty entry between END marker and last NUL.
        let s = "\x01MINOS_ENV_BEGIN\x01PATH=/usr/bin\0\0\x01MINOS_ENV_END\x01";
        let map = parse_env_dump(s);
        assert_eq!(map.len(), 1);
    }
}
