//! Public entry point: `detect_all` returns one descriptor per known agent.

use std::sync::Arc;
use std::time::Duration;

use minos_domain::{AgentDescriptor, AgentName, AgentStatus};
use tracing::warn;

use crate::CommandRunner;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

pub async fn detect_all(runner: Arc<dyn CommandRunner>) -> Vec<AgentDescriptor> {
    let mut out = Vec::with_capacity(AgentName::all().len());
    for &name in AgentName::all() {
        out.push(detect_one(&*runner, name).await);
    }
    out
}

async fn detect_one(runner: &dyn CommandRunner, name: AgentName) -> AgentDescriptor {
    let bin = name.bin_name();
    let Some(path) = runner.which(bin).await else {
        return AgentDescriptor {
            name,
            path: None,
            version: None,
            status: AgentStatus::Missing,
        };
    };

    match runner.run(bin, &["--version"], PROBE_TIMEOUT).await {
        Ok(outcome) if outcome.exit_code == 0 => {
            let version = parse_version(&outcome.stdout).or_else(|| parse_version(&outcome.stderr));
            AgentDescriptor {
                name,
                path: Some(path),
                version,
                status: AgentStatus::Ok,
            }
        }
        Ok(outcome) => {
            warn!(
                ?name,
                exit_code = outcome.exit_code,
                "non-zero exit from --version probe"
            );
            AgentDescriptor {
                name,
                path: Some(path),
                version: None,
                status: AgentStatus::Error {
                    reason: format!("exit {}: {}", outcome.exit_code, outcome.stderr.trim()),
                },
            }
        }
        Err(e) => AgentDescriptor {
            name,
            path: Some(path),
            version: None,
            status: AgentStatus::Error {
                reason: e.to_string(),
            },
        },
    }
}

/// Extract the first whitespace-delimited token that looks like a semver.
fn parse_version(s: &str) -> Option<String> {
    s.split_whitespace()
        .find(|tok| tok.chars().next().is_some_and(|c| c.is_ascii_digit()) && tok.contains('.'))
        .map(|tok| tok.trim_matches(',').to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CommandOutcome;
    use minos_domain::MinosError;
    use std::sync::Mutex;

    /// Per-call scripted runner.
    struct ScriptRunner {
        script: Mutex<Vec<(&'static str, ScriptStep)>>,
    }
    enum ScriptStep {
        Which(Option<&'static str>),
        Run(Result<CommandOutcome, MinosError>),
    }

    #[async_trait::async_trait]
    impl CommandRunner for ScriptRunner {
        async fn which(&self, bin: &str) -> Option<String> {
            let mut s = self.script.lock().unwrap();
            let (expected, step) = s.remove(0);
            assert_eq!(expected, bin);
            match step {
                ScriptStep::Which(v) => v.map(String::from),
                _ => panic!("script expected which, got run"),
            }
        }
        async fn run(
            &self,
            bin: &str,
            _args: &[&str],
            _t: Duration,
        ) -> Result<CommandOutcome, MinosError> {
            let mut s = self.script.lock().unwrap();
            let (expected, step) = s.remove(0);
            assert_eq!(expected, bin);
            match step {
                ScriptStep::Run(r) => r,
                _ => panic!("script expected run, got which"),
            }
        }
    }

    fn outcome_ok(stdout: &str) -> ScriptStep {
        ScriptStep::Run(Ok(CommandOutcome {
            exit_code: 0,
            stdout: stdout.to_owned(),
            stderr: String::new(),
        }))
    }

    #[tokio::test]
    async fn missing_bin_yields_missing_status() {
        let runner = Arc::new(ScriptRunner {
            script: Mutex::new(vec![
                ("codex", ScriptStep::Which(None)),
                ("claude", ScriptStep::Which(None)),
                ("gemini", ScriptStep::Which(None)),
            ]),
        });
        let out = detect_all(runner).await;
        assert_eq!(out.len(), 3);
        for d in out {
            assert_eq!(d.status, AgentStatus::Missing);
            assert!(d.path.is_none());
        }
    }

    #[tokio::test]
    async fn version_parsed_from_stdout() {
        let runner = Arc::new(ScriptRunner {
            script: Mutex::new(vec![
                ("codex", ScriptStep::Which(Some("/u/c"))),
                ("codex", outcome_ok("codex 0.18.2\n")),
                ("claude", ScriptStep::Which(None)),
                ("gemini", ScriptStep::Which(None)),
            ]),
        });
        let out = detect_all(runner).await;
        assert_eq!(out[0].status, AgentStatus::Ok);
        assert_eq!(out[0].version.as_deref(), Some("0.18.2"));
        assert_eq!(out[0].path.as_deref(), Some("/u/c"));
    }

    #[tokio::test]
    async fn timeout_yields_error_status() {
        let runner = Arc::new(ScriptRunner {
            script: Mutex::new(vec![
                ("codex", ScriptStep::Which(Some("/u/c"))),
                (
                    "codex",
                    ScriptStep::Run(Err(MinosError::CliProbeTimeout {
                        bin: "codex".into(),
                        timeout_ms: 5000,
                    })),
                ),
                ("claude", ScriptStep::Which(None)),
                ("gemini", ScriptStep::Which(None)),
            ]),
        });
        let out = detect_all(runner).await;
        assert!(matches!(out[0].status, AgentStatus::Error { .. }));
    }
}
