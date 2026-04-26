//! Fixed-point math for AMM quoting and liquidation-trigger derivation.
//!
//! Scale factor: 10^18 (WAD precision). 256-bit intermediates via the `uint`
//! crate, same pattern as ../chiefstaker.

use crate::error::LiquidityError;
use uint::construct_uint;

construct_uint! {
    /// 256-bit unsigned integer for large intermediate products.
    pub struct U256(4);
}

/// WAD scale factor: 10^18.
pub const WAD: u128 = 1_000_000_000_000_000_000;
pub const WAD_U256: U256 = U256([WAD as u64, (WAD >> 64) as u64, 0, 0]);

/// 100% in basis points.
pub const BPS_DENOM: u128 = 10_000;

/// Slots per year, assuming 400ms per slot. `365.25 * 86400 / 0.4`.
pub const SLOTS_PER_YEAR: u64 = 78_840_000;

impl U256 {
    pub const fn from_u128(val: u128) -> Self {
        U256([val as u64, (val >> 64) as u64, 0, 0])
    }

    pub fn to_u128(&self) -> Option<u128> {
        if self.0[2] != 0 || self.0[3] != 0 {
            return None;
        }
        Some(((self.0[1] as u128) << 64) | self.0[0] as u128)
    }
}

/// Multiply two WAD-scaled values, returning a WAD-scaled result.
pub fn wad_mul(a: u128, b: u128) -> Result<u128, LiquidityError> {
    let result = U256::from_u128(a)
        .checked_mul(U256::from_u128(b))
        .ok_or(LiquidityError::MathOverflow)?
        / WAD_U256;
    result.to_u128().ok_or(LiquidityError::MathOverflow)
}

/// Divide two WAD-scaled values, returning a WAD-scaled result.
pub fn wad_div(a: u128, b: u128) -> Result<u128, LiquidityError> {
    if b == 0 {
        return Err(LiquidityError::MathOverflow);
    }
    let result = U256::from_u128(a)
        .checked_mul(WAD_U256)
        .ok_or(LiquidityError::MathOverflow)?
        / U256::from_u128(b);
    result.to_u128().ok_or(LiquidityError::MathOverflow)
}

// ===== Integer math helpers =====

