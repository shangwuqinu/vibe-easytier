use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(unix)]
use std::fs::File;

use base64::{engine::general_purpose::STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    crypto::{CryptoError, StateProtector},
    profile::{NetworkProfile, ProfileError},
    security::harden_service_path,
};

pub const STATE_SCHEMA_VERSION: u32 = 1;
const ENVELOPE_FORMAT_VERSION: u32 = 1;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersistedState {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub profiles: BTreeMap<String, NetworkProfile>,
    #[serde(default)]
    pub active_profile_id: Option<String>,
}

impl PersistedState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn validate(&self) -> Result<(), StateError> {
        if self.schema_version != STATE_SCHEMA_VERSION {
            return Err(StateError::UnsupportedSchema(self.schema_version));
        }

        let mut auto_connect_profile_id = None;
        for (id, profile) in &self.profiles {
            if &profile.id != id {
                return Err(StateError::InvalidState(format!(
                    "profile map key {id:?} does not match profile id {:?}",
                    profile.id
                )));
            }
            profile.validate()?;
            if profile.auto_connect {
                if auto_connect_profile_id.replace(id.as_str()).is_some() {
                    return Err(StateError::InvalidState(
                        "the slim client supports one auto-connected private network".to_owned(),
                    ));
                }
            }
        }
        if let Some(active) = &self.active_profile_id {
            if !self.profiles.contains_key(active) {
                return Err(StateError::InvalidState(format!(
                    "active profile {active:?} does not exist"
                )));
            }
        }
        if let Some(auto_connect_profile_id) = auto_connect_profile_id {
            if self.active_profile_id.as_deref() != Some(auto_connect_profile_id) {
                return Err(StateError::InvalidState(
                    "the auto-connected profile must be the active profile".to_owned(),
                ));
            }
        }
        Ok(())
    }

    pub fn auto_connect_profile(&self) -> Option<&NetworkProfile> {
        self.profiles.values().find(|profile| profile.auto_connect)
    }
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            schema_version: STATE_SCHEMA_VERSION,
            profiles: BTreeMap::new(),
            active_profile_id: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServicePaths {
    root: PathBuf,
}

