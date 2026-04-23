use std::path::PathBuf;

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
}