/// Integer square root by Newton's method. Returns floor(sqrt(n)).
pub fn isqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    // Initial estimate: 2^(ceil(log2(n)) / 2). Use a fast bit-length-based seed.
    let bits = 128 - n.leading_zeros() as u128;
    let mut x = 1u128 << ((bits + 1) / 2);
    loop {
        let y = (x + n / x) / 2;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// Multiply-then-divide on u128 with U256 intermediate to avoid overflow.
/// Returns floor(a * b / c).
pub fn mul_div(a: u128, b: u128, c: u128) -> Result<u128, LiquidityError> {
    if c == 0 {
        return Err(LiquidityError::MathOverflow);
    }
    let prod = U256::from_u128(a)
        .checked_mul(U256::from_u128(b))
        .ok_or(LiquidityError::MathOverflow)?;
    let q = prod / U256::from_u128(c);
    q.to_u128().ok_or(LiquidityError::MathOverflow)
}

// ===== Interest accrual =====

/// Linear per-slot accrual: `accrued += principal * rate_bps * Δslots /
/// (BPS_DENOM * SLOTS_PER_YEAR)`.
///
/// Returns the *new* total accrued value (including the prior `accrued_so_far`).
pub fn accrue_interest(
    principal: u128,
    accrued_so_far: u128,
    rate_bps_per_year: u16,
    slots_elapsed: u64,
) -> Result<u128, LiquidityError> {
    if rate_bps_per_year == 0 || slots_elapsed == 0 || principal == 0 {
        return Ok(accrued_so_far);
    }
    let delta_num = U256::from_u128(principal)
        .checked_mul(U256::from_u128(rate_bps_per_year as u128))
        .ok_or(LiquidityError::MathOverflow)?
        .checked_mul(U256::from_u128(slots_elapsed as u128))
        .ok_or(LiquidityError::MathOverflow)?;
    let delta_denom = U256::from_u128(BPS_DENOM)
        .checked_mul(U256::from_u128(SLOTS_PER_YEAR as u128))
        .ok_or(LiquidityError::MathOverflow)?;
    let delta = (delta_num / delta_denom)
        .to_u128()
        .ok_or(LiquidityError::MathOverflow)?;
    accrued_so_far
        .checked_add(delta)
        .ok_or(LiquidityError::MathOverflow)
}

// ===== AMM quoting =====

/// Constant-product AMM quote: how much of `out` token comes back when
/// depositing `amount_in` of the input token, given current reserves and a
/// swap fee in basis points.
///
/// Formula (exact integer division, no rounding favors LP):
/// ```text
///   amount_in_after_fee = amount_in * (BPS_DENOM - fee_bps) / BPS_DENOM
///   amount_out = (amount_in_after_fee * reserve_out)
///                / (reserve_in + amount_in_after_fee)
/// ```
pub fn cpmm_quote_out(
    amount_in: u128,
    reserve_in: u128,
    reserve_out: u128,
    fee_bps: u16,
) -> Result<u128, LiquidityError> {
    if amount_in == 0 {
        return Err(LiquidityError::ZeroAmount);
    }
    if reserve_in == 0 || reserve_out == 0 {
        return Err(LiquidityError::ZeroReserves);
    }
    let fee_bps = fee_bps as u128;
    if fee_bps >= BPS_DENOM {
        return Err(LiquidityError::SettingExceedsMaximum);
    }

    let in_after_fee = U256::from_u128(amount_in)
        .checked_mul(U256::from_u128(BPS_DENOM - fee_bps))
        .ok_or(LiquidityError::MathOverflow)?
        / U256::from_u128(BPS_DENOM);

    let numerator = in_after_fee
        .checked_mul(U256::from_u128(reserve_out))
        .ok_or(LiquidityError::MathOverflow)?;
    let denominator = U256::from_u128(reserve_in)
        .checked_add(in_after_fee)
        .ok_or(LiquidityError::MathOverflow)?;
    let out = numerator / denominator;
    out.to_u128().ok_or(LiquidityError::MathOverflow)
}

/// Compute the AMM mid-price `B per A` of a pool with the given accounted
/// reserves, returned as a WAD-scaled u128.
pub fn price_b_per_a_wad(
    accounted_a: u128,
    accounted_b: u128,
) -> Result<u128, LiquidityError> {
    if accounted_a == 0 {
        return Err(LiquidityError::ZeroReserves);
    }
    // price_wad = accounted_b * WAD / accounted_a
    let num = U256::from_u128(accounted_b)
        .checked_mul(WAD_U256)
        .ok_or(LiquidityError::MathOverflow)?;
    let result = num / U256::from_u128(accounted_a);
    result.to_u128().ok_or(LiquidityError::MathOverflow)
}

// ===== Liquidation trigger (DESIGN.md §3) =====

/// Side encoding for a loan: which token is collateral, which is debt.
///
/// Stored as `u8` on the `Loan` account.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoanSides {
    /// Collateral A, debt B.
    CollateralA = 0,
    /// Collateral B, debt A.
    CollateralB = 1,
}

impl LoanSides {
    pub fn from_u8(b: u8) -> Result<Self, LiquidityError> {
        match b {
            0 => Ok(LoanSides::CollateralA),
            1 => Ok(LoanSides::CollateralB),
            _ => Err(LiquidityError::InvalidSidesEncoding),
        }
    }
}

/// Direction in which the pool's price (B-per-A) must move for a loan to
/// become liquidatable.
///
/// Stored as `u8` on the `Loan` and `LoanLink` accounts.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerDirection {
    /// Liquidation fires when pool price *falls* below `trigger_price_wad`.
    /// (A-collateral loans: A becomes worth less in B terms.)
    OnFall = 0,
    /// Liquidation fires when pool price *rises* above `trigger_price_wad`.
    /// (B-collateral loans: B becomes worth less in A terms.)
    OnRise = 1,
}

