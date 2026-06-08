use anyhow::Result;
use std::path::{Path, PathBuf};

const MINOS_TEAMWORK_SKILL_NAME: &str = "minos-teamwork";
const MINOS_TEAMWORK_SKILL_MD: &str = include_str!("../skills/minos-teamwork/SKILL.md");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallReport {
    pub installed_paths: Vec<PathBuf>,
}

pub fn install_global_agent_skills() -> Result<SkillInstallReport> {
    let home = resolve_home_dir()?;
    install_global_agent_skills_for_home(&home)
}

fn install_global_agent_skills_for_home(home: &Path) -> Result<SkillInstallReport> {
    let installed_paths = global_skill_paths(home)
        .into_iter()
        .map(|path| {
            write_if_changed(&path, MINOS_TEAMWORK_SKILL_MD)?;
            Ok(path)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SkillInstallReport { installed_paths })
}

fn global_skill_paths(home: &Path) -> Vec<PathBuf> {
    [
        home.join(".agents").join("skills"),
        home.join(".claude").join("skills"),
        home.join(".gemini").join("skills"),
        home.join(".config").join("opencode").join("skills"),
    ]
    .into_iter()
    .map(|root| root.join(MINOS_TEAMWORK_SKILL_NAME).join("SKILL.md"))
    .collect()
}

fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if matches!(std::fs::read_to_string(path), Ok(existing) if existing == content) {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("skill path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)?;
    std::fs::write(path, content)?;
    Ok(())
}

fn resolve_home_dir() -> Result<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(home));
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
        return Ok(PathBuf::from(user_profile));
    }
    let home_drive = std::env::var_os("HOMEDRIVE").filter(|value| !value.is_empty());
    let home_path = std::env::var_os("HOMEPATH").filter(|value| !value.is_empty());
    if let (Some(drive), Some(path)) = (home_drive, home_path) {
        return Ok(PathBuf::from(drive).join(path));
    }
    anyhow::bail!("unable to resolve home directory from environment")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_skill_paths_include_supported_cli_locations() {
        let home = PathBuf::from("/tmp/minos-home");
        let paths = global_skill_paths(&home);

        assert_eq!(paths.len(), 4);
        assert!(paths
            .iter()
            .any(|path| path == &home.join(".agents/skills/minos-teamwork/SKILL.md")));
        assert!(paths
            .iter()
            .any(|path| path == &home.join(".claude/skills/minos-teamwork/SKILL.md")));
        assert!(paths
            .iter()
            .any(|path| path == &home.join(".gemini/skills/minos-teamwork/SKILL.md")));
        assert!(paths
            .iter()
            .any(|path| path == &home.join(".config/opencode/skills/minos-teamwork/SKILL.md")));
    }

    #[test]
    fn install_global_agent_skills_writes_embedded_skill() {
        let temp = tempfile::tempdir().unwrap();

        let report = install_global_agent_skills_for_home(temp.path()).unwrap();

        assert_eq!(report.installed_paths.len(), 4);
        for path in report.installed_paths {
            let content = std::fs::read_to_string(path).unwrap();
            assert!(content.contains("name: minos-teamwork"));
            assert!(content.contains("minos_chat"));
        }
    }
}
