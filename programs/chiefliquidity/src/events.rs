//! Structured binary log events emitted via sol_log_data.
//!
//! Each event has a unique 8-byte discriminator (random sentinels — not
//! Anchor-derived). Off-chain consumers identify events by reading the first
//! 8 bytes of each `Program data:` log line.
