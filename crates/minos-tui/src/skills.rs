use anyhow::Result;
use minos_chat_store::teamwork_mcp::SkillRef;
use minos_prompt_runtime::{TEAMWORK_SKILL_ID, TEAMWORK_SKILL_MD};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillInstallReport {
    pub installed_paths: Vec<PathBuf>,
}

pub fn install_global_agent_skills(skill_refs: &[SkillRef]) -> Result<SkillInstallReport> {
    let home = resolve_home_dir()?;
    install_global_agent_skills_for_home(&home, skill_refs)
}

fn install_global_agent_skills_for_home(
    home: &Path,
    skill_refs: &[SkillRef],
) -> Result<SkillInstallReport> {
    let mut installed_paths = Vec::new();
    for skill_ref in skill_refs {
        let content = embedded_skill_content(skill_ref.id)?;
        for path in global_skill_paths(home, skill_ref.id) {
            write_if_changed(&path, content)?;
            installed_paths.push(path);
        }
    }
    Ok(SkillInstallReport { installed_paths })
}

fn embedded_skill_content(skill_id: &str) -> Result<&'static str> {
    // Canonical skill body: minos-prompt-runtime package (Task B SSOT).
    if skill_id == TEAMWORK_SKILL_ID {
        return Ok(TEAMWORK_SKILL_MD);
    }
    anyhow::bail!("unknown embedded skill ref: {skill_id}")
}

fn global_skill_paths(home: &Path, skill_name: &str) -> Vec<PathBuf> {
    [
        home.join(".agents").join("skills"),
        home.join(".claude").join("skills"),
        home.join(".gemini").join("skills"),
        home.join(".config").join("opencode").join("skills"),
    ]
    .into_iter()
    .map(|root| root.join(skill_name).join("SKILL.md"))
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
    use minos_prompt_runtime::TEAMWORK_SKILL_MD;

    #[test]
    fn global_skill_paths_include_supported_cli_locations() {
        let home = PathBuf::from("/tmp/minos-home");
        let paths = global_skill_paths(&home, "minos-teamwork");

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

        let report = install_global_agent_skills_for_home(
            temp.path(),
            &[minos_chat_store::teamwork_mcp::MINOS_TEAMWORK_SKILL],
        )
        .unwrap();

        assert_eq!(report.installed_paths.len(), 4);
        for path in report.installed_paths {
            let content = std::fs::read_to_string(path).unwrap();
            assert!(content.contains("name: minos-teamwork"));
            assert!(content.contains("minos_teamwork"));
            assert!(content.contains("Use when "));
            assert_eq!(content, TEAMWORK_SKILL_MD);
        }
    }
}
