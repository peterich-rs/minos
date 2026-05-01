use std::fs;
use std::path::PathBuf;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use minos_domain::MinosError;

pub fn minos_home() -> Result<PathBuf, MinosError> {
    if let Ok(path) = std::env::var("MINOS_HOME") {
        return Ok(PathBuf::from(path));
    }

    let home = std::env::var("HOME").map_err(|_| MinosError::StoreIo {
        path: "$HOME".into(),
        message: "HOME env var not set".into(),
    })?;
    Ok(PathBuf::from(home).join(".minos"))
}

fn ensure_subdir(name: &str) -> Result<PathBuf, MinosError> {
    let p = minos_home()?.join(name);
    fs::create_dir_all(&p).map_err(|e| MinosError::StoreIo {
        path: p.display().to_string(),
        message: e.to_string(),
    })?;
    Ok(p)
}

pub fn state_dir() -> Result<PathBuf, MinosError> {
    ensure_subdir("state")
}

pub fn secrets_dir() -> Result<PathBuf, MinosError> {
    let p = ensure_subdir("secrets")?;
    #[cfg(unix)]
    {
        let mut perm = fs::metadata(&p)
            .map_err(|e| MinosError::StoreIo {
                path: p.display().to_string(),
                message: e.to_string(),
            })?
            .permissions();
        perm.set_mode(0o700);
        fs::set_permissions(&p, perm).map_err(|e| MinosError::StoreIo {
            path: p.display().to_string(),
            message: e.to_string(),
        })?;
    }
    Ok(p)
}

pub fn db_dir() -> Result<PathBuf, MinosError> {
    ensure_subdir("db")
}

pub fn db_path() -> Result<PathBuf, MinosError> {
    Ok(db_dir()?.join("minos.sqlite"))
}

pub fn logs_dir() -> Result<PathBuf, MinosError> {
    ensure_subdir("logs")
}

pub fn workspaces_dir() -> Result<PathBuf, MinosError> {
    ensure_subdir("workspaces")
}

pub fn run_dir() -> Result<PathBuf, MinosError> {
    ensure_subdir("run")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minos_home_prefers_env_override() {
        std::env::set_var("MINOS_HOME", "/tmp/minos-home-test");
        let resolved = minos_home().unwrap();
        assert_eq!(resolved, PathBuf::from("/tmp/minos-home-test"));
        std::env::remove_var("MINOS_HOME");
    }

    #[test]
    fn state_dir_is_under_minos_home() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("MINOS_HOME", tmp.path());
        let s = state_dir().unwrap();
        assert_eq!(s, tmp.path().join("state"));
        assert!(s.is_dir());
        std::env::remove_var("MINOS_HOME");
    }

    #[test]
    fn db_path_is_under_db_dir() {
        let tmp = tempfile::tempdir().unwrap();
        std::env::set_var("MINOS_HOME", tmp.path());
        let p = db_path().unwrap();
        assert_eq!(p, tmp.path().join("db").join("minos.sqlite"));
        std::env::remove_var("MINOS_HOME");
    }

    #[test]
    fn secrets_dir_has_owner_only_perms() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let tmp = tempfile::tempdir().unwrap();
            std::env::set_var("MINOS_HOME", tmp.path());
            let s = secrets_dir().unwrap();
            let mode = std::fs::metadata(&s).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700);
            std::env::remove_var("MINOS_HOME");
        }
    }
}
