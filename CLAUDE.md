# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

Lithos is a high-performance market microstructure monitoring system for financial trading, built in Rust. It captures real-time market data from exchanges (Binance) via WebSocket, processes it through lock-free shared memory IPC, and maintains market state for consumers.

## Build & Development Commands

```bash
cargo build --release    # Build optimized binaries (thin LTO, single codegen unit)
cargo test               # Run all tests
cargo fmt                # Format code
cargo clippy             # Lint
cargo test -p <crate>    # Run tests for a single crate (e.g., lithos-events, lithos-icc)
```

Running the binaries:
```bash
./target/release/obsidian   # Market data publisher (WebSocket → shared memory)
./target/release/onyx       # Market state consumer (shared memory → analytics)
RUST_LOG=debug ./target/release/obsidian  # Override log level
```

Config paths are currently hardcoded to `/Users/adityaanand/dev/lithos/config/{obsidian,onyx}/config.toml`.

## Architecture

```
Binance WebSocket → [OBSIDIAN] → mmap ring buffer → [ONYX] → MarketState
```

Three main components organized as a Cargo workspace:

- **bins/obsidian** — Market data ingestion. Connects to Binance WebSocket feeds, parses JSON with `sonic-rs`, publishes `TopOfBook` events to shared memory. One thread per connection.
- **bins/onyx** — Market state processor. Reads events from shared memory, updates `MarketStateManager` (fixed array of 256 slots indexed by `SymbolId`).
- **crates/lithos/** — Core infrastructure:
  - `lithos-icc` — SPMC broadcast ring buffer using seqlocks and atomic sequence numbers (lock-free). Default path: `/tmp/lithos_md_bus`, capacity 65536.
  - `lithos-events` — Binary event types (`TopOfBook`, `SymbolId`). All `#[repr(C)]`, `Copy`. `TopOfBook` must be ≤64 bytes (cache line).
  - `lithos-mmap` — Memory-mapped file abstractions via `memmap2`.
- **crates/obsidian/** — Obsidian-specific: config loading, WebSocket parsing, engine loop.
- **crates/onyx/** — Onyx-specific: config loading, market state management, engine loop.

## Key Design Constraints

- **No floats on the hot path.** Prices are `i64` ticks, quantities are `i64` lots. Mid price stored as `mid_x2 = bid + ask` to avoid division. This prevents floating-point error accumulation.
- **Fixed-size arrays over HashMaps.** `MarketStateManager` uses `[MarketState; 256]` indexed by `SymbolId(u16)` for O(1) branchless lookup. See `docs/vec_vs_hashmap.md` for rationale.
- **Events are `#[repr(C)]` `Copy` types** written directly into memory-mapped ring buffers. No serialization on the hot path.
- **Lock-free IPC.** The broadcast ring uses atomic sequence numbers, not mutexes. Single-producer multi-consumer pattern.
- **Rust nightly required.** Uses edition 2024 features. Build flags include `-C target-cpu=native`.

## Configuration

TOML files in `config/`:
- `obsidian/config.toml` — WebSocket URLs, symbol_id mappings, shm path, capacity
- `onyx/config.toml` — shm path, log level

## Key Specification

`dev_docs/phase2.md` is the comprehensive technical specification covering market state design, integer precision, clock handling, and engine architecture.
