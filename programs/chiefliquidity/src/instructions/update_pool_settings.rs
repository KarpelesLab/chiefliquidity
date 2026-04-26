//! Authority-only: retune fee, liquidation, LTV, and interest parameters.
//!
//! Bounds match `InitializePool::validate_params`. Changes apply
//! prospectively: existing loans keep their stored `trigger_price_wad`
//! (so liq_ratio_bps changes don't retroactively re-bucket open loans);
//! interest accrual since `last_accrual_slot` is computed at the rate in
//! effect at the time of the next touch (open / repay / liquidation).

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::{next_account_info, AccountInfo},
    clock::Clock,
    entrypoint::ProgramResult,
    msg,
    pubkey::Pubkey,
    sysvar::Sysvar,
};

use crate::{
    error::LiquidityError,
    instructions::initialize_pool::validate_params,
    state::Pool,
};

#[allow(clippy::too_many_arguments)]
pub fn process_update_pool_settings(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    swap_fee_bps: u16,
    protocol_fee_bps: u16,
    liq_ratio_bps: u16,
    liq_penalty_bps: u16,
    max_ltv_bps: u16,
    interest_rate_bps_per_year: u16,
) -> ProgramResult {
    let it = &mut accounts.iter();
    let pool_info = next_account_info(it)?;
    let authority_info = next_account_info(it)?;

    if !authority_info.is_signer {
        return Err(LiquidityError::MissingRequiredSigner.into());
    }
    if pool_info.owner != program_id {
        return Err(LiquidityError::InvalidAccountOwner.into());
    }

    let mut pool = {
        let data = pool_info.try_borrow_data()?;
        Pool::try_from_slice(&data).map_err(|_| LiquidityError::AccountDataTooSmall)?
    };
    if !pool.is_initialized() {
        return Err(LiquidityError::NotInitialized.into());
    }
    if pool.is_authority_renounced() {
        return Err(LiquidityError::AuthorityRenounced.into());
    }
    if pool.authority != *authority_info.key {
        return Err(LiquidityError::InvalidAuthority.into());
    }

    validate_params(
        swap_fee_bps,
        protocol_fee_bps,
        liq_ratio_bps,
        liq_penalty_bps,
        max_ltv_bps,
        interest_rate_bps_per_year,
    )?;

    pool.swap_fee_bps = swap_fee_bps;
    pool.protocol_fee_bps = protocol_fee_bps;
    pool.liq_ratio_bps = liq_ratio_bps;
    pool.liq_penalty_bps = liq_penalty_bps;
    pool.max_ltv_bps = max_ltv_bps;
    pool.interest_rate_bps_per_year = interest_rate_bps_per_year;
    pool.last_update_slot = Clock::get()?.slot;
    let mut data = pool_info.try_borrow_mut_data()?;
    pool.serialize(&mut &mut data[..])?;

    msg!(
        "UpdatePoolSettings swap_fee={} prot_fee={} liq_ratio={} liq_pen={} max_ltv={} rate={}",
        swap_fee_bps,
        protocol_fee_bps,
        liq_ratio_bps,
        liq_penalty_bps,
        max_ltv_bps,
        interest_rate_bps_per_year
    );
    Ok(())
}
