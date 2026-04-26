//! Account state structures — see `DESIGN.md` §4–§5.

use borsh::{BorshDeserialize, BorshSerialize};
use solana_program::pubkey::Pubkey;

// ===== PDA seed prefixes =====

pub const POOL_SEED: &[u8] = b"pool";
pub const VAULT_A_SEED: &[u8] = b"vault_a";
pub const VAULT_B_SEED: &[u8] = b"vault_b";
pub const LP_MINT_SEED: &[u8] = b"lp_mint";
pub const LOAN_SEED: &[u8] = b"loan";
pub const LOAN_LINK_SEED: &[u8] = b"loan_link";
pub const BAND_SEED: &[u8] = b"band";

// ===== Account discriminators (random sentinels — not Anchor-derived) =====

pub const POOL_DISCRIMINATOR: [u8; 8] = [0xa1, 0xc4, 0xe7, 0x12, 0x3b, 0x8f, 0xd5, 0x6e];
pub const LOAN_DISCRIMINATOR: [u8; 8] = [0xb2, 0x7e, 0x3c, 0xa0, 0x91, 0x4d, 0x8e, 0x55];
pub const LOAN_LINK_DISCRIMINATOR: [u8; 8] = [0xc3, 0x82, 0x1f, 0x6d, 0xb4, 0x59, 0xe2, 0x70];
pub const LOAN_INDEX_BAND_DISCRIMINATOR: [u8; 8] =
    [0xd4, 0x95, 0x3a, 0x71, 0x68, 0x2e, 0xc1, 0x88];

// ===== Foreign program IDs =====

/// The original SPL Token program ID (TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA).
pub const SPL_TOKEN_PROGRAM_ID: Pubkey = Pubkey::new_from_array([
    0x06, 0xdd, 0xf6, 0xe1, 0xd7, 0x65, 0xa1, 0x93,
    0xd9, 0xcb, 0xe1, 0x46, 0xce, 0xeb, 0x79, 0xac,
    0x1c, 0xb4, 0x85, 0xed, 0x5f, 0x5b, 0x37, 0x91,
    0x3a, 0x8c, 0xf5, 0x85, 0x7e, 0xff, 0x00, 0xa9,
]);

/// Returns true if `key` is one of the two SPL Token program IDs we accept.
pub fn is_valid_token_program(key: &Pubkey) -> bool {
    *key == spl_token_2022::id() || *key == SPL_TOKEN_PROGRAM_ID
}

// ===== Curve kinds =====

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CurveKind {
    Cpmm = 0,
}

// ===== Pool =====

/// Per-pool state. PDA: `["pool", mint_a, mint_b]` with `mint_a < mint_b`.
///
/// `accounted_x = real_x + total_debt_x` (see DESIGN.md §2). `real_x` lives
/// in the corresponding Vault SPL account; only `total_debt_x` is stored here
/// so liquidation can update it locally without re-aggregating loan balances.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct Pool {
    pub discriminator: [u8; 8],

    // Identity
    pub mint_a: Pubkey,
    pub mint_b: Pubkey,
    pub vault_a: Pubkey,
    pub vault_b: Pubkey,
    pub lp_mint: Pubkey,
    pub authority: Pubkey,

    // PDA bumps
    pub pool_bump: u8,
    pub vault_a_bump: u8,
    pub vault_b_bump: u8,
    pub lp_mint_bump: u8,

    // Reserve accounting
    pub total_debt_a: u128,
    pub total_debt_b: u128,
    pub total_collateral_a: u128,
    pub total_collateral_b: u128,

    // Curve config
    pub curve_kind: u8,
    pub swap_fee_bps: u16,
    pub protocol_fee_bps: u16,
    pub _curve_pad: [u8; 3],

    // Lending config
    pub liq_ratio_bps: u16,
    pub liq_penalty_bps: u16,
    pub max_ltv_bps: u16,
    pub interest_rate_bps_per_year: u16,
    pub _lending_pad: [u8; 8],

    // Loan-ordering index heads (DESIGN.md §6)
    pub head_fall: Pubkey,
    pub head_rise: Pubkey,
    pub band_count_fall: u32,
    pub band_count_rise: u32,

    // Counters
    pub open_loans: u64,
    pub next_loan_nonce: u64,
    pub last_update_slot: u64,

    // Treasury
    pub protocol_fees_a: u64,
    pub protocol_fees_b: u64,

    pub _reserved: [u8; 64],
}

