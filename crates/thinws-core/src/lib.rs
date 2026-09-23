#![forbid(unsafe_code)]
#![deny(missing_docs)]

//! Pure Phase 1 domain types for ThinWorkspace.

mod diagnostic;
mod id;
mod workspace_name;

pub use diagnostic::{ContextValue, CoreError, ErrorCode};
pub use id::{IdParseError, InstanceId, OperationId, WorkspaceId};
pub use workspace_name::{WorkspaceName, WorkspaceNameError};
