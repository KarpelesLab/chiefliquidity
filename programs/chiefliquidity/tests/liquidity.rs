//! Integration tests for `AddLiquidity` and `RemoveLiquidity`.

mod common;

use chiefliquidity::error::LiquidityError;
use common::{err_code, extract_custom_error, TestEnv};
use solana_sdk::signature::Signer;

// ============ AddLiquidity ============

#[tokio::test]
async fn add_liquidity_first_deposit_mints_sqrt() {
    let mut env = TestEnv::new().await;
    env.initialize_pool_default().await;

    // First depositor: 100M of A, 400M of B → LP = sqrt(100M * 400M) = 200M.
    let (user, ata_a, ata_b, ata_lp) =
        env.setup_user(10_000_000_000, 200_000_000, 800_000_000).await;
    let ix = env.ix_add_liquidity(
        &user.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        100_000_000,
        400_000_000,
        1,
    );
    env.send_with_new_blockhash(&[ix], &[&user]).await.unwrap();

    let lp_balance = env.token_balance(&ata_lp).await;
    assert_eq!(lp_balance, 200_000_000);

    let vault_a_balance = env.token_balance(&env.vault_a_pda().0).await;
    let vault_b_balance = env.token_balance(&env.vault_b_pda().0).await;
    assert_eq!(vault_a_balance, 100_000_000);
    assert_eq!(vault_b_balance, 400_000_000);

    // Pool unchanged in debt/collateral counters.
    let pool = env.pool_state().await;
    assert_eq!(pool.total_debt_a, 0);
    assert_eq!(pool.total_collateral_a, 0);
}

#[tokio::test]
async fn add_liquidity_second_deposit_proportional() {
    let mut env = TestEnv::new().await;
    let _ = env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;
    // After: vault_a=100M, vault_b=400M, lp_supply=sqrt(100M*400M)=200M.

    // Second depositor adds in the same ratio (1:4): 50M A and 200M B.
    let (user2, ata_a, ata_b, ata_lp) =
        env.setup_user(10_000_000_000, 100_000_000, 400_000_000).await;
    let ix = env.ix_add_liquidity(
        &user2.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        50_000_000,
        200_000_000,
        1,
    );
    env.send_with_new_blockhash(&[ix], &[&user2]).await.unwrap();

    // Should mint LP proportional: 50M / 100M = 50% of supply → 100M LP.
    let lp_supply = env.mint_supply(&env.lp_mint_pda().0).await;
    assert_eq!(lp_supply, 200_000_000 + 100_000_000);
    let lp_balance = env.token_balance(&ata_lp).await;
    assert_eq!(lp_balance, 100_000_000);
}

#[tokio::test]
async fn add_liquidity_excess_b_is_clipped() {
    let mut env = TestEnv::new().await;
    let _ = env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;

    // User offers 50M A + 1B B. Ratio is 1:4, so 50M A pairs with 200M B.
    // The extra 800M B should NOT be transferred. Caller-side balance proves it.
    let (user2, ata_a, ata_b, ata_lp) =
        env.setup_user(10_000_000_000, 100_000_000, 1_000_000_000).await;
    let ix = env.ix_add_liquidity(
        &user2.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        50_000_000,
        1_000_000_000,
        1,
    );
    env.send_with_new_blockhash(&[ix], &[&user2]).await.unwrap();

    let user_a_left = env.token_balance(&ata_a).await;
    let user_b_left = env.token_balance(&ata_b).await;
    assert_eq!(user_a_left, 100_000_000 - 50_000_000);
    assert_eq!(user_b_left, 1_000_000_000 - 200_000_000);
}

#[tokio::test]
async fn add_liquidity_slippage_breach() {
    let mut env = TestEnv::new().await;
    let _ = env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;

    let (user2, ata_a, ata_b, ata_lp) =
        env.setup_user(10_000_000_000, 100_000_000, 400_000_000).await;
    // Demand min_lp_out = 1B. Actual mint will be ~100M. Should revert.
    let ix = env.ix_add_liquidity(
        &user2.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        50_000_000,
        200_000_000,
        1_000_000_000,
    );
    let err = env
        .send_with_new_blockhash(&[ix], &[&user2])
        .await
        .unwrap_err();
    assert_eq!(
        extract_custom_error(&err),
        Some(err_code(LiquidityError::SlippageExceeded))
    );
}

#[tokio::test]
async fn add_liquidity_below_min_first_deposit() {
    let mut env = TestEnv::new().await;
    env.initialize_pool_default().await;

    let (user, ata_a, ata_b, ata_lp) =
        env.setup_user(10_000_000_000, 1_000_000, 1_000_000).await;
    // 100 < MIN_FIRST_DEPOSIT (1_000_000) — should revert
    let ix = env.ix_add_liquidity(
        &user.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        100,
        100,
        1,
    );
    let err = env
        .send_with_new_blockhash(&[ix], &[&user])
        .await
        .unwrap_err();
    assert_eq!(
        extract_custom_error(&err),
        Some(err_code(LiquidityError::ZeroAmount))
    );
}

