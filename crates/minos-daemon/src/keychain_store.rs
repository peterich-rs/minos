//! macOS Keychain adapter for the `device-secret` account under service
//! `ai.minos.macos`. CF Access credentials are process-env configuration;
//! this module only owns the Minos business-layer device secret.

#[cfg(target_os = "macos")]
pub mod imp {
    use minos_domain::{DeviceSecret, MinosError};
    use security_framework::item::{ItemClass, ItemSearchOptions, SearchResult};
    use security_framework::passwords::{delete_generic_password, set_generic_password};

    const SERVICE: &str = "ai.minos.macos";
    const ACCOUNT_DEVICE_SECRET: &str = "device-secret";

    /// errSecItemNotFound — returned by the Keychain when no matching
    /// entry exists for a get/delete. We map this to `Ok(None)` on read
    /// and a clean `Ok(())` on delete so callers can treat "nothing to
    /// do" identically regardless of prior state.
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;

    pub struct KeychainTrustedDeviceStore;

    impl KeychainTrustedDeviceStore {
        /// Read the persisted `device-secret`. Returns `Ok(None)` when
        /// no entry is present or when the only matching entry requires
        /// interactive authentication. Startup must never trigger a Keychain
        /// password / Touch ID prompt.
        pub fn read(&self) -> Result<Option<DeviceSecret>, MinosError> {
            match read_no_ui() {
                Ok(Some(bytes)) => decode_secret(bytes),
                Ok(None) => Ok(None),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(None),
                Err(e) => Err(MinosError::StoreIo {
                    path: format!("Keychain {SERVICE}/{ACCOUNT_DEVICE_SECRET}"),
                    message: format!("keychain read: {e}"),
                }),
            }
        }

        pub fn write(&self, secret: &DeviceSecret) -> Result<(), MinosError> {
            set_generic_password(SERVICE, ACCOUNT_DEVICE_SECRET, secret.0.as_bytes()).map_err(|e| {
                MinosError::StoreIo {
                    path: format!("Keychain {SERVICE}/{ACCOUNT_DEVICE_SECRET}"),
                    message: format!("keychain write: {e}"),
                }
            })
        }

        /// Delete the entry. Succeeds (`Ok`) if the entry doesn't exist,
        /// since the caller's intent ("make sure this isn't present")
        /// is satisfied either way.
        pub fn delete(&self) -> Result<(), MinosError> {
            match delete_generic_password(SERVICE, ACCOUNT_DEVICE_SECRET) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(()),
                Err(e) => Err(MinosError::StoreIo {
                    path: format!("Keychain {SERVICE}/{ACCOUNT_DEVICE_SECRET}"),
                    message: format!("keychain delete: {e}"),
                }),
            }
        }
    }

    fn read_no_ui() -> security_framework::base::Result<Option<Vec<u8>>> {
        let mut search = ItemSearchOptions::new();
        search
            .class(ItemClass::generic_password())
            .service(SERVICE)
            .account(ACCOUNT_DEVICE_SECRET)
            .load_data(true)
            .skip_authenticated_items(true);

        let mut items = search.search()?;
        Ok(items.pop().and_then(|item| match item {
            SearchResult::Data(bytes) => Some(bytes),
            _ => None,
        }))
    }

    fn decode_secret(bytes: Vec<u8>) -> Result<Option<DeviceSecret>, MinosError> {
        let s = String::from_utf8(bytes).map_err(|e| MinosError::StoreCorrupt {
            path: format!("Keychain {SERVICE}/{ACCOUNT_DEVICE_SECRET}"),
            message: format!("utf8 decode: {e}"),
        })?;
        Ok(Some(DeviceSecret(s)))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Integration: writes then reads via the real login keychain.
        /// Runs on macOS dev + CI. The GitHub macos-15 runner has an
        /// unlocked login.keychain-db by default. Reads use
        /// `kSecUseAuthenticationUISkip` through `skip_authenticated_items`,
        /// so the test should fail rather than prompt.
        ///
        /// Cleans up on entry and exit so a crashed prior run leaves no
        /// residue that would confuse subsequent runs.
        #[test]
        fn write_then_read_round_trips() {
            let store = KeychainTrustedDeviceStore;
            let _ = store.delete();

            let secret = DeviceSecret("test-secret-xyz".into());
            store.write(&secret).unwrap();
            let got = store.read().unwrap().expect("just wrote");
            assert_eq!(got, secret);

            store.delete().unwrap();
            assert!(store.read().unwrap().is_none());
        }
    }
}

#[cfg(target_os = "macos")]
pub use imp::KeychainTrustedDeviceStore;
