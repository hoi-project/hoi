//! One Windows-specific write: keyring otherwise selects enterprise-roaming persistence.
use keyring::windows::WinCredential;
use windows_sys::Win32::Security::Credentials::{CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC, CREDENTIALW, CredWriteW};
use zeroize::Zeroizing;

pub fn set_local(entry: &WinCredential, secret: &[u8]) -> Result<(), ()> {
    let wide = |text: &str| text.encode_utf16().chain(std::iter::once(0)).collect::<Vec<_>>();
    let mut username = wide(&entry.username);
    let mut target = wide(&entry.target_name);
    let mut comment = wide("hoi: unlock this computer's encrypted vault");
    let mut blob = Zeroizing::new(secret.to_vec());
    let credential = CREDENTIALW {
        Type: CRED_TYPE_GENERIC,
        TargetName: target.as_mut_ptr(),
        UserName: username.as_mut_ptr(),
        Comment: comment.as_mut_ptr(),
        CredentialBlobSize: blob.len().try_into().map_err(|_| ())?,
        CredentialBlob: blob.as_mut_ptr(),
        Persist: CRED_PERSIST_LOCAL_MACHINE,
        ..Default::default()
    };
    // All buffers outlive this synchronous call. Windows copies the blob before returning.
    if unsafe { CredWriteW(&credential, 0) } == 0 { Err(()) } else { Ok(()) }
}