impl Pool {
    /// Size in bytes when serialized with borsh.
    pub const LEN: usize = 8                 // discriminator
        + 32 * 6                              // mint_a, mint_b, vault_a, vault_b, lp_mint, authority
        + 4                                   // 4× bump
        + 16 * 4                              // 4× u128 debt/collateral totals
        + 1 + 2 + 2 + 3                       // curve_kind, swap_fee_bps, protocol_fee_bps, _curve_pad
        + 2 * 4 + 8                           // 4× u16 lending bps + _lending_pad
        + 32 * 2 + 4 * 2                      // head_fall, head_rise, band_count_fall, band_count_rise
        + 8 * 3                               // open_loans, next_loan_nonce, last_update_slot
        + 8 * 2                               // protocol_fees_a, protocol_fees_b
        + 64;                                 // _reserved

    pub fn is_initialized(&self) -> bool {
        self.discriminator == POOL_DISCRIMINATOR
    }

    pub fn is_authority_renounced(&self) -> bool {
        self.authority == Pubkey::default()
    }

    /// Derive the pool PDA. Caller must pass mints already sorted (mint_a <
    /// mint_b lexicographically).
    pub fn derive_pda(
        mint_a: &Pubkey,
        mint_b: &Pubkey,
        program_id: &Pubkey,
    ) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[POOL_SEED, mint_a.as_ref(), mint_b.as_ref()],
            program_id,
        )
    }

    pub fn derive_vault_a_pda(pool: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[VAULT_A_SEED, pool.as_ref()], program_id)
    }

    pub fn derive_vault_b_pda(pool: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[VAULT_B_SEED, pool.as_ref()], program_id)
    }

    pub fn derive_lp_mint_pda(pool: &Pubkey, program_id: &Pubkey) -> (Pubkey, u8) {
        Pubkey::find_program_address(&[LP_MINT_SEED, pool.as_ref()], program_id)
    }
}

// ===== Loan =====

/// Per-position loan state. PDA: `["loan", pool, borrower, nonce_le_bytes]`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct Loan {
    pub discriminator: [u8; 8],

    pub pool: Pubkey,
    pub borrower: Pubkey,
    pub nonce: u64,
    pub bump: u8,

    /// `LoanSides` (CollateralA = 0, CollateralB = 1).
    pub sides: u8,

    pub collateral_amount: u128,
    pub debt_principal: u128,
    pub debt_accrued: u128,
    pub last_accrual_slot: u64,

    /// B-per-A trigger price, WAD-scaled.
    pub trigger_price_wad: u128,
    /// `TriggerDirection` (OnFall = 0, OnRise = 1).
    pub trigger_direction: u8,

    /// 0 = open, 1 = closed-by-repay, 2 = liquidated.
    pub status: u8,
    pub _status_pad: [u8; 6],

    pub opened_slot: u64,
    pub closed_slot: u64,

    pub _reserved: [u8; 32],
}

impl Loan {
    pub const LEN: usize = 8                 // discriminator
        + 32 * 2                              // pool, borrower
        + 8                                   // nonce
        + 1                                   // bump
        + 1                                   // sides
        + 16 * 3                              // collateral, principal, accrued
        + 8                                   // last_accrual_slot
        + 16                                  // trigger_price_wad
        + 1                                   // trigger_direction
        + 1 + 6                               // status + _status_pad
        + 8 * 2                               // opened_slot, closed_slot
        + 32;                                 // _reserved

