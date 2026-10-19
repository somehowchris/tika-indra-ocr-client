//! Small resource-control primitives shared by the HTTP service and inference workers.
//!
//! These types own no heap storage. Request payloads and model tensors belong to
//! separately bounded, reusable worker buffers.

pub mod batch;
pub mod lifecycle;
pub mod queue;
