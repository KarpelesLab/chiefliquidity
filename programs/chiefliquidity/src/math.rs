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
