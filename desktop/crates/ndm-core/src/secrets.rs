#[cfg(windows)]
fn protect_or_unprotect(input: &[u8], protect: bool) -> crate::Result<Vec<u8>> {
    use std::{io, ptr, slice};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };

    let input_len = u32::try_from(input.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "protected data is too large"))?;
    let source = CRYPT_INTEGER_BLOB {
        cbData: input_len,
        pbData: input.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();

    // SAFETY: source points to input for the duration of the call; output is
    // initialized by DPAPI. The returned allocation is copied and freed below.
    let success = unsafe {
        if protect {
            CryptProtectData(
                &source,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &source,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 {
        return Err(io::Error::last_os_error().into());
    }

    // SAFETY: a successful DPAPI call returns cbData initialized bytes at
    // pbData, allocated with LocalAlloc. Wipe that copy before LocalFree.
    let bytes = unsafe {
        let bytes = slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec();
        ptr::write_bytes(output.pbData, 0, output.cbData as usize);
        LocalFree(output.pbData.cast());
        bytes
    };
    Ok(bytes)
}

#[cfg(not(any(windows, target_os = "macos")))]
fn protect_or_unprotect(_input: &[u8], _protect: bool) -> crate::Result<Vec<u8>> {
    Err(crate::Error::UnsupportedPlatform)
}

pub(crate) fn protect(input: &[u8]) -> crate::Result<Vec<u8>> {
    protect_or_unprotect(input, true)
}

pub(crate) fn unprotect(input: &[u8]) -> crate::Result<Vec<u8>> {
    protect_or_unprotect(input, false)
}

#[cfg(target_os = "macos")]
fn protect_or_unprotect(input: &[u8], protect: bool) -> crate::Result<Vec<u8>> {
    use aes_gcm::{Aes256Gcm, KeyInit, aead::Aead};
    use std::{
        fs::OpenOptions,
        io::{self, Read, Write},
        os::unix::fs::OpenOptionsExt,
        sync::Mutex,
    };
    const PREFIX: &[u8] = b"NDMK1";
    static KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid protected credentials");
    let key = {
        let mut cached = KEY
            .lock()
            .map_err(|_| io::Error::other("credential lock poisoned"))?;
        if cached.is_none() {
            let directory = crate::application_data_directory();
            std::fs::create_dir_all(&directory)?;
            let path = directory.join("credentials.key");
            let mut bytes = Vec::new();
            match OpenOptions::new().read(true).write(true).open(&path) {
                Ok(mut file) => {
                    file.read_to_end(&mut bytes)?;
                    if bytes.len() != 32 {
                        return Err(invalid().into());
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    bytes.resize(32, 0);
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&path)?;
                    getrandom::fill(&mut bytes)
                        .map_err(|error| io::Error::other(error.to_string()))?;
                    file.write_all(&bytes)?;
                    file.sync_all()?;
                }
                Err(error) => return Err(error.into()),
            }
            *cached = Some(bytes.try_into().map_err(|_| invalid())?);
        }
        cached.unwrap()
    };
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|_| invalid())?;
    if protect {
        let mut nonce = [0; 12];
        getrandom::fill(&mut nonce).map_err(|error| io::Error::other(error.to_string()))?;
        let encrypted = cipher
            .encrypt(&nonce.into(), input)
            .map_err(|_| invalid())?;
        let mut output = Vec::with_capacity(PREFIX.len() + 12 + encrypted.len());
        output.extend_from_slice(PREFIX);
        output.extend_from_slice(&nonce);
        output.extend(encrypted);
        Ok(output)
    } else {
        if input.len() < PREFIX.len() + 12 + 16 || !input.starts_with(PREFIX) {
            return Err(invalid().into());
        }
        let nonce: [u8; 12] = input[PREFIX.len()..PREFIX.len() + 12]
            .try_into()
            .map_err(|_| invalid())?;
        cipher
            .decrypt(&nonce.into(), &input[PREFIX.len() + 12..])
            .map_err(|_| invalid().into())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod macos_tests {
    #[test]
    fn credentials_round_trip_with_random_nonces_and_reject_tampering() {
        let secret = b"private-cookie-and-password";
        let first = super::protect(secret).unwrap();
        let second = super::protect(secret).unwrap();
        assert_ne!(first, second);
        assert!(!first.windows(secret.len()).any(|window| window == secret));
        assert_eq!(super::unprotect(&first).unwrap(), secret);
        let mut damaged = first;
        *damaged.last_mut().unwrap() ^= 1;
        assert!(super::unprotect(&damaged).is_err());
        assert!(super::unprotect(secret).is_err());
    }
}