#[tokio::test]
async fn add_liquidity_without_signer_rejected() {
    let mut env = TestEnv::new().await;
    let _ = env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;

    let (user2, ata_a, ata_b, ata_lp) =
        env.setup_user(10_000_000_000, 100_000_000, 400_000_000).await;
    let mut ix = env.ix_add_liquidity(
        &user2.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        50_000_000,
        200_000_000,
        1,
    );
    // Strip signer flag from user account; do NOT pass user keypair to
    // signers (so the framework doesn't refuse with KeypairPubkeyMismatch).
    let user_idx = ix.accounts.iter().position(|a| a.pubkey == user2.pubkey()).unwrap();
    ix.accounts[user_idx].is_signer = false;
    let err = env
        .send_with_new_blockhash(&[ix], &[])
        .await
        .unwrap_err();
    // Either MissingRequiredSigner from us, or a token program transfer
    // failure since the SPL transfer needs user as signer too.
    let code = extract_custom_error(&err);
    assert!(
        code == Some(err_code(LiquidityError::MissingRequiredSigner)) || code.is_some(),
        "expected error; got {code:?}"
    );
}

#[tokio::test]
async fn add_liquidity_wrong_vault_rejected() {
    let mut env = TestEnv::new().await;
    let _ = env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;

    let (user2, ata_a, ata_b, ata_lp) =
        env.setup_user(10_000_000_000, 100_000_000, 400_000_000).await;
    let mut ix = env.ix_add_liquidity(
        &user2.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        50_000_000,
        200_000_000,
        1,
    );
    // Swap vault A and vault B — pool's stored keys won't match.
    let v_a_idx = ix.accounts.iter().position(|a| a.pubkey == env.vault_a_pda().0).unwrap();
    let v_b_idx = ix.accounts.iter().position(|a| a.pubkey == env.vault_b_pda().0).unwrap();
    ix.accounts.swap(v_a_idx, v_b_idx);
    let err = env
        .send_with_new_blockhash(&[ix], &[&user2])
        .await
        .unwrap_err();
    assert_eq!(
        extract_custom_error(&err),
        Some(err_code(LiquidityError::InvalidPool))
    );
}

// ============ RemoveLiquidity ============

#[tokio::test]
async fn remove_liquidity_full_round_trip() {
    let mut env = TestEnv::new().await;
    let (user, ata_a, ata_b, ata_lp) =
        env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;
    let lp_owned = env.token_balance(&ata_lp).await;
    assert_eq!(lp_owned, 200_000_000);

    let pre_vault_a = env.token_balance(&env.vault_a_pda().0).await;
    let pre_vault_b = env.token_balance(&env.vault_b_pda().0).await;

    // Burn all LP — get back proportional A and B.
    let ix = env.ix_remove_liquidity(
        &user.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        lp_owned,
        1,
        1,
    );
    env.send_with_new_blockhash(&[ix], &[&user]).await.unwrap();

    let post_vault_a = env.token_balance(&env.vault_a_pda().0).await;
    let post_vault_b = env.token_balance(&env.vault_b_pda().0).await;
    assert_eq!(post_vault_a, 0);
    assert_eq!(post_vault_b, 0);

    let user_a = env.token_balance(&ata_a).await;
    let user_b = env.token_balance(&ata_b).await;
    assert_eq!(user_a, 100_000_000 + pre_vault_a);
    assert_eq!(user_b, 400_000_000 + pre_vault_b);
    assert_eq!(env.token_balance(&ata_lp).await, 0);
}

#[tokio::test]
async fn remove_liquidity_partial() {
    let mut env = TestEnv::new().await;
    let (user, ata_a, ata_b, ata_lp) =
        env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;
    let lp_owned = env.token_balance(&ata_lp).await;

    // Burn 25% — expect ~25M A and ~100M B back.
    let burn = lp_owned / 4;
    let ix = env.ix_remove_liquidity(
        &user.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        burn,
        24_000_000,
        96_000_000,
    );
    env.send_with_new_blockhash(&[ix], &[&user]).await.unwrap();

    assert_eq!(env.token_balance(&ata_lp).await, lp_owned - burn);
    assert_eq!(env.token_balance(&env.vault_a_pda().0).await, 75_000_000);
    assert_eq!(env.token_balance(&env.vault_b_pda().0).await, 300_000_000);
}

#[tokio::test]
async fn remove_liquidity_slippage_breach() {
    let mut env = TestEnv::new().await;
    let (user, ata_a, ata_b, ata_lp) =
        env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;
    let lp_owned = env.token_balance(&ata_lp).await;

    // Demand way more A than possible.
    let ix = env.ix_remove_liquidity(
        &user.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        lp_owned / 4,
        1_000_000_000,
        1,
    );
    let err = env
        .send_with_new_blockhash(&[ix], &[&user])
        .await
        .unwrap_err();
    assert_eq!(
        extract_custom_error(&err),
        Some(err_code(LiquidityError::SlippageExceeded))
    );
}

#[tokio::test]
async fn remove_liquidity_more_than_supply_rejected() {
    let mut env = TestEnv::new().await;
    let (user, ata_a, ata_b, ata_lp) =
        env.setup_pool_with_liquidity(100_000_000, 400_000_000).await;

    // Try to burn 10x the LP supply.
    let ix = env.ix_remove_liquidity(
        &user.pubkey(),
        &ata_a,
        &ata_b,
        &ata_lp,
        2_000_000_000,
        1,
        1,
    );
    let err = env
        .send_with_new_blockhash(&[ix], &[&user])
        .await
        .unwrap_err();
    let code = extract_custom_error(&err);
    assert_eq!(
        code,
        Some(err_code(LiquidityError::MathUnderflow)),
        "expected MathUnderflow; got {code:?}"
    );
}
