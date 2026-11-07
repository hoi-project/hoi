//! Optional, device-local OS credential. The unlock key never crosses the webview.
//! A marker contains only a header fingerprint; reading status never prompts the keystore.
use std::path::Path;

use hoi_crypto::vault::LocalVault;
use keyring::Entry;
use zeroize::Zeroizing;

#[cfg(windows)]
#[path = "windows_credential.rs"]
mod windows_credential;

const MARKER: &str = "os-unlock";
const UNAVAILABLE: &str = "OS unlock is unavailable or was cancelled. Unlock with your vault password, then try enabling OS unlock again in You.";

pub fn entry(directory: &Path) -> Result<Entry, String> {
    let account = LocalVault::fingerprint(directory.as_os_str().as_encoded_bytes());
    Entry::new("sh.hoi.desktop.vault", &account).map_err(|_| UNAVAILABLE.into())
}

pub fn configured(directory: &Path) -> bool {
    directory.join(MARKER).exists()
}

pub fn enabled(directory: &Path, header: &[u8]) -> bool {
    std::fs::read_to_string(directory.join(MARKER)).ok().as_deref() == Some(&LocalVault::fingerprint(header))
}

/// Leave a pending marker on failure so disable/reset can clean up even a partial save.
pub fn enable(directory: &Path, entry: &Entry, vault: &LocalVault) -> Result<(), String> {
    let secret = vault.os_unlock_secret().map_err(|_| UNAVAILABLE.to_string())?;
    std::fs::write(directory.join(MARKER), "pending").map_err(|_| "Could not save OS unlock settings.".to_string())?;
    store_local(entry, &secret)?;
    let retrieved = read(entry)?;
    if *retrieved != *secret {
        return Err(UNAVAILABLE.into());
    }
    std::fs::write(directory.join(MARKER), LocalVault::fingerprint(vault.header())).map_err(|_| "Could not save OS unlock settings.".to_string())
}

fn store_local(entry: &Entry, secret: &[u8]) -> Result<(), String> {
    // keyring's Windows writer uses CRED_PERSIST_ENTERPRISE. Keep this vault's key on
    // this machine; the same keyring entry still handles lookup and deletion.
    #[cfg(windows)]
    if let Some(credential) = entry.get_credential().downcast_ref::<keyring::windows::WinCredential>() {
        return windows_credential::set_local(credential, secret).map_err(|_| UNAVAILABLE.into());
    }
    #[cfg(all(windows, not(test)))]
    return Err(UNAVAILABLE.into());
    // On Windows tests use the in-memory mock; never write to a real credential store.
    #[cfg(any(not(windows), test))]
    entry.set_secret(secret).map_err(|_| UNAVAILABLE.into())
}

pub fn read(entry: &Entry) -> Result<Zeroizing<Vec<u8>>, String> {
    entry.get_secret().map(Zeroizing::new).map_err(|_| UNAVAILABLE.into())
}

pub fn disable(directory: &Path, entry: &Entry) -> Result<(), String> {
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => {}
        Err(_) => return Err("The OS could not remove its unlock entry. Unlock your OS keystore and try again. Your vault password still works.".into()),
    }
    match std::fs::remove_file(directory.join(MARKER)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("The OS entry was removed, but its local setting could not be cleared. Try disabling it again.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keyring::mock::MockCredential;

    #[test]
    fn verify_before_enable_and_keep_failed_cleanup_retryable() {
        let directory = std::env::temp_dir().join(format!("hoi-os-unlock-{}", LocalVault::fingerprint(&hoi_crypto::random_bytes::<32>().unwrap())));
        std::fs::create_dir(&directory).unwrap();
        let entry = Entry::new_with_credential(Box::<MockCredential>::default());
        let mock: &MockCredential = entry.get_credential().downcast_ref().unwrap();
        let vault = LocalVault::create("an OS unlock regression password").unwrap();
        assert!(!enabled(&directory, vault.header()));
        assert!(read(&entry).is_err());
        mock.set_error(keyring::Error::NoEntry);
        assert!(enable(&directory, &entry, &vault).is_err());
        assert!(configured(&directory));
        assert!(!enabled(&directory, vault.header()));
        enable(&directory, &entry, &vault).unwrap();
        assert!(enabled(&directory, vault.header()));
        assert_eq!(*read(&entry).unwrap(), *vault.os_unlock_secret().unwrap());
        assert!(!enabled(&directory, b"another vault"));
        mock.set_error(keyring::Error::Invalid("locked".into(), "test".into()));
        assert!(disable(&directory, &entry).is_err());
        assert!(configured(&directory));
        disable(&directory, &entry).unwrap();
        assert!(!configured(&directory));
        assert!(read(&entry).is_err());
        disable(&directory, &entry).unwrap();
        std::fs::remove_dir(directory).unwrap();
    }
}