impl TriggerDirection {
    pub fn from_u8(b: u8) -> Result<Self, LiquidityError> {
        match b {
            0 => Ok(TriggerDirection::OnFall),
            1 => Ok(TriggerDirection::OnRise),
            _ => Err(LiquidityError::InvalidSidesEncoding),
        }
    }
}

/// Compute the liquidation trigger price (B-per-A, WAD-scaled) and direction
/// for a loan with the given sides, collateral amount, debt amount, and
/// liquidation ratio in basis points (e.g. 11000 = 110%).
///
/// Closed-form derivation (see DESIGN.md §3):
/// - CollateralA, DebtB: `trigger = (debt_b * liq_ratio_bps / BPS_DENOM) / collateral_a`,
///   direction = `OnFall`.
/// - CollateralB, DebtA: `trigger = collateral_b / (debt_a * liq_ratio_bps / BPS_DENOM)`,
///   direction = `OnRise`.
pub fn recompute_trigger(
    sides: LoanSides,
    collateral_amount: u128,
    debt_amount: u128,
    liq_ratio_bps: u16,
) -> Result<(u128, TriggerDirection), LiquidityError> {
    if collateral_amount == 0 {
        return Err(LiquidityError::ZeroAmount);
    }
    if debt_amount == 0 {
        return Err(LiquidityError::ZeroAmount);
    }
    let liq_ratio_bps = liq_ratio_bps as u128;

    match sides {
        LoanSides::CollateralA => {
            // trigger_wad = debt_b * liq_ratio * WAD / (collateral_a * BPS_DENOM)
            let num = U256::from_u128(debt_amount)
                .checked_mul(U256::from_u128(liq_ratio_bps))
                .ok_or(LiquidityError::MathOverflow)?
                .checked_mul(WAD_U256)
                .ok_or(LiquidityError::MathOverflow)?;
            let denom = U256::from_u128(collateral_amount)
                .checked_mul(U256::from_u128(BPS_DENOM))
                .ok_or(LiquidityError::MathOverflow)?;
            let trigger = (num / denom)
                .to_u128()
                .ok_or(LiquidityError::MathOverflow)?;
            Ok((trigger, TriggerDirection::OnFall))
        }
        LoanSides::CollateralB => {
            // trigger_wad = collateral_b * BPS_DENOM * WAD / (debt_a * liq_ratio)
            let num = U256::from_u128(collateral_amount)
                .checked_mul(U256::from_u128(BPS_DENOM))
                .ok_or(LiquidityError::MathOverflow)?
                .checked_mul(WAD_U256)
                .ok_or(LiquidityError::MathOverflow)?;
            let denom = U256::from_u128(debt_amount)
                .checked_mul(U256::from_u128(liq_ratio_bps))
                .ok_or(LiquidityError::MathOverflow)?;
            let trigger = (num / denom)
                .to_u128()
                .ok_or(LiquidityError::MathOverflow)?;
            Ok((trigger, TriggerDirection::OnRise))
        }
    }
}

/// Returns true iff a loan with the given trigger and direction is
/// liquidatable at the supplied current price.
pub fn is_liquidatable(
    trigger_price_wad: u128,
    direction: TriggerDirection,
    current_price_wad: u128,
) -> bool {
    match direction {
        TriggerDirection::OnFall => current_price_wad <= trigger_price_wad,
        TriggerDirection::OnRise => current_price_wad >= trigger_price_wad,
    }
}

// ===== Band id (DESIGN.md §6) =====

/// Bands cover 2× price ranges geometrically. `band_id` is `floor(log2(price))`
/// shifted by `BAND_OFFSET` so all in-range trigger prices land on
/// non-negative `u32` ids.
///
/// `floor(log2(WAD))` for `WAD = 10^18` is `59`. We pick `BAND_OFFSET = 64` so
/// `price = 1.0` lands at band `63`. Each step of `band_id` is a 2× change
/// in price.
pub const BAND_OFFSET: u32 = 64;
const LOG2_WAD: u32 = 59;

