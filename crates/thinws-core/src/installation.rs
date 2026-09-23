use crate::{AbsolutePath, InstanceId, UnixMillis, VolumeId};

/// Identity shared by bootstrap config, root marker and SQLite installation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallationIdentity {
    instance_id: InstanceId,
    data_root: AbsolutePath,
    volume_id: VolumeId,
}

impl InstallationIdentity {
    /// Creates a fully typed installation identity.
    #[must_use]
    pub const fn new(
        instance_id: InstanceId,
        data_root: AbsolutePath,
        volume_id: VolumeId,
    ) -> Self {
        Self {
            instance_id,
            data_root,
            volume_id,
        }
    }

    /// Returns the installation identifier.
    #[must_use]
    pub const fn instance_id(&self) -> InstanceId {
        self.instance_id
    }

    /// Returns the canonical raw data-root path.
    #[must_use]
    pub const fn data_root(&self) -> &AbsolutePath {
        &self.data_root
    }

    /// Returns the APFS volume identifier.
    #[must_use]
    pub const fn volume_id(&self) -> VolumeId {
        self.volume_id
    }
}

/// Installation identity plus the database creation timestamp.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InstallationRecord {
    identity: InstallationIdentity,
    created_at: UnixMillis,
}

impl InstallationRecord {
    /// Creates an installation record for first schema initialization.
    #[must_use]
    pub const fn new(identity: InstallationIdentity, created_at: UnixMillis) -> Self {
        Self {
            identity,
            created_at,
        }
    }

    /// Returns the shared installation identity.
    #[must_use]
    pub const fn identity(&self) -> &InstallationIdentity {
        &self.identity
    }

    /// Returns the database creation timestamp.
    #[must_use]
    pub const fn created_at(&self) -> UnixMillis {
        self.created_at
    }
}

/// Durable lifecycle state of the data-root ownership marker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RootMarkerState {
    /// Initialization has started but has not published a usable installation.
    Initializing,
    /// All initialization components were verified and published.
    Ready,
}

/// Parsed data-root marker content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RootMarker {
    identity: InstallationIdentity,
    state: RootMarkerState,
}

impl RootMarker {
    /// Creates a marker with an explicit durable state.
    #[must_use]
    pub const fn new(identity: InstallationIdentity, state: RootMarkerState) -> Self {
        Self { identity, state }
    }

    /// Returns the marker's installation identity.
    #[must_use]
    pub const fn identity(&self) -> &InstallationIdentity {
        &self.identity
    }

    /// Returns the marker's lifecycle state.
    #[must_use]
    pub const fn state(&self) -> RootMarkerState {
        self.state
    }
}
