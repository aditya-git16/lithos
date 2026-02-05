//! `lithos-icc`: Inter-Component Communication primitives for Lithos.
//!
//! This crate provides high-performance, lock-free data structures for
//! communication between components, particularly suited for:
//! - Single-producer, multi-consumer (SPMC) broadcast patterns
//! - Inter-process communication via shared memory (mmap)
//! - Low-latency market data distribution
//!
//! # Core Components
//!
//! - [`BroadcastWriter`]: Creates and writes to a broadcast ring buffer
//! - [`BroadcastReader`]: Reads from an existing broadcast ring buffer
//! - [`RingConfig`]: Configuration for ring buffer capacity
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────┐      shared memory file      ┌─────────────────┐
//! │ BroadcastWriter │ ──────────────────────────── │ BroadcastReader │
//! │   (Process A)   │        (mmap region)         │   (Process B)   │
//! └─────────────────┘                              └─────────────────┘
//!                                                  ┌─────────────────┐
//!                                                  │ BroadcastReader │
//!                                                  │   (Process C)   │
//!                                                  └─────────────────┘
//! ```
//!
//! # Example
//!
//! ```ignore
//! use lithos_icc::{BroadcastWriter, BroadcastReader, RingConfig};
//!
//! // Writer (typically in one process)
//! let cfg = RingConfig::new(1024);
//! let mut writer = BroadcastWriter::<u64>::create("/tmp/ring.bin", cfg)?;
//! writer.publish(42);
//!
//! // Reader (can be in same or different process)
//! let mut reader = BroadcastReader::<u64>::open("/tmp/ring.bin")?;
//! if let Some(value) = reader.try_read() {
//!     println!("Received: {}", value);
//! }
//! ```
//!
//! # Internal Modules
//!
//! - `broadcast`: SPMC broadcast ring implementation
//! - `ring`: Ring buffer configuration and index arithmetic
//! - `seqlock`: Sequence lock for lock-free reader/writer synchronization
//! - `shm_layout`: Shared memory binary layout definitions

mod broadcast;
mod ring;
mod seqlock;
mod shm_layout;

pub use broadcast::{BroadcastReader, BroadcastWriter};
pub use ring::RingConfig;
