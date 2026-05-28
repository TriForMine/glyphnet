//! Core protocol types for GlyphNet.
//!
//! `glyphnet-core` is intentionally small and deterministic. It defines the
//! wire format, symbol matrix model, mode descriptors, and layout invariants
//! shared by encoders, decoders, renderers, scanners, and SDK bindings.

pub mod auth;
pub mod bitstream;
pub mod burst_packet;
pub mod descriptor;
pub mod error;
pub mod frame;
pub mod geometry;
pub mod layout;
pub mod matrix;
pub mod mode;
pub mod profile;

pub use auth::{
    AuthEnvelopeHeader, open_payload as open_authenticated_payload,
    seal_payload as seal_authenticated_payload,
};
pub use burst_packet::{
    BURST_PACKET_HEADER_LEN, BURST_PACKET_MAGIC, BURST_PACKET_VERSION, BurstPacket,
    BurstPacketHeader,
};
pub use descriptor::{
    Capability, CapabilitySet, ColorEncoding, LayoutFamily, ProtocolVersion, SymbolDescriptor,
};
pub use error::{GlyphError, Result};
pub use frame::{Frame, FrameHeader, HEADER_LEN, MAGIC, WIRE_VERSION};
pub use geometry::{GeometryProfile, SymbolGeometry, choose_symbol_geometry};
pub use matrix::{Cell, SymbolMatrix};
pub use mode::{EccLevel, TransmissionMode};
pub use profile::{
    BenchmarkTarget, ProfileId, ProfileSpec, UseCase, profile_catalog, profile_spec,
};
