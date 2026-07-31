use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("durable secret storage is only available through Windows DPAPI")]
    UnsupportedPlatform,
    #[error("DPAPI operation failed: {0}")]
    Dpapi(String),
    #[error("encrypted payload is invalid")]
    InvalidPayload,
}

/// Encrypts the complete persisted state, including all EasyTier secrets.
pub trait StateProtector: Send + Sync {
    fn algorithm(&self) -> &'static str;
    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError>;
    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError>;
}

/// Windows DPAPI machine-scope protector. The Windows service can decrypt the
/// state before a user signs in, while the data remains unreadable at rest.
#[derive(Clone, Copy, Debug, Default)]
pub struct DpapiProtector;

#[cfg(windows)]
impl StateProtector for DpapiProtector {
    fn algorithm(&self) -> &'static str {
        "dpapi-local-machine-v1"
    }

    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use std::{ptr, slice};
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::Cryptography::{
                CryptProtectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPT_INTEGER_BLOB,
            },
        };

        let length = u32::try_from(plaintext.len()).map_err(|_| CryptoError::InvalidPayload)?;
        let input = CRYPT_INTEGER_BLOB {
            cbData: length,
            pbData: plaintext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let protected = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_LOCAL_MACHINE,
                &mut output,
            )
        };
        if protected == 0 {
            return Err(CryptoError::Dpapi(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        if output.cbData > 0 && output.pbData.is_null() {
            return Err(CryptoError::InvalidPayload);
        }
        let bytes = if output.cbData == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
        };
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        Ok(bytes)
    }

    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        use std::{ptr, slice};
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB},
        };

        let length = u32::try_from(ciphertext.len()).map_err(|_| CryptoError::InvalidPayload)?;
        let mut input = CRYPT_INTEGER_BLOB {
            cbData: length,
            pbData: ciphertext.as_ptr() as *mut u8,
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: ptr::null_mut(),
        };
        let unprotected = unsafe {
            CryptUnprotectData(
                &mut input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                0,
                &mut output,
            )
        };
        if unprotected == 0 {
            return Err(CryptoError::Dpapi(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        if output.cbData > 0 && output.pbData.is_null() {
            return Err(CryptoError::InvalidPayload);
        }
        let bytes = if output.cbData == 0 {
            Vec::new()
        } else {
            unsafe { slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() }
        };
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        Ok(bytes)
    }
}

#[cfg(not(windows))]
impl StateProtector for DpapiProtector {
    fn algorithm(&self) -> &'static str {
        "unavailable"
    }

    fn protect(&self, _plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedPlatform)
    }

    fn unprotect(&self, _ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Err(CryptoError::UnsupportedPlatform)
    }
}

#[cfg(test)]
pub(crate) struct TestProtector;

#[cfg(test)]
impl StateProtector for TestProtector {
    fn algorithm(&self) -> &'static str {
        "test-reversible-v1"
    }

    fn protect(&self, plaintext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Ok(plaintext.iter().rev().map(|byte| byte ^ 0xA5).collect())
    }

    fn unprotect(&self, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        Ok(ciphertext.iter().rev().map(|byte| byte ^ 0xA5).collect())
    }
}

#[cfg(all(test, windows))]
mod windows_tests {
    use super::{DpapiProtector, StateProtector};

    #[test]
    fn dpapi_machine_scope_round_trips_state() {
        let protector = DpapiProtector;
        let plaintext = b"vibe-easytier durable state";

        let encrypted = protector.protect(plaintext).unwrap();
        assert_ne!(encrypted, plaintext);
        assert_eq!(protector.unprotect(&encrypted).unwrap(), plaintext);
    }
}