    pub const STATUS_OPEN: u8 = 0;
    pub const STATUS_REPAID: u8 = 1;
    pub const STATUS_LIQUIDATED: u8 = 2;

    pub fn is_initialized(&self) -> bool {
        self.discriminator == LOAN_DISCRIMINATOR
    }

    pub fn is_open(&self) -> bool {
        self.status == Self::STATUS_OPEN
    }

    pub fn derive_pda(
        pool: &Pubkey,
        borrower: &Pubkey,
        nonce: u64,
        program_id: &Pubkey,
    ) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[LOAN_SEED, pool.as_ref(), borrower.as_ref(), &nonce.to_le_bytes()],
            program_id,
        )
    }
}

// ===== LoanLink =====

/// Doubly-linked list node for the loan-ordering index. PDA:
/// `["loan_link", pool, loan]`.
///
/// `prev`/`next` point to other `LoanLink` PDAs (default = chain end).
/// `trigger_price_wad` is denormalized from `Loan` so a swap doesn't have to
/// load the full `Loan` to decide whether the chain stops.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct LoanLink {
    pub discriminator: [u8; 8],

    pub pool: Pubkey,
    pub loan: Pubkey,

    pub band_id: u32,
    pub direction: u8,
    pub bump: u8,
    pub _pad: [u8; 2],

    pub prev: Pubkey,
    pub next: Pubkey,
    pub trigger_price_wad: u128,

    pub _reserved: [u8; 16],
}

impl LoanLink {
    pub const LEN: usize = 8                 // discriminator
        + 32 * 2                              // pool, loan
        + 4 + 1 + 1 + 2                       // band_id, direction, bump, _pad
        + 32 * 2                              // prev, next
        + 16                                  // trigger_price_wad
        + 16;                                 // _reserved

    pub fn is_initialized(&self) -> bool {
        self.discriminator == LOAN_LINK_DISCRIMINATOR
    }

    pub fn derive_pda(
        pool: &Pubkey,
        loan: &Pubkey,
        program_id: &Pubkey,
    ) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[LOAN_LINK_SEED, pool.as_ref(), loan.as_ref()],
            program_id,
        )
    }
}

// ===== LoanIndexBand =====

/// Bucket head for one (pool, direction, band_id). PDA:
/// `["band", pool, direction_byte, band_id_le_bytes]`.
#[derive(BorshSerialize, BorshDeserialize, Debug, Clone)]
pub struct LoanIndexBand {
    pub discriminator: [u8; 8],

    pub pool: Pubkey,
    pub band_id: u32,
    pub direction: u8,
    pub bump: u8,
    pub _pad: [u8; 2],

    pub head_link: Pubkey,
    pub tail_link: Pubkey,
    pub count: u32,
    pub _pad2: [u8; 4],

    pub min_trigger_wad: u128,
    pub max_trigger_wad: u128,

    pub _reserved: [u8; 32],
}

impl LoanIndexBand {
    pub const LEN: usize = 8                 // discriminator
        + 32                                  // pool
        + 4 + 1 + 1 + 2                       // band_id, direction, bump, _pad
        + 32 * 2                              // head_link, tail_link
        + 4 + 4                               // count, _pad2
        + 16 * 2                              // min/max trigger
        + 32;                                 // _reserved

    /// Hard cap on intra-band link count. When exceeded, callers must use
    /// `RebalanceBands` to subdivide before opening more loans in this band.
    pub const MAX_LINKS: u32 = 64;

    pub fn is_initialized(&self) -> bool {
        self.discriminator == LOAN_INDEX_BAND_DISCRIMINATOR
    }

