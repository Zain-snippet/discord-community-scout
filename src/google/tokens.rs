//! Token storage. On Windows the blob is encrypted with DPAPI and bound to the
//! current Windows user. On other platforms it is a file with mode 0600, which
//! is documented as weaker than an OS keychain.

use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

/// Tokens exactly as returned by Google. Never rendered, logged or committed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenSet {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    /// Unix seconds.
    pub expires_at: i64,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub token_type: Option<String>,
}

impl TokenSet {
    pub fn is_expired(&self, skew_secs: i64) -> bool {
        Utc::now().timestamp() + skew_secs >= self.expires_at
    }
}

/// File-backed token storage with the strongest protection available locally.
pub struct TokenStore {
    path: PathBuf,
}

impl TokenStore {
    pub fn new(data_dir: &Path) -> Self {
        let name = if cfg!(windows) {
            "google-tokens.bin"
        } else {
            "google-tokens.json"
        };
        Self {
            path: data_dir.join(name),
        }
    }

    /// Only the tests need the raw path; production code goes through
    /// load/save/clear so the protection scheme stays encapsulated.
    #[cfg(test)]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> io::Result<Option<TokenSet>> {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        };
        let plain = decode(&bytes)?;
        serde_json::from_slice(&plain)
            .map(Some)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))
    }

    pub fn save(&self, tokens: &TokenSet) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let plain = serde_json::to_vec(tokens)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let bytes = encode(&plain)?;

        // Write-then-rename so a crash cannot leave a truncated credential file.
        let temp = self.path.with_extension("tmp");
        write_private(&temp, &bytes)?;
        std::fs::rename(&temp, &self.path)?;
        Ok(())
    }

    pub fn clear(&self) -> io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

#[cfg(windows)]
fn encode(plain: &[u8]) -> io::Result<Vec<u8>> {
    dpapi::protect(plain)
}

#[cfg(not(windows))]
fn encode(plain: &[u8]) -> io::Result<Vec<u8>> {
    Ok(plain.to_vec())
}

#[cfg(windows)]
fn decode(bytes: &[u8]) -> io::Result<Vec<u8>> {
    dpapi::unprotect(bytes)
}

#[cfg(not(windows))]
fn decode(bytes: &[u8]) -> io::Result<Vec<u8>> {
    Ok(bytes.to_vec())
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(windows)]
mod dpapi {
    //! Minimal DPAPI binding. Declaring the two functions directly avoids a
    //! heavy dependency and keeps the behaviour identical across Rust versions.

    use std::ffi::c_void;
    use std::io;
    use std::ptr;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            data_in: *const DataBlob,
            data_description: *const u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
        fn CryptUnprotectData(
            data_in: *const DataBlob,
            data_description: *mut *mut u16,
            optional_entropy: *const DataBlob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    pub fn protect(plain: &[u8]) -> io::Result<Vec<u8>> {
        if plain.is_empty() {
            return Ok(Vec::new());
        }
        let input = DataBlob {
            cb_data: plain.len() as u32,
            pb_data: plain.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };
        // SAFETY: both blobs point at valid buffers for the duration of the call.
        let ok = unsafe {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success DPAPI allocated `output.cb_data` bytes.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
        // SAFETY: the buffer came from DPAPI and is freed exactly once.
        unsafe {
            LocalFree(output.pb_data as *mut c_void);
        }
        Ok(bytes)
    }

    pub fn unprotect(blob: &[u8]) -> io::Result<Vec<u8>> {
        if blob.is_empty() {
            return Ok(Vec::new());
        }
        let input = DataBlob {
            cb_data: blob.len() as u32,
            pb_data: blob.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };
        // SAFETY: valid input buffer; output is initialised by Windows.
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: on success DPAPI allocated `output.cb_data` bytes.
        let bytes =
            unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize).to_vec() };
        // SAFETY: the buffer came from DPAPI and is freed exactly once.
        unsafe {
            LocalFree(output.pb_data as *mut c_void);
        }
        Ok(bytes)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn protects_and_unprotects_a_token() {
            let secret = br#"{"access_token":"a","refresh_token":"r"}"#;
            let sealed = protect(secret).unwrap();
            assert_ne!(sealed, secret);
            assert_eq!(unprotect(&sealed).unwrap(), secret);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> TokenSet {
        TokenSet {
            access_token: "access-value".to_string(),
            refresh_token: Some("refresh-value".to_string()),
            expires_at: 1_900_000_000,
            scope: Some("drive.appdata".to_string()),
            token_type: Some("Bearer".to_string()),
        }
    }

    #[test]
    fn stores_and_clears_a_token_set() {
        let dir = std::env::temp_dir().join(format!("scout-tokens-{}", uuid::Uuid::new_v4()));
        let store = TokenStore::new(&dir);
        assert!(store.load().unwrap().is_none());
        store.save(&tokens()).unwrap();
        let loaded = store.load().unwrap().expect("token file");
        assert_eq!(loaded.access_token, "access-value");
        assert_eq!(loaded.refresh_token.as_deref(), Some("refresh-value"));
        store.clear().unwrap();
        assert!(store.load().unwrap().is_none());
        // Clearing twice is fine.
        store.clear().unwrap();
    }

    #[test]
    fn refuses_a_corrupt_token_file() {
        let dir = std::env::temp_dir().join(format!("scout-tokens-bad-{}", uuid::Uuid::new_v4()));
        let store = TokenStore::new(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(store.path(), b"not a token").unwrap();
        assert!(store.load().is_err());
    }

    #[test]
    fn expiry_uses_a_safety_skew() {
        let mut set = tokens();
        set.expires_at = Utc::now().timestamp() + 30;
        assert!(set.is_expired(60));
        assert!(!set.is_expired(0));
        set.expires_at = Utc::now().timestamp() + 3600;
        assert!(!set.is_expired(60));
    }
}