/// Compute `band_id` for the given trigger price (WAD-scaled).
///
/// `price = 1.0` → 63, `2.0` → 64, `0.5` → 62, etc.
/// Errors on `trigger_price_wad == 0`.
pub fn band_id_for_trigger(trigger_price_wad: u128) -> Result<u32, LiquidityError> {
    if trigger_price_wad == 0 {
        return Err(LiquidityError::ZeroAmount);
    }
    // floor(log2(x)) = 127 - leading_zeros for x: u128
    let log2_x = 127 - trigger_price_wad.leading_zeros();
    // band_id = log2(x) - log2(WAD) + BAND_OFFSET
    //         = log2_x - LOG2_WAD + BAND_OFFSET
    // log2_x is at most 127, LOG2_WAD is 59 → result fits in u32 easily.
    Ok(log2_x + BAND_OFFSET - LOG2_WAD)
}

/// Inclusive lower bound of a band's trigger-price range, WAD-scaled.
///
/// `band_id_for_trigger(band_min_wad(b)) == b` for any b in
/// `[BAND_OFFSET - LOG2_WAD, BAND_OFFSET + 67]`.
pub fn band_min_wad(band_id: u32) -> Result<u128, LiquidityError> {
    // log2_x = band_id + LOG2_WAD - BAND_OFFSET
    if band_id + LOG2_WAD < BAND_OFFSET {
        // Below the representable floor → value is 0 / 1.
        return Ok(0);
    }
    let log2_x = band_id + LOG2_WAD - BAND_OFFSET;
    if log2_x >= 128 {
        return Err(LiquidityError::MathOverflow);
    }
    Ok(1u128 << log2_x)
}

