//! Git author identity checks (user.name / user.email).

use std::path::Path;

use super::exec::run_git;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitIdentity {
    pub name: Option<String>,
    pub email: Option<String>,
}

impl GitIdentity {
    pub fn is_complete(&self) -> bool {
        self.name
            .as_ref()
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false)
            && self
                .email
                .as_ref()
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
    }

    pub fn ensure_complete(&self) -> Result<(), String> {
        if self.is_complete() {
            Ok(())
        } else {
            Err(
                "git user.name / user.email are not configured; set them before commit or push"
                    .into(),
            )
        }
    }
}

/// Resolve identity with local repo config overriding global.
pub fn read_identity(repo: &Path) -> GitIdentity {
    let name = run_git(repo, &["config", "--get", "user.name"])
        .ok()
        .filter(|s| !s.is_empty());
    let email = run_git(repo, &["config", "--get", "user.email"])
        .ok()
        .filter(|s| !s.is_empty());
    GitIdentity { name, email }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    #[test]
    fn incomplete_identity_rejected() {
        let id = GitIdentity {
            name: Some("a".into()),
            email: None,
        };
        assert!(!id.is_complete());
        assert!(id.ensure_complete().is_err());
    }

    #[test]
    fn reads_local_config() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let st = Command::new("git")
            .args(["init", "-b", "main"])
            .current_dir(root)
            .status()
            .unwrap();
        assert!(st.success());
        Command::new("git")
            .args(["config", "user.name", "Minos Test"])
            .current_dir(root)
            .status()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "minos@example.com"])
            .current_dir(root)
            .status()
            .unwrap();
        let id = read_identity(root);
        assert_eq!(id.name.as_deref(), Some("Minos Test"));
        assert_eq!(id.email.as_deref(), Some("minos@example.com"));
        assert!(id.ensure_complete().is_ok());
    }
}
