//! DeviceId + account/host tokens.
//!
//! macOS: Data Protection Keychain, AfterFirstUnlockThisDeviceOnly (no prompt).
//! Other targets: 0600 files under `~/.minos/secrets/desktop/` for tests.

const SERVICE: &str = "com.minos.desktop";

#[derive(Debug, thiserror::Error)]
pub enum SecureStoreError {
    #[error("secure store: {0}")]
    Backend(String),
}

pub fn get(account: &str) -> Result<Option<String>, SecureStoreError> {
    platform::get(SERVICE, account)
}

pub fn set(account: &str, value: &str) -> Result<(), SecureStoreError> {
    platform::set(SERVICE, account, value)
}

pub fn delete(account: &str) -> Result<(), SecureStoreError> {
    platform::delete(SERVICE, account)
}

#[cfg(target_os = "macos")]
mod platform {
    use super::SecureStoreError;
    use security_framework::access_control::{ProtectionMode, SecAccessControl};
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password_options,
        PasswordOptions,
    };

    fn write_options(service: &str, account: &str) -> Result<PasswordOptions, SecureStoreError> {
        let mut options = PasswordOptions::new_generic_password(service, account);
        // Never sync host_token / refresh / DeviceId to iCloud Keychain.
        options.set_access_synchronized(Some(false));
        let access = SecAccessControl::create_with_protection(
            Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly),
            0,
        )
        .map_err(|e| SecureStoreError::Backend(e.to_string()))?;
        options.set_access_control(access);
        Ok(options)
    }

    pub fn get(service: &str, account: &str) -> Result<Option<String>, SecureStoreError> {
        match get_generic_password(service, account) {
            Ok(bytes) => String::from_utf8(bytes)
                .map(Some)
                .map_err(|e| SecureStoreError::Backend(e.to_string())),
            Err(err) if is_not_found(&err) => Ok(None),
            Err(err) => Err(SecureStoreError::Backend(err.to_string())),
        }
    }

    pub fn set(service: &str, account: &str, value: &str) -> Result<(), SecureStoreError> {
        let _ = delete_generic_password(service, account);
        let options = write_options(service, account)?;
        set_generic_password_options(value.as_bytes(), options)
            .map_err(|e| SecureStoreError::Backend(e.to_string()))
    }

    pub fn delete(service: &str, account: &str) -> Result<(), SecureStoreError> {
        match delete_generic_password(service, account) {
            Ok(()) => Ok(()),
            Err(err) if is_not_found(&err) => Ok(()),
            Err(err) => Err(SecureStoreError::Backend(err.to_string())),
        }
    }

    fn is_not_found(err: &security_framework::base::Error) -> bool {
        let msg = err.to_string();
        msg.contains("not found") || msg.contains("-25300") || msg.contains("errSecItemNotFound")
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use super::SecureStoreError;
    use std::fs;
    use std::io::ErrorKind;
    use std::path::PathBuf;

    fn path(account: &str) -> Result<PathBuf, SecureStoreError> {
        let home = dirs::home_dir()
            .ok_or_else(|| SecureStoreError::Backend("home directory unavailable".into()))?;
        Ok(home.join(".minos/secrets/desktop").join(account))
    }

    pub fn get(_service: &str, account: &str) -> Result<Option<String>, SecureStoreError> {
        let p = path(account)?;
        match fs::read_to_string(&p) {
            Ok(s) => Ok(Some(s)),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(None),
            Err(e) => Err(SecureStoreError::Backend(e.to_string())),
        }
    }

    pub fn set(_service: &str, account: &str, value: &str) -> Result<(), SecureStoreError> {
        let p = path(account)?;
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir).map_err(|e| SecureStoreError::Backend(e.to_string()))?;
        }
        fs::write(&p, value).map_err(|e| SecureStoreError::Backend(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&p, fs::Permissions::from_mode(0o600));
        }
        Ok(())
    }

    pub fn delete(_service: &str, account: &str) -> Result<(), SecureStoreError> {
        let p = path(account)?;
        match fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(SecureStoreError::Backend(e.to_string())),
        }
    }
}