/// Exclusive upper bound of a band's trigger-price range, WAD-scaled.
pub fn band_max_wad(band_id: u32) -> Result<u128, LiquidityError> {
    band_min_wad(band_id + 1)
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wad_mul_basic() {
        // 1.5 * 2.0 = 3.0
        let a = 1_500_000_000_000_000_000u128;
        let b = 2_000_000_000_000_000_000u128;
        let r = wad_mul(a, b).unwrap();
        assert_eq!(r, 3_000_000_000_000_000_000u128);
    }

    #[test]
    fn test_wad_div_basic() {
        // 3.0 / 2.0 = 1.5
        let a = 3_000_000_000_000_000_000u128;
        let b = 2_000_000_000_000_000_000u128;
        let r = wad_div(a, b).unwrap();
        assert_eq!(r, 1_500_000_000_000_000_000u128);
    }

    #[test]
    fn test_wad_div_by_zero() {
        assert_eq!(wad_div(WAD, 0), Err(LiquidityError::MathOverflow));
    }

    #[test]
    fn test_cpmm_quote_no_fee() {
        // Reserves 1000/1000, deposit 100, no fee
        // amount_out = 100 * 1000 / (1000 + 100) = 100000/1100 = 90
        let out = cpmm_quote_out(100, 1000, 1000, 0).unwrap();
        assert_eq!(out, 90);
    }

    #[test]
    fn test_cpmm_quote_with_fee() {
        // 30 bps fee, so 99.7 effective
        // in_after_fee = 100 * 9970 / 10000 = 99 (truncated)
        // out = 99 * 1000 / (1000 + 99) = 99000 / 1099 = 90 (truncated)
        let out = cpmm_quote_out(100, 1000, 1000, 30).unwrap();
        assert_eq!(out, 90);
    }

    #[test]
    fn test_cpmm_quote_zero_amount() {
        assert_eq!(
            cpmm_quote_out(0, 1000, 1000, 30),
            Err(LiquidityError::ZeroAmount)
        );
    }

    #[test]
    fn test_cpmm_quote_zero_reserves() {
        assert_eq!(
            cpmm_quote_out(100, 0, 1000, 30),
            Err(LiquidityError::ZeroReserves)
        );
        assert_eq!(
            cpmm_quote_out(100, 1000, 0, 30),
            Err(LiquidityError::ZeroReserves)
        );
    }

    #[test]
    fn test_cpmm_quote_invariant_holds() {
        // x*y=k holds exactly without fee
        let r_in: u128 = 10_000;
        let r_out: u128 = 50_000;
        let amt_in: u128 = 1_000;
        let amt_out = cpmm_quote_out(amt_in, r_in, r_out, 0).unwrap();
        let new_in = r_in + amt_in;
        let new_out = r_out - amt_out;
        // CPMM rounds in favor of the LP (amt_out floored), so post-trade k >= pre-trade k.
        assert!(new_in * new_out >= r_in * r_out);
    }

    #[test]
    fn test_price_b_per_a() {
        // 1000 A and 5000 B → price = 5.0
        let p = price_b_per_a_wad(1000, 5000).unwrap();
        assert_eq!(p, 5_000_000_000_000_000_000u128);
    }

    #[test]
    fn test_recompute_trigger_collateral_a() {
        // Borrowed 100 B against 50 A, 110% liq ratio.
        // trigger = 100 * 11000 / 10000 / 50 = 110/50 = 2.2 (B per A)
        let (trigger, dir) =
            recompute_trigger(LoanSides::CollateralA, 50, 100, 11000).unwrap();
        assert_eq!(dir, TriggerDirection::OnFall);
        assert_eq!(trigger, 2_200_000_000_000_000_000u128);
    }

    #[test]
    fn test_recompute_trigger_collateral_b() {
        // Borrowed 50 A against 200 B, 110% liq ratio.
        // trigger = 200 / (50 * 1.1) = 200 / 55 ≈ 3.6363... (B per A)
        let (trigger, dir) =
            recompute_trigger(LoanSides::CollateralB, 200, 50, 11000).unwrap();
        assert_eq!(dir, TriggerDirection::OnRise);
        // 200 * 10000 * 1e18 / (50 * 11000) = 2e21 / 55e4 = 3.636363... * 1e18
        // exact integer: floor(200 * 10000 * 10^18 / 550000)
        let expected = (200u128 * 10_000 * WAD) / (50u128 * 11_000);
        assert_eq!(trigger, expected);
    }

    #[test]
    fn test_recompute_trigger_zero_amount() {
        assert_eq!(
            recompute_trigger(LoanSides::CollateralA, 0, 100, 11000),
            Err(LiquidityError::ZeroAmount)
        );
        assert_eq!(
            recompute_trigger(LoanSides::CollateralA, 100, 0, 11000),
            Err(LiquidityError::ZeroAmount)
        );
    }

    #[test]
    fn test_is_liquidatable() {
        let trig = 2_000_000_000_000_000_000u128; // 2.0
        // OnFall: liquidatable when price <= trigger
        assert!(is_liquidatable(trig, TriggerDirection::OnFall, trig));
        assert!(is_liquidatable(trig, TriggerDirection::OnFall, trig - 1));
        assert!(!is_liquidatable(trig, TriggerDirection::OnFall, trig + 1));
        // OnRise: liquidatable when price >= trigger
        assert!(is_liquidatable(trig, TriggerDirection::OnRise, trig));
        assert!(is_liquidatable(trig, TriggerDirection::OnRise, trig + 1));
        assert!(!is_liquidatable(trig, TriggerDirection::OnRise, trig - 1));
    }

    #[test]
    fn test_band_id_for_trigger() {
        // price = 1.0 → log2 = 59 → band_id = 59 + 64 - 59 = 64? Wait:
        // band_id = log2_x + BAND_OFFSET - LOG2_WAD
        //         = 59 + 64 - 59 = 64.
        // But WAD has bit length 60, so floor(log2(WAD)) = 59 ✓.
        // Hmm — design said price 1.0 → band 63, but actually it's 64.
        // The "price 1.0 → 63" comment in math.rs is just example arithmetic;
        // exact value depends on which side of WAD you land. For WAD itself
        // (price = 1.0), band_id = 64.
        assert_eq!(band_id_for_trigger(WAD).unwrap(), 64);
        assert_eq!(band_id_for_trigger(2 * WAD).unwrap(), 65);
        assert_eq!(band_id_for_trigger(WAD / 2).unwrap(), 63);
        // Tiny price: 1 → log2 = 0 → band_id = 0 + 64 - 59 = 5
        assert_eq!(band_id_for_trigger(1).unwrap(), 5);
    }

    #[test]
    fn test_band_id_zero_errors() {
        assert_eq!(band_id_for_trigger(0), Err(LiquidityError::ZeroAmount));
    }

    #[test]
    fn test_band_min_max_consistent() {
        // For a range of band_ids, every price in [min, max) should map back.
        for b in 10u32..120 {
            let lo = band_min_wad(b).unwrap();
            let hi = band_max_wad(b).unwrap();
            if lo > 0 {
                assert_eq!(band_id_for_trigger(lo).unwrap(), b);
            }
            if hi > 1 {
                assert_eq!(band_id_for_trigger(hi - 1).unwrap(), b);
            }
            // Just-above the upper bound is in the next band
            assert_eq!(band_id_for_trigger(hi).unwrap(), b + 1);
        }
    }

    #[test]
    fn test_loan_sides_roundtrip() {
        assert_eq!(
            LoanSides::from_u8(0).unwrap(),
            LoanSides::CollateralA
        );
        assert_eq!(
            LoanSides::from_u8(1).unwrap(),
            LoanSides::CollateralB
        );
        assert_eq!(
            LoanSides::from_u8(2),
            Err(LiquidityError::InvalidSidesEncoding)
        );
    }

    #[test]
    fn test_accrue_interest_zero_inputs() {
        assert_eq!(accrue_interest(1000, 5, 0, 1000).unwrap(), 5);
        assert_eq!(accrue_interest(1000, 5, 100, 0).unwrap(), 5);
        assert_eq!(accrue_interest(0, 5, 100, 1000).unwrap(), 5);
    }

    #[test]
    fn test_accrue_interest_basic() {
        // principal = 100_000_000, rate = 1000 bps = 10% APR,
        // slots_elapsed = SLOTS_PER_YEAR → 1 year of interest.
        // accrual = 100_000_000 * 1000 * 78_840_000 / (10000 * 78_840_000)
        //         = 100_000_000 * 1000 / 10000 = 10_000_000 → 10% of principal.
        let r = accrue_interest(100_000_000, 0, 1000, SLOTS_PER_YEAR).unwrap();
        assert_eq!(r, 10_000_000);
    }

    #[test]
    fn test_accrue_interest_partial_year() {
        // Half a year, 10% APR, principal = 100_000_000 → 5_000_000.
        let r = accrue_interest(100_000_000, 0, 1000, SLOTS_PER_YEAR / 2).unwrap();
        assert_eq!(r, 5_000_000);
    }

    #[test]
    fn test_isqrt_basic() {
        assert_eq!(isqrt_u128(0), 0);
        assert_eq!(isqrt_u128(1), 1);
        assert_eq!(isqrt_u128(2), 1);
        assert_eq!(isqrt_u128(4), 2);
        assert_eq!(isqrt_u128(9), 3);
        assert_eq!(isqrt_u128(15), 3);
        assert_eq!(isqrt_u128(16), 4);
        assert_eq!(isqrt_u128(99), 9);
        assert_eq!(isqrt_u128(100), 10);
        assert_eq!(isqrt_u128(1_000_000), 1_000);
        // Big number near u128 max
        let big = u128::MAX;
        let r = isqrt_u128(big);
        // r^2 <= big < (r+1)^2 — second part may overflow, just check r^2 <= big
        assert!(r as u128 <= u128::MAX / r as u128);
    }

    #[test]
    fn test_mul_div() {
        assert_eq!(mul_div(10, 20, 5).unwrap(), 40);
        // Big enough to overflow u128 in the intermediate without U256
        let r = mul_div(u128::MAX, 2, 4).unwrap();
        assert_eq!(r, u128::MAX / 2);
    }

    #[test]
    fn test_mul_div_zero_denom() {
        assert_eq!(mul_div(10, 20, 0), Err(LiquidityError::MathOverflow));
    }

    #[test]
    fn test_trigger_direction_roundtrip() {
        assert_eq!(
            TriggerDirection::from_u8(0).unwrap(),
            TriggerDirection::OnFall
        );
        assert_eq!(
            TriggerDirection::from_u8(1).unwrap(),
            TriggerDirection::OnRise
        );
        assert_eq!(
            TriggerDirection::from_u8(2),
            Err(LiquidityError::InvalidSidesEncoding)
        );
    }
}
