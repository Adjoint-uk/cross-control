//! Shared types for cross-control.
//!
//! This crate contains all types shared across the cross-control workspace:
//! input events, device descriptors, screen geometry, machine identity,
//! barrier definitions, and protocol messages.

pub mod clipboard;
pub mod device;
pub mod event;
pub mod machine;
pub mod message;
pub mod screen;

pub use clipboard::{ClipboardContent, ClipboardFormat};
pub use device::{DeviceCapability, DeviceId, DeviceInfo, VirtualDeviceId};
pub use event::{
    ButtonState, CapturedEvent, InputEvent, KeyCode, MouseButton, ScrollAxis, ScrollDirection,
};
pub use machine::MachineId;
pub use message::{
    ClipboardMessage, ControlMessage, InputMessage, Message, ProtocolVersion, PROTOCOL_VERSION,
};
pub use screen::{Barrier, BarrierId, Position, ScreenEdge, ScreenGeometry};
