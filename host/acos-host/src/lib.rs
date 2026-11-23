//! Backend-independent ACOS computer model and lifecycle contract.

pub mod api;
pub mod qemu;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Stable identifier assigned by the host to a computer.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ComputerId(String);

impl ComputerId {
    pub fn new(value: impl Into<String>) -> Result<Self, ComputerError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(ComputerError::InvalidId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ComputerId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Execution boundary exposed by the unified Computer API.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerKind {
    #[serde(rename = "vm")]
    VirtualMachine,
    #[serde(rename = "container")]
    Container,
}

/// Host-observed lifecycle state. `Unknown` represents a backend whose state
/// could not be determined; it must never be treated as stopped.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerState {
    Creating,
    Stopped,
    Starting,
    Running,
    Stopping,
    Paused,
    Failed,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ResourceLimits {
    pub vcpus: u16,
    pub memory_mib: u32,
}

impl ResourceLimits {
    pub fn validate(&self) -> Result<(), ComputerError> {
        if self.vcpus == 0 || self.memory_mib < 64 {
            return Err(ComputerError::InvalidResources);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Computer {
    pub id: ComputerId,
    pub name: String,
    pub kind: ComputerKind,
    pub state: ComputerState,
    pub resources: ResourceLimits,
    pub backend: String,
}

/// Validated request passed from the host service to a backend adapter.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreateComputer {
    pub id: ComputerId,
    pub name: String,
    pub kind: ComputerKind,
    pub resources: ResourceLimits,
    /// VM image identifier or OCI image reference, interpreted by the backend.
    pub image: String,
}

/// Backend failure with a stable category and a diagnostic suitable for logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendError {
    pub code: String,
    pub message: String,
}

/// Adapter contract implemented by Virtualization.framework, QEMU/KVM and
/// OCI runtime backends. Implementations report observed state after actions.
pub trait ComputerBackend: Send + Sync {
    fn name(&self) -> &'static str;
    fn supports(&self, kind: ComputerKind) -> bool;
    fn create(&self, request: &CreateComputer) -> Result<Computer, BackendError>;
    fn inspect(&self, id: &ComputerId) -> Result<Computer, BackendError>;
    fn start(&self, id: &ComputerId) -> Result<Computer, BackendError>;
    fn stop(&self, id: &ComputerId) -> Result<Computer, BackendError>;
    fn pause(&self, id: &ComputerId) -> Result<Computer, BackendError>;
    fn resume(&self, id: &ComputerId) -> Result<Computer, BackendError>;
    fn delete(&self, id: &ComputerId) -> Result<(), BackendError>;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuestCommandRequest {
    pub argv: Vec<String>,
    pub working_directory: Option<String>,
    #[serde(default)]
    pub environment: std::collections::BTreeMap<String, String>,
    pub timeout_ms: Option<u64>,
    pub output_limit_bytes: Option<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct GuestCommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
    pub truncated: bool,
}

pub trait CommandBackend: Send + Sync {
    fn execute(
        &self,
        id: &ComputerId,
        request: &GuestCommandRequest,
    ) -> Result<GuestCommandResult, BackendError>;
}

pub trait FileBackend: Send + Sync {
    fn read_file(&self, id: &ComputerId, path: &str) -> Result<Vec<u8>, BackendError>;
    fn write_file(&self, id: &ComputerId, path: &str, contents: &[u8]) -> Result<(), BackendError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ComputerError {
    InvalidId,
    InvalidName,
    InvalidBackend,
    InvalidResources,
    AlreadyExists,
    NotFound,
    InvalidTransition {
        from: ComputerState,
        to: ComputerState,
    },
}

impl fmt::Display for ComputerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidId => write!(
                formatter,
                "computer ID must be 1-64 ASCII letters, digits, '-' or '_'"
            ),
            Self::InvalidName => write!(
                formatter,
                "computer name must contain 1-128 non-whitespace characters"
            ),
            Self::InvalidBackend => write!(formatter, "backend name must not be empty"),
            Self::InvalidResources => write!(
                formatter,
                "resources require at least 1 vCPU and 64 MiB memory"
            ),
            Self::AlreadyExists => write!(formatter, "computer already exists"),
            Self::NotFound => write!(formatter, "computer not found"),
            Self::InvalidTransition { from, to } => write!(
                formatter,
                "invalid computer state transition: {from:?} -> {to:?}"
            ),
        }
    }
}

impl std::error::Error for ComputerError {}

/// In-memory metadata registry used by the first host-core milestone.
/// Persistence and backend calls are deliberately supplied by later layers.
#[derive(Default)]
pub struct ComputerRegistry {
    computers: BTreeMap<ComputerId, Computer>,
}

impl ComputerRegistry {
    pub fn create(&mut self, computer: Computer) -> Result<(), ComputerError> {
        validate_computer(&computer)?;
        if self.computers.contains_key(&computer.id) {
            return Err(ComputerError::AlreadyExists);
        }
        if computer.state != ComputerState::Creating {
            return Err(ComputerError::InvalidTransition {
                from: computer.state,
                to: ComputerState::Creating,
            });
        }
        self.computers.insert(computer.id.clone(), computer);
        Ok(())
    }

    pub fn get(&self, id: &ComputerId) -> Result<&Computer, ComputerError> {
        self.computers.get(id).ok_or(ComputerError::NotFound)
    }

    pub fn list(&self) -> impl Iterator<Item = &Computer> {
        self.computers.values()
    }

    pub fn transition(
        &mut self,
        id: &ComputerId,
        next: ComputerState,
    ) -> Result<&Computer, ComputerError> {
        let computer = self.computers.get_mut(id).ok_or(ComputerError::NotFound)?;
        if !is_valid_transition(computer.state, next) {
            return Err(ComputerError::InvalidTransition {
                from: computer.state,
                to: next,
            });
        }
        computer.state = next;
        Ok(computer)
    }

    pub fn remove(&mut self, id: &ComputerId) -> Result<Computer, ComputerError> {
        self.computers.remove(id).ok_or(ComputerError::NotFound)
    }
}

fn validate_computer(computer: &Computer) -> Result<(), ComputerError> {
    if computer.name.trim().is_empty() || computer.name.chars().count() > 128 {
        return Err(ComputerError::InvalidName);
    }
    if computer.backend.trim().is_empty() {
        return Err(ComputerError::InvalidBackend);
    }
    computer.resources.validate()
}

fn is_valid_transition(from: ComputerState, to: ComputerState) -> bool {
    use ComputerState::*;
    matches!(
        (from, to),
        (Creating, Stopped | Failed)
            | (Stopped, Starting | Failed)
            | (Starting, Running | Stopped | Failed | Unknown)
            | (Running, Stopping | Paused | Failed | Unknown)
            | (Stopping, Stopped | Failed | Unknown)
            | (Paused, Running | Stopping | Failed | Unknown)
            | (Failed, Starting | Stopped)
            | (Unknown, Running | Stopped | Paused | Failed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn computer(id: &str) -> Computer {
        Computer {
            id: ComputerId::new(id).unwrap(),
            name: "test computer".into(),
            kind: ComputerKind::VirtualMachine,
            state: ComputerState::Creating,
            resources: ResourceLimits {
                vcpus: 2,
                memory_mib: 1024,
            },
            backend: "test".into(),
        }
    }

    #[test]
    fn registry_tracks_a_valid_lifecycle() {
        let mut registry = ComputerRegistry::default();
        let id = ComputerId::new("dev-box").unwrap();
        registry.create(computer("dev-box")).unwrap();
        registry.transition(&id, ComputerState::Stopped).unwrap();
        registry.transition(&id, ComputerState::Starting).unwrap();
        registry.transition(&id, ComputerState::Running).unwrap();
        registry.transition(&id, ComputerState::Paused).unwrap();
        registry.transition(&id, ComputerState::Running).unwrap();
        assert_eq!(registry.get(&id).unwrap().state, ComputerState::Running);
    }

    #[test]
    fn rejects_duplicate_ids_and_invalid_transitions() {
        let mut registry = ComputerRegistry::default();
        let id = ComputerId::new("box").unwrap();
        registry.create(computer("box")).unwrap();
        assert_eq!(
            registry.create(computer("box")),
            Err(ComputerError::AlreadyExists)
        );
        assert!(matches!(
            registry.transition(&id, ComputerState::Running),
            Err(ComputerError::InvalidTransition { .. })
        ));
    }

    #[test]
    fn validates_identifiers_and_resource_limits() {
        assert!(ComputerId::new("bad/id").is_err());
        assert!(
            ResourceLimits {
                vcpus: 0,
                memory_mib: 512
            }
            .validate()
            .is_err()
        );
        assert!(
            ResourceLimits {
                vcpus: 1,
                memory_mib: 64
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn unknown_state_is_not_treated_as_stopped() {
        let mut registry = ComputerRegistry::default();
        let id = ComputerId::new("box").unwrap();
        registry.create(computer("box")).unwrap();
        registry.transition(&id, ComputerState::Stopped).unwrap();
        registry.transition(&id, ComputerState::Starting).unwrap();
        registry.transition(&id, ComputerState::Unknown).unwrap();
        assert!(matches!(
            registry.transition(&id, ComputerState::Starting),
            Err(ComputerError::InvalidTransition { .. })
        ));
    }
}
