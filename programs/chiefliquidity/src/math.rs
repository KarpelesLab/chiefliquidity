//! Fixed-point math for AMM quoting and liquidation-trigger derivation.
//!
//! Scale factor: 10^18 (WAD precision). 256-bit intermediates via the `uint`
//! crate, same pattern as ../chiefstaker.
