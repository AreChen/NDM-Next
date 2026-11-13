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

#[cfg(not(windows))]
fn protect_or_unprotect(_input: &[u8], _protect: bool) -> crate::Result<Vec<u8>> {
    Err(crate::Error::UnsupportedPlatform)
}

pub(crate) fn protect(input: &[u8]) -> crate::Result<Vec<u8>> {
    protect_or_unprotect(input, true)
}

pub(crate) fn unprotect(input: &[u8]) -> crate::Result<Vec<u8>> {
    protect_or_unprotect(input, false)
}
