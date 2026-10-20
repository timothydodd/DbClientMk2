//! Credentials at rest. Formats (prefix + base64), kept compatible with the earlier .NET app:
//! - `dpapi:` Windows DPAPI (current user) with app entropy `dbclient-credential-v2`
//! - `aesgcm:` AES-256-GCM, nonce[12] | tag[16] | ciphertext; key = PBKDF2-SHA256 (600k) over
//!   machine id + NUL + user name, salted with a per-install random `~/.dbclient/.salt`
//!
//! `encrypt` fails rather than return plaintext; `try_decrypt` returns `Err` when a stored value
//! can't be read (another machine / account), so the UI can ask for the password again.

use crate::paths::{write_atomic, AppPaths};
use base64::Engine;

const DPAPI_PREFIX: &str = "dpapi:";
const AESGCM_PREFIX: &str = "aesgcm:";
const LEGACY_PREFIX: &str = "ENC:";

fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(DPAPI_PREFIX) || value.starts_with(AESGCM_PREFIX) || value.starts_with(LEGACY_PREFIX)
}

pub struct Protector {
    paths: AppPaths,
    key: std::sync::OnceLock<[u8; 32]>,
}

impl Protector {
    pub fn new(paths: AppPaths) -> Self {
        Self { paths, key: std::sync::OnceLock::new() }
    }

    pub fn encrypt(&self, plaintext: &str) -> Result<String, String> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        #[cfg(windows)]
        {
            let blob = dpapi::protect(plaintext.as_bytes())?;
            return Ok(format!("{DPAPI_PREFIX}{}", b64().encode(blob)));
        }
        #[cfg(not(windows))]
        {
            use aes_gcm::aead::{Aead, KeyInit};
            use aes_gcm::{Aes256Gcm, Nonce};
            let key = self.key()?;
            let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
            let mut nonce = [0u8; 12];
            rand::fill(&mut nonce);
            // aes-gcm appends the tag; the stored layout is nonce | tag | ciphertext.
            let sealed = cipher.encrypt(&Nonce::from(nonce), plaintext.as_bytes()).map_err(|e| e.to_string())?;
            let (cipher_text, tag) = sealed.split_at(sealed.len() - 16);
            let mut blob = Vec::with_capacity(12 + 16 + cipher_text.len());
            blob.extend_from_slice(&nonce);
            blob.extend_from_slice(tag);
            blob.extend_from_slice(cipher_text);
            Ok(format!("{AESGCM_PREFIX}{}", b64().encode(blob)))
        }
    }

    /// Plain (unprefixed) values pass through: they were saved before encryption existed.
    pub fn try_decrypt(&self, stored: &str) -> Result<String, String> {
        if stored.is_empty() {
            return Ok(String::new());
        }
        if !is_encrypted(stored) {
            return Ok(stored.to_string());
        }
        if let Some(rest) = stored.strip_prefix(DPAPI_PREFIX) {
            #[cfg(windows)]
            {
                let data = b64().decode(rest).map_err(|_| "The saved value is damaged.")?;
                let plain = dpapi::unprotect(&data)?;
                return String::from_utf8(plain).map_err(|_| "The saved value is damaged.".to_string());
            }
            #[cfg(not(windows))]
            {
                let _ = rest;
                return Err("Saved with Windows DPAPI; this platform can't decrypt it.".into());
            }
        }
        if let Some(rest) = stored.strip_prefix(AESGCM_PREFIX) {
            use aes_gcm::aead::{Aead, KeyInit};
            use aes_gcm::{Aes256Gcm, Nonce};
            let blob = b64().decode(rest).map_err(|_| "The saved value is damaged.")?;
            if blob.len() < 28 {
                return Err("The saved value is damaged.".into());
            }
            let (nonce, rest) = blob.split_at(12);
            let (tag, cipher_text) = rest.split_at(16);
            let mut sealed = Vec::with_capacity(rest.len());
            sealed.extend_from_slice(cipher_text);
            sealed.extend_from_slice(tag);
            let key = self.key()?;
            let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| e.to_string())?;
            let nonce: [u8; 12] = nonce.try_into().map_err(|_| "The saved value is damaged.".to_string())?;
            let plain = cipher
                .decrypt(&Nonce::from(nonce), sealed.as_ref())
                .map_err(|_| "It was saved on another computer or by another user.".to_string())?;
            return String::from_utf8(plain).map_err(|_| "The saved value is damaged.".to_string());
        }
        Err("This credential uses a format this version no longer reads; enter it again.".into())
    }

    fn key(&self) -> Result<&[u8; 32], String> {
        if let Some(k) = self.key.get() {
            return Ok(k);
        }
        let salt = self.load_or_create_salt()?;
        let mut material = machine_id();
        material.push(0);
        material.extend_from_slice(user_name().as_bytes());
        let mut key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<sha2::Sha256>(&material, &salt, 600_000, &mut key);
        let _ = self.key.set(key);
        Ok(self.key.get().unwrap())
    }

    fn load_or_create_salt(&self) -> Result<Vec<u8>, String> {
        let path = self.paths.salt_file();
        if let Ok(existing) = std::fs::read(&path) {
            if existing.len() == 32 {
                return Ok(existing);
            }
            crate::logger::warn("Credential salt file has unexpected length; regenerating (stored passwords will need re-entry)");
        }
        let mut salt = vec![0u8; 32];
        rand::fill(&mut salt[..]);
        write_atomic(&path, &salt).map_err(|e| format!("Couldn't write the credential salt: {e}"))?;
        crate::logger::info("Created new credential salt");
        Ok(salt)
    }
}