    pub fn derive_pda(
        pool: &Pubkey,
        direction: u8,
        band_id: u32,
        program_id: &Pubkey,
    ) -> (Pubkey, u8) {
        Pubkey::find_program_address(
            &[
                BAND_SEED,
                pool.as_ref(),
                &[direction],
                &band_id.to_le_bytes(),
            ],
            program_id,
        )
    }
}

// ===== Tests =====

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_pool() -> Pool {
        Pool {
            discriminator: POOL_DISCRIMINATOR,
            mint_a: Pubkey::new_unique(),
            mint_b: Pubkey::new_unique(),
            vault_a: Pubkey::new_unique(),
            vault_b: Pubkey::new_unique(),
            lp_mint: Pubkey::new_unique(),
            authority: Pubkey::new_unique(),
            pool_bump: 255,
            vault_a_bump: 254,
            vault_b_bump: 253,
            lp_mint_bump: 252,
            total_debt_a: 0,
            total_debt_b: 0,
            total_collateral_a: 0,
            total_collateral_b: 0,
            curve_kind: CurveKind::Cpmm as u8,
            swap_fee_bps: 30,
            protocol_fee_bps: 5,
            _curve_pad: [0; 3],
            liq_ratio_bps: 11000,
            liq_penalty_bps: 500,
            max_ltv_bps: 8000,
            interest_rate_bps_per_year: 500,
            _lending_pad: [0; 8],
            head_fall: Pubkey::default(),
            head_rise: Pubkey::default(),
            band_count_fall: 0,
            band_count_rise: 0,
            open_loans: 0,
            next_loan_nonce: 0,
            last_update_slot: 0,
            protocol_fees_a: 0,
            protocol_fees_b: 0,
            _reserved: [0; 64],
        }
    }

    fn fake_loan() -> Loan {
        Loan {
            discriminator: LOAN_DISCRIMINATOR,
            pool: Pubkey::new_unique(),
            borrower: Pubkey::new_unique(),
            nonce: 1,
            bump: 255,
            sides: 0,
            collateral_amount: 50,
            debt_principal: 100,
            debt_accrued: 0,
            last_accrual_slot: 0,
            trigger_price_wad: 2_200_000_000_000_000_000,
            trigger_direction: 0,
            status: Loan::STATUS_OPEN,
            _status_pad: [0; 6],
            opened_slot: 0,
            closed_slot: 0,
            _reserved: [0; 32],
        }
    }

    fn fake_loan_link() -> LoanLink {
        LoanLink {
            discriminator: LOAN_LINK_DISCRIMINATOR,
            pool: Pubkey::new_unique(),
            loan: Pubkey::new_unique(),
            band_id: 7,
            direction: 0,
            bump: 255,
            _pad: [0; 2],
            prev: Pubkey::default(),
            next: Pubkey::default(),
            trigger_price_wad: 2_200_000_000_000_000_000,
            _reserved: [0; 16],
        }
    }

    fn fake_band() -> LoanIndexBand {
        LoanIndexBand {
            discriminator: LOAN_INDEX_BAND_DISCRIMINATOR,
            pool: Pubkey::new_unique(),
            band_id: 7,
            direction: 0,
            bump: 255,
            _pad: [0; 2],
            head_link: Pubkey::default(),
            tail_link: Pubkey::default(),
            count: 0,
            _pad2: [0; 4],
            min_trigger_wad: 0,
            max_trigger_wad: 0,
            _reserved: [0; 32],
        }
    }

    #[test]
    fn pool_size() {
        let p = fake_pool();
        let v = borsh::to_vec(&p).unwrap();
        assert_eq!(v.len(), Pool::LEN);
    }

    #[test]
    fn loan_size() {
        let l = fake_loan();
        let v = borsh::to_vec(&l).unwrap();
        assert_eq!(v.len(), Loan::LEN);
    }

    #[test]
    fn loan_link_size() {
        let l = fake_loan_link();
        let v = borsh::to_vec(&l).unwrap();
        assert_eq!(v.len(), LoanLink::LEN);
    }

    #[test]
    fn band_size() {
        let b = fake_band();
        let v = borsh::to_vec(&b).unwrap();
        assert_eq!(v.len(), LoanIndexBand::LEN);
    }

    #[test]
    fn pool_roundtrip() {
        let p = fake_pool();
        let v = borsh::to_vec(&p).unwrap();
        let p2 = Pool::try_from_slice(&v).unwrap();
        assert_eq!(p2.swap_fee_bps, 30);
        assert_eq!(p2.liq_ratio_bps, 11000);
        assert!(p2.is_initialized());
        assert!(!p2.is_authority_renounced());
    }

    #[test]
    fn loan_roundtrip() {
        let l = fake_loan();
        let v = borsh::to_vec(&l).unwrap();
        let l2 = Loan::try_from_slice(&v).unwrap();
        assert_eq!(l2.collateral_amount, 50);
        assert_eq!(l2.debt_principal, 100);
        assert!(l2.is_open());
    }

    #[test]
    fn loan_link_roundtrip() {
        let l = fake_loan_link();
        let v = borsh::to_vec(&l).unwrap();
        let l2 = LoanLink::try_from_slice(&v).unwrap();
        assert_eq!(l2.band_id, 7);
        assert_eq!(l2.direction, 0);
        assert_eq!(l2.prev, Pubkey::default());
        assert_eq!(l2.next, Pubkey::default());
    }

    #[test]
    fn band_roundtrip() {
        let b = fake_band();
        let v = borsh::to_vec(&b).unwrap();
        let b2 = LoanIndexBand::try_from_slice(&v).unwrap();
        assert_eq!(b2.band_id, 7);
        assert_eq!(b2.count, 0);
    }

    #[test]
    fn spl_token_program_id_constant() {
        let expected: Pubkey = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
            .parse()
            .unwrap();
        assert_eq!(SPL_TOKEN_PROGRAM_ID, expected);
    }

    #[test]
    fn is_valid_token_program_accepts_both() {
        assert!(is_valid_token_program(&SPL_TOKEN_PROGRAM_ID));
        assert!(is_valid_token_program(&spl_token_2022::id()));
        assert!(!is_valid_token_program(&Pubkey::default()));
    }

    #[test]
    fn pool_pda_is_canonical() {
        let mint_a = Pubkey::new_unique();
        let mint_b = Pubkey::new_unique();
        let prog = Pubkey::new_unique();
        let (a, _) = Pool::derive_pda(&mint_a, &mint_b, &prog);
        let (b, _) = Pool::derive_pda(&mint_a, &mint_b, &prog);
        assert_eq!(a, b);
        // Different ordering produces different PDA — caller must sort.
        let (c, _) = Pool::derive_pda(&mint_b, &mint_a, &prog);
        if mint_a != mint_b {
            assert_ne!(a, c);
        }
    }

    #[test]
    fn band_pda_distinct_per_direction() {
        let pool = Pubkey::new_unique();
        let prog = Pubkey::new_unique();
        let (a, _) = LoanIndexBand::derive_pda(&pool, 0, 5, &prog);
        let (b, _) = LoanIndexBand::derive_pda(&pool, 1, 5, &prog);
        let (c, _) = LoanIndexBand::derive_pda(&pool, 0, 6, &prog);
        assert_ne!(a, b);
        assert_ne!(a, c);
    }

    /// Confirm the corrected LEN constants, since DESIGN.md had arithmetic errors.
    /// (DESIGN.md had Pool=442 and Loan=224; correct values verified by the
    /// `*_size` borsh roundtrip tests above are 468 and 210.)
    #[test]
    fn known_sizes() {
        assert_eq!(Pool::LEN, 468);
        assert_eq!(Loan::LEN, 210);
        assert_eq!(LoanLink::LEN, 176);
        assert_eq!(LoanIndexBand::LEN, 184);
    }
}
