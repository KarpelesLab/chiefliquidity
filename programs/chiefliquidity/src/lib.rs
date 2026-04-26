//! ChiefLiquidity: liquidation-aware AMM lending protocol.
//!
//! Each pool holds two arbitrary SPL tokens, accepts LP deposits, accepts
//! collateralized borrows of either side against the other, and executes
//! swaps against a post-liquidation reserve state — see `DESIGN.md`.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg,
    program_error::ProgramError, pubkey::Pubkey,
};

pub mod error;
pub mod events;
pub mod instructions;
pub mod math;
pub mod state;

use instructions::*;

// Matches target/deploy/chiefliquidity-keypair.json. Regenerate the keypair
// (and update this constant) before publishing to a public cluster.
solana_program::declare_id!("D8K39AXioKew7kLfKEjsBtW3BuDXnYqntk2z4PWxzPAW");

/// Program instructions.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub enum LiquidityInstruction {
    /// Initialize a new (mint_a, mint_b) pool. Mints must be sorted; the
    /// program enforces `mint_a < mint_b` so the pool PDA is canonical.
    ///
    /// Accounts:
    /// 0. `[writable]` Pool account (PDA: ["pool", mint_a, mint_b])
    /// 1. `[]` Mint A
    /// 2. `[]` Mint B
    /// 3. `[writable]` Vault A (PDA: ["vault_a", pool])
    /// 4. `[writable]` Vault B (PDA: ["vault_b", pool])
    /// 5. `[writable]` LP mint (PDA: ["lp_mint", pool])
    /// 6. `[writable, signer]` Authority/payer
    /// 7. `[]` System program
    /// 8. `[]` Token program
    /// 9. `[]` Rent sysvar
    InitializePool {
        swap_fee_bps: u16,
        protocol_fee_bps: u16,
        liq_ratio_bps: u16,
        liq_penalty_bps: u16,
        max_ltv_bps: u16,
        interest_rate_bps_per_year: u16,
    },

    /// Deposit `amount_a_max` of A and `amount_b_max` of B (or proportional
    /// fraction thereof) and receive LP tokens.
    ///
    /// First deposit: takes both maxes verbatim and mints `sqrt(a*b)` LP.
    /// Subsequent: takes the larger ratio-matched fraction and mints
    /// `min(a/A, b/B) * lp_supply`.
    ///
    /// Accounts:
    /// 0. `[writable]` Pool
    /// 1. `[writable]` Vault A
    /// 2. `[writable]` Vault B
    /// 3. `[writable]` LP mint
    /// 4. `[writable]` User token account A (source)
    /// 5. `[writable]` User token account B (source)
    /// 6. `[writable]` User LP token account (destination)
    /// 7. `[signer]`   User
    /// 8. `[]`         Mint A
    /// 9. `[]`         Mint B
    /// 10. `[]`        Token program
    AddLiquidity {
        amount_a_max: u64,
        amount_b_max: u64,
        min_lp_out: u64,
    },

    /// Burn `lp_amount` of LP and receive proportional shares of accounted
    /// reserves. Reverts if real-reserve balances cannot cover the
    /// withdrawal (e.g. pool is heavily lent out).
    ///
    /// Accounts:
    /// 0. `[writable]` Pool
    /// 1. `[writable]` Vault A
    /// 2. `[writable]` Vault B
    /// 3. `[writable]` LP mint
    /// 4. `[writable]` User token account A (destination)
    /// 5. `[writable]` User token account B (destination)
    /// 6. `[writable]` User LP token account (source)
    /// 7. `[signer]`   User
    /// 8. `[]`         Mint A
    /// 9. `[]`         Mint B
    /// 10. `[]`        Token program
    RemoveLiquidity {
        lp_amount: u64,
        min_a_out: u64,
        min_b_out: u64,
    },
}

#[cfg(not(feature = "no-entrypoint"))]
entrypoint!(process_instruction);

#[cfg(not(feature = "no-entrypoint"))]
use solana_security_txt::security_txt;

#[cfg(not(feature = "no-entrypoint"))]
security_txt! {
    name: "ChiefLiquidity",
    project_url: "https://github.com/KarpelesLab/chiefliquidity",
    contacts: "link:https://github.com/KarpelesLab/chiefliquidity/security/advisories",
    policy: "https://github.com/KarpelesLab/chiefliquidity/security/policy",
    source_code: "https://github.com/KarpelesLab/chiefliquidity"
}

/// Program entrypoint.
pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    if program_id != &crate::id() {
        return Err(ProgramError::IncorrectProgramId);
    }

    let instruction = LiquidityInstruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    match instruction {
        LiquidityInstruction::InitializePool {
            swap_fee_bps,
            protocol_fee_bps,
            liq_ratio_bps,
            liq_penalty_bps,
            max_ltv_bps,
            interest_rate_bps_per_year,
        } => {
            msg!("Instruction: InitializePool");
            process_initialize_pool(
                program_id,
                accounts,
                swap_fee_bps,
                protocol_fee_bps,
                liq_ratio_bps,
                liq_penalty_bps,
                max_ltv_bps,
                interest_rate_bps_per_year,
            )
        }
        LiquidityInstruction::AddLiquidity {
            amount_a_max,
            amount_b_max,
            min_lp_out,
        } => {
            msg!("Instruction: AddLiquidity");
            process_add_liquidity(
                program_id,
                accounts,
                amount_a_max,
                amount_b_max,
                min_lp_out,
            )
        }
        LiquidityInstruction::RemoveLiquidity {
            lp_amount,
            min_a_out,
            min_b_out,
        } => {
            msg!("Instruction: RemoveLiquidity");
            process_remove_liquidity(
                program_id,
                accounts,
                lp_amount,
                min_a_out,
                min_b_out,
            )
        }
    }
}