impl ServicePaths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn default_for_host() -> Self {
        #[cfg(windows)]
        {
            let root = std::env::var_os("PROGRAMDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
            return Self::new(root.join("VibeEasyTier"));
        }

        #[cfg(not(windows))]
        {
            Self::new(std::env::temp_dir().join("vibe-easytier-service"))
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn state_file(&self) -> PathBuf {
        self.root.join("state.v1.json")
    }

    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn runtime_profile_toml(&self, profile_id: &str) -> PathBuf {
        self.runtime_dir().join(format!("{profile_id}.toml"))
    }

    pub fn staged_runtime_profile_toml(&self, profile_id: &str) -> PathBuf {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        self.runtime_dir().join(format!(
            ".{profile_id}.validate.{}.{}.toml",
            std::process::id(),
            sequence
        ))
    }
}

#[derive(Debug, Error)]
pub enum StateError {
    #[error("I/O failure: {0}")]
    Io(#[from] io::Error),
    #[error("state serialization failed: {0}")]
    Serialization(String),
    #[error("state encryption failed: {0}")]
    Crypto(#[from] CryptoError),
    #[error("state schema {0} is unsupported")]
    UnsupportedSchema(u32),
    #[error("invalid persisted state: {0}")]
    InvalidState(String),
    #[error("invalid profile: {0}")]
    Profile(#[from] ProfileError),
}

#[derive(Debug, Serialize, Deserialize)]
struct EncryptedEnvelope {
    format_version: u32,
    algorithm: String,
    ciphertext: String,
}

pub struct StateStore<P> {
    paths: ServicePaths,
    protector: P,
}

impl<P: StateProtector> StateStore<P> {
    pub fn new(paths: ServicePaths, protector: P) -> Self {
        Self { paths, protector }
    }

    pub fn paths(&self) -> &ServicePaths {
        &self.paths
    }

    pub fn load_or_default(&self) -> Result<PersistedState, StateError> {
        fs::create_dir_all(self.paths.root())?;
        harden_service_path(self.paths.root())?;
        let path = self.paths.state_file();
        if !path.exists() {
            return Ok(PersistedState::new());
        }
        harden_service_path(&path)?;
        let bytes = fs::read(path)?;
        let envelope: EncryptedEnvelope = serde_json::from_slice(&bytes)
            .map_err(|error| StateError::Serialization(error.to_string()))?;
        if envelope.format_version != ENVELOPE_FORMAT_VERSION {
            return Err(StateError::InvalidState(format!(
                "unsupported envelope version {}",
                envelope.format_version
            )));
        }
        if envelope.algorithm != self.protector.algorithm() {
            return Err(StateError::InvalidState(format!(
                "state uses {}, expected {}",
                envelope.algorithm,
                self.protector.algorithm()
            )));
        }
        let ciphertext = STANDARD
            .decode(envelope.ciphertext)
            .map_err(|error| StateError::Serialization(error.to_string()))?;
        let plaintext = self.protector.unprotect(&ciphertext)?;
        let state: PersistedState = serde_json::from_slice(&plaintext)
            .map_err(|error| StateError::Serialization(error.to_string()))?;
        state.validate()?;
        Ok(state)
    }

    pub fn save(&self, state: &PersistedState) -> Result<(), StateError> {
        state.validate()?;
        let plaintext = serde_json::to_vec(state)
            .map_err(|error| StateError::Serialization(error.to_string()))?;
        let ciphertext = self.protector.protect(&plaintext)?;
        let envelope = EncryptedEnvelope {
            format_version: ENVELOPE_FORMAT_VERSION,
            algorithm: self.protector.algorithm().to_owned(),
            ciphertext: STANDARD.encode(ciphertext),
        };
        let bytes = serde_json::to_vec(&envelope)
            .map_err(|error| StateError::Serialization(error.to_string()))?;
        atomic_write(&self.paths.state_file(), &bytes)?;
        Ok(())
    }

    pub fn write_runtime_profile(&self, profile: &NetworkProfile) -> Result<PathBuf, StateError> {
        let path = self.paths.runtime_profile_toml(&profile.id);
        let content = profile.render_core_toml()?;
        atomic_write(&path, content.as_bytes())?;
        Ok(path)
    }

    /// Writes a private, service-owned config solely for `--check-config`.
    /// The caller removes it before deciding whether to replace durable state.
    pub fn write_staged_runtime_profile(
        &self,
        profile: &NetworkProfile,
    ) -> Result<PathBuf, StateError> {
        let path = self.paths.staged_runtime_profile_toml(&profile.id);
        let content = profile.render_core_toml()?;
        atomic_write(&path, content.as_bytes())?;
        Ok(path)
    }
}

pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "durable state path must have a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    harden_service_path(parent)?;
    let temporary = temporary_path(path);

    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        harden_service_path(&temporary)?;
        file.write_all(bytes)?;
        file.flush()?;
        file.sync_all()?;
        drop(file);
        replace_file(&temporary, path)?;
        harden_service_path(path)?;
        sync_parent(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("state");
    path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), sequence))
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let moved = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(unix)]
fn sync_parent(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_parent: &Path) -> io::Result<()> {
    Ok(())
}

fn default_schema_version() -> u32 {
    STATE_SCHEMA_VERSION
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        crypto::TestProtector,
        profile::{AddressMode, SecretString},
    };

    fn test_paths(name: &str) -> ServicePaths {
        ServicePaths::new(std::env::temp_dir().join(format!(
            "vibe-easytier-service-{name}-{}",
            std::process::id()
        )))
    }

    fn profile() -> NetworkProfile {
        NetworkProfile {
            id: "home".to_owned(),
            name: "Home".to_owned(),
            instance_name: "home".to_owned(),
            hostname: "laptop".to_owned(),
            network_name: "private-home".to_owned(),
            network_secret: SecretString::new("only-in-encrypted-state"),
            address_mode: AddressMode::Static {
                cidr: "10.44.0.2/24".to_owned(),
            },
            peers: vec!["tcp://seed.example.net:11010".to_owned()],
            flags: crate::profile::EasyTierFlags::default(),
            auto_connect: true,
        }
    }

    #[test]
    fn state_round_trip_is_encrypted_and_atomic() {
        let paths = test_paths("round-trip");
        let _ = fs::remove_dir_all(paths.root());
        let store = StateStore::new(paths.clone(), TestProtector);
        let mut state = PersistedState::new();
        state.active_profile_id = Some("home".to_owned());
        state.profiles.insert("home".to_owned(), profile());

        store.save(&state).unwrap();
        let on_disk = fs::read_to_string(paths.state_file()).unwrap();
        assert!(!on_disk.contains("only-in-encrypted-state"));
        assert_eq!(store.load_or_default().unwrap(), state);

        let _ = fs::remove_dir_all(paths.root());
    }

    #[test]
    fn runtime_toml_contains_the_secret_only_in_the_service_protected_directory() {
        let paths = test_paths("runtime-toml");
        let _ = fs::remove_dir_all(paths.root());
        let store = StateStore::new(paths.clone(), TestProtector);

        let path = store.write_runtime_profile(&profile()).unwrap();
        let content = fs::read_to_string(path).unwrap();
        assert!(content.contains("network_secret = \"only-in-encrypted-state\""));

        let _ = fs::remove_dir_all(paths.root());
    }

    #[test]
    fn state_rejects_multiple_auto_connect_profiles() {
        let mut state = PersistedState::new();
        state.profiles.insert("home".to_owned(), profile());
        let mut second = profile();
        second.id = "office".to_owned();
        second.instance_name = "office".to_owned();
        state.profiles.insert("office".to_owned(), second);

        assert!(matches!(state.validate(), Err(StateError::InvalidState(_))));
    }
}