fn user_name() -> String {
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()
}

fn machine_id() -> Vec<u8> {
    if let Ok(s) = std::fs::read_to_string("/etc/machine-id") {
        return s.trim().as_bytes().to_vec();
    }
    if let Ok(b) = std::fs::read("/var/db/SystemKey") {
        return b;
    }
    let host = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_else(|_| "localhost".into());
    format!("{host}:{}", user_name()).into_bytes()
}

#[cfg(windows)]
mod dpapi {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB};

    /// Same entropy the .NET app used, so its saved passwords still open.
    const ENTROPY: &[u8] = b"dbclient-credential-v2";

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB { cbData: data.len() as u32, pbData: data.as_ptr() as *mut u8 }
    }

    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        v
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(plain);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        // SAFETY: input/entropy point at live slices for the call; the OS allocates `out`, which `take` copies and frees.
        let ok = unsafe { CryptProtectData(&input, null(), &entropy, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
        if ok == 0 {
            return Err("Windows could not protect the credential.".into());
        }
        Ok(unsafe { take(out) })
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let input = blob(data);
        let entropy = blob(ENTROPY);
        let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: null_mut() };
        // SAFETY: as in `protect`.
        let ok = unsafe { CryptUnprotectData(&input, null_mut(), &entropy, null(), null(), CRYPTPROTECT_UI_FORBIDDEN, &mut out) };
        if ok == 0 {
            return Err("It was saved by another Windows account or on another computer.".into());
        }
        Ok(unsafe { take(out) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_never_stores_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let p = Protector::new(AppPaths::at(dir.path()));
        let enc = p.encrypt("s3cret-plain").unwrap();
        assert!(is_encrypted(&enc));
        assert!(!enc.contains("s3cret-plain"));
        assert_eq!(p.try_decrypt(&enc).unwrap(), "s3cret-plain");
        assert_eq!(p.try_decrypt("plain").unwrap(), "plain");
        assert_eq!(p.encrypt("").unwrap(), "");
    }

    #[test]
    fn damaged_values_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = Protector::new(AppPaths::at(dir.path()));
        assert!(p.try_decrypt("aesgcm:!!!").is_err());
        assert!(p.try_decrypt("ENC:abcd").is_err());
    }
}
