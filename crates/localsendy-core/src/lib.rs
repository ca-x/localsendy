//! Protocol-facing building blocks for Localsendy.
//!
//! The official `localsend` crate owns the LocalSend wire implementation. This
//! crate only provides the small amount of application-facing state needed by
//! the web service, plus explicit conversions to the official DTOs.

mod identity;
mod receiver;
mod types;

pub use identity::{DeviceIdentity, IdentityMaterial};
pub use receiver::{
    IncomingTransfer, IncomingTransferStatus, PendingTransfer, ReceiverHandle, ReceiverState,
    start_receiver,
};
pub use types::{
    AnnouncementMessage, DeviceInfo, DeviceType, FileId, FileMetadata, PROTOCOL_VERSION, Protocol,
    ReceivedFile,
};

pub use localsend;
