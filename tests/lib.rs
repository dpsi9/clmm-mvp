use bytemuck;
use litesvm::LiteSVM;

use mini_clmm::state::Pool;

use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_pubkey::{pubkey, Pubkey};
use solana_signer::Signer;
use solana_transaction::Transaction;

mod helpers;
use helpers::*;

const PROGRAM_ID: Pubkey = pubkey!("FmBWcVKgRj8RqdQx1MZ3g6arqJtx8q1UDqSGSiKPy9oV");

use mini_clmm::processor::ClmmInstruction;

fn setup_clmm() -> (
    LiteSVM,
    Keypair,
    Keypair,
    Pubkey,
    Pubkey,
    Pubkey,
    Pubkey,
    Pubkey,
    Pubkey,
    Pubkey,
) {
    let mut svm = LiteSVM::new();

    // Add the compiled program
    svm.add_program_from_file(PROGRAM_ID, "target/deploy/mini_clmm.so")
        .unwrap();

    assert!(svm.get_account(&PROGRAM_ID).unwrap().executable);

    let owner = Keypair::new();
    let trader = Keypair::new();

    svm.airdrop(&owner.pubkey(), 100_000_000_000).unwrap();
    svm.airdrop(&trader.pubkey(), 100_000_000_000).unwrap();

    // Create mints for USDC and SOL (or token0/token1)
    let usdc_mint = create_mint(&mut svm, 6, &owner);
    let sol_mint = create_mint(&mut svm, 9, &owner);

    // Create token accounts for owner
    let owner_usdc = create_token_account(&mut svm, &owner, &owner.pubkey(), &usdc_mint);
    let owner_sol = create_token_account(&mut svm, &owner, &owner.pubkey(), &sol_mint);

    // Create token accounts for trader
    let trader_usdc = create_token_account(&mut svm, &trader, &trader.pubkey(), &usdc_mint);
    let trader_sol = create_token_account(&mut svm, &trader, &trader.pubkey(), &sol_mint);

    // Mint tokens
    mint_tokens(&mut svm, &owner, &usdc_mint, &owner, &owner_usdc, 1_000_000);
    mint_tokens(&mut svm, &owner, &sol_mint, &owner, &owner_sol, 1_000_000);
    mint_tokens(
        &mut svm,
        &trader,
        &usdc_mint,
        &owner,
        &trader_usdc,
        1_000_000,
    );
    mint_tokens(&mut svm, &trader, &sol_mint, &owner, &trader_sol, 1_000_000);

    // Create pool account
    let pool_account = Keypair::new();
    let space = Pool::LEN;
    let lamports = svm.minimum_balance_for_rent_exemption(space);

    let create_ix = solana_system_interface::instruction::create_account(
        &owner.pubkey(),
        &pool_account.pubkey(),
        lamports,
        space as u64,
        &PROGRAM_ID,
    );

    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[create_ix],
        Some(&owner.pubkey()),
        &[&owner, &pool_account],
        blockhash,
    );
    svm.send_transaction(tx).unwrap();

    (
        svm,
        owner,
        trader,
        usdc_mint,
        sol_mint,
        owner_usdc,
        owner_sol,
        trader_usdc,
        trader_sol,
        pool_account.pubkey(),
    )
}

#[test]
fn initialize_pool() {
    let (
        mut svm,
        owner,
        _trader,
        usdc_mint,
        sol_mint,
        _owner_usdc,
        _owner_sol,
        _trader_usdc,
        _trader_sol,
        pool_account,
    ) = setup_clmm();

    // Create vaults for the pool
    let usdc_vault = create_token_account(&mut svm, &owner, &pool_account, &usdc_mint);
    let sol_vault = create_token_account(&mut svm, &owner, &pool_account, &sol_mint);

    let instruction_data = borsh::to_vec(&ClmmInstruction::InitializePool {
        token_0_mint: usdc_mint,
        token_1_mint: sol_mint,
        token_0_vault: usdc_vault,
        token_1_vault: sol_vault,
    })
    .unwrap();

    let ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: instruction_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(&[ix], Some(&owner.pubkey()), &[&owner], blockhash);

    let result = svm.send_transaction(tx);

    match result {
        Ok(_meta) => {
            println!("Initialize pool ix succeeded!");
        }
        Err(e) => {
            panic!("Initialize pool ix failed: {:#?}", e);
        }
    }

    let pool_account_data = svm.get_account(&pool_account).unwrap();
    let pool: &Pool = bytemuck::from_bytes(&pool_account_data.data[..Pool::LEN]);

    assert_eq!(pool.token_0_mint, usdc_mint);
    assert_eq!(pool.token_1_mint, sol_mint);
    assert_eq!(pool.token_0_vault, usdc_vault);
    assert_eq!(pool.token_1_vault, sol_vault);
    assert_eq!(pool.sqrt_price, 1 << 64); // Q64
    assert_eq!(pool.liquidity, 0);
    assert_eq!(pool.positions_count, 0);
}

#[test]
fn add_liquidity() {
    let (
        mut svm,
        owner,
        _trader,
        usdc_mint,
        sol_mint,
        _owner_usdc,
        _owner_sol,
        _trader_usdc,
        _trader_sol,
        pool_account,
    ) = setup_clmm();

    // Create vaults
    let usdc_vault = create_token_account(&mut svm, &owner, &pool_account, &usdc_mint);
    let sol_vault = create_token_account(&mut svm, &owner, &pool_account, &sol_mint);

    // Initialize pool first
    let init_data = borsh::to_vec(&ClmmInstruction::InitializePool {
        token_0_mint: usdc_mint,
        token_1_mint: sol_mint,
        token_0_vault: usdc_vault,
        token_1_vault: sol_vault,
    })
    .unwrap();

    let init_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: init_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&owner.pubkey()), &[&owner], blockhash);
    svm.send_transaction(tx).expect("Initialize pool failed");

    // Add liquidity
    let add_liquidity_data = borsh::to_vec(&ClmmInstruction::AddLiquidity {
        tick_lower: -10,
        tick_upper: 10,
        usdc_amount: 1000,
    })
    .unwrap();

    let add_liquidity_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: add_liquidity_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[add_liquidity_ix],
        Some(&owner.pubkey()),
        &[&owner],
        blockhash,
    );

    let result = svm.send_transaction(tx);

    match result {
        Ok(_meta) => {
            println!("Add liquidity ix succeeded!");
        }
        Err(e) => {
            panic!("Add liquidity ix failed: {:#?}", e);
        }
    }

    let pool_account_data = svm.get_account(&pool_account).unwrap();

    println!("Pool::LEN = {}", Pool::LEN);
    println!("Account data len = {}", pool_account_data.data.len());
    println!(
        "Position size = {}",
        std::mem::size_of::<mini_clmm::state::Position>()
    );

    let pool: &Pool = bytemuck::from_bytes(&pool_account_data.data[..Pool::LEN]);

    println!("Pool liquidity: {}", pool.liquidity);
    println!("Pool positions_count: {}", pool.positions_count);
    println!("Owner pubkey: {}", owner.pubkey());

    // Debug: print first position
    let first_pos = &pool.positions[0];
    println!("First position is_active: {}", first_pos.is_active);
    println!("First position owner: {}", first_pos.owner);
    println!("First position liquidity: {}", first_pos.liquidity);

    assert!(pool.liquidity > 0);
    assert_eq!(pool.positions_count, 1);

    let position = pool.get_position(&owner.pubkey()).unwrap();
    assert_eq!(position.tick_lower, -10);
    assert_eq!(position.tick_upper, 10);
    assert!(position.liquidity > 0);
}

#[test]
fn buy_sol() {
    let (
        mut svm,
        owner,
        trader,
        usdc_mint,
        sol_mint,
        _owner_usdc,
        _owner_sol,
        _trader_usdc,
        _trader_sol,
        pool_account,
    ) = setup_clmm();

    // Create vaults
    let usdc_vault = create_token_account(&mut svm, &owner, &pool_account, &usdc_mint);
    let sol_vault = create_token_account(&mut svm, &owner, &pool_account, &sol_mint);

    // Initialize pool
    let init_data = borsh::to_vec(&ClmmInstruction::InitializePool {
        token_0_mint: usdc_mint,
        token_1_mint: sol_mint,
        token_0_vault: usdc_vault,
        token_1_vault: sol_vault,
    })
    .unwrap();

    let init_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: init_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&owner.pubkey()), &[&owner], blockhash);
    svm.send_transaction(tx).expect("Initialize pool failed");

    // Add liquidity first
    let add_liquidity_data = borsh::to_vec(&ClmmInstruction::AddLiquidity {
        tick_lower: -50,
        tick_upper: 50,
        usdc_amount: 10_000,
    })
    .unwrap();

    let add_liquidity_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: add_liquidity_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[add_liquidity_ix],
        Some(&owner.pubkey()),
        &[&owner],
        blockhash,
    );
    svm.send_transaction(tx).expect("Add liquidity failed");

    // Get price before swap
    let pool_before = svm.get_account(&pool_account).unwrap();
    let pool_data_before: &Pool = bytemuck::from_bytes(&pool_before.data[..Pool::LEN]);
    let sqrt_price_before = pool_data_before.sqrt_price;

    // Buy SOL (swap USDC for SOL)
    let buy_sol_data = borsh::to_vec(&ClmmInstruction::BuySol { usdc_amount: 100 }).unwrap();

    let buy_sol_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(trader.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: buy_sol_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[buy_sol_ix],
        Some(&trader.pubkey()),
        &[&trader],
        blockhash,
    );

    let result = svm.send_transaction(tx);

    match result {
        Ok(_meta) => {
            println!("Buy SOL ix succeeded!");
        }
        Err(e) => {
            panic!("Buy SOL ix failed: {:#?}", e);
        }
    }

    // Get price after swap
    let pool_after = svm.get_account(&pool_account).unwrap();
    let pool_data_after: &Pool = bytemuck::from_bytes(&pool_after.data[..Pool::LEN]);

    assert_ne!(pool_data_after.sqrt_price, sqrt_price_before);
    assert!(pool_data_after.sqrt_price > sqrt_price_before); // Price increases when buying SOL
}

#[test]
fn remove_liquidity() {
    let (
        mut svm,
        owner,
        _trader,
        usdc_mint,
        sol_mint,
        _owner_usdc,
        _owner_sol,
        _trader_usdc,
        _trader_sol,
        pool_account,
    ) = setup_clmm();

    // Create vaults
    let usdc_vault = create_token_account(&mut svm, &owner, &pool_account, &usdc_mint);
    let sol_vault = create_token_account(&mut svm, &owner, &pool_account, &sol_mint);

    // Initialize pool
    let init_data = borsh::to_vec(&ClmmInstruction::InitializePool {
        token_0_mint: usdc_mint,
        token_1_mint: sol_mint,
        token_0_vault: usdc_vault,
        token_1_vault: sol_vault,
    })
    .unwrap();

    let init_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: init_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&owner.pubkey()), &[&owner], blockhash);
    svm.send_transaction(tx).expect("Initialize pool failed");

    // Add liquidity
    let add_liquidity_data = borsh::to_vec(&ClmmInstruction::AddLiquidity {
        tick_lower: -20,
        tick_upper: 20,
        usdc_amount: 5000,
    })
    .unwrap();

    let add_liquidity_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: add_liquidity_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[add_liquidity_ix],
        Some(&owner.pubkey()),
        &[&owner],
        blockhash,
    );
    svm.send_transaction(tx).expect("Add liquidity failed");

    // Get position liquidity before removal
    let pool_before = svm.get_account(&pool_account).unwrap();
    let pool_data_before: &Pool = bytemuck::from_bytes(&pool_before.data[..Pool::LEN]);
    let position_before = pool_data_before.get_position(&owner.pubkey()).unwrap();
    let liquidity_to_remove = position_before.liquidity / 2;

    // Remove liquidity
    let remove_liquidity_data = borsh::to_vec(&ClmmInstruction::RemoveLiquidity {
        liquidity: liquidity_to_remove,
    })
    .unwrap();

    let remove_liquidity_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: remove_liquidity_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx = Transaction::new_signed_with_payer(
        &[remove_liquidity_ix],
        Some(&owner.pubkey()),
        &[&owner],
        blockhash,
    );

    let result = svm.send_transaction(tx);

    match result {
        Ok(_meta) => {
            println!("Remove liquidity ix succeeded!");
        }
        Err(e) => {
            panic!("Remove liquidity ix failed: {:#?}", e);
        }
    }

    // Get position after removal
    let pool_after = svm.get_account(&pool_account).unwrap();
    let pool_data_after: &Pool = bytemuck::from_bytes(&pool_after.data[..Pool::LEN]);
    let position_after = pool_data_after.get_position(&owner.pubkey()).unwrap();

    assert_eq!(
        position_after.liquidity,
        position_before.liquidity - liquidity_to_remove
    );
}

#[test]
fn test_max_positions() {
    let (
        mut svm,
        owner,
        _trader,
        usdc_mint,
        sol_mint,
        _owner_usdc,
        _owner_sol,
        _trader_usdc,
        _trader_sol,
        pool_account,
    ) = setup_clmm();

    // Create vaults
    let usdc_vault = create_token_account(&mut svm, &owner, &pool_account, &usdc_mint);
    let sol_vault = create_token_account(&mut svm, &owner, &pool_account, &sol_mint);

    // Initialize pool
    let init_data = borsh::to_vec(&ClmmInstruction::InitializePool {
        token_0_mint: usdc_mint,
        token_1_mint: sol_mint,
        token_0_vault: usdc_vault,
        token_1_vault: sol_vault,
    })
    .unwrap();

    let init_ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(owner.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: init_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[init_ix], Some(&owner.pubkey()), &[&owner], blockhash);
    svm.send_transaction(tx).expect("Initialize pool failed");

    // Create 10 different users to add positions (max positions)
    for i in 0..10 {
        let user = Keypair::new();
        svm.airdrop(&user.pubkey(), 100_000_000_000).unwrap();

        let user_usdc = create_token_account(&mut svm, &user, &user.pubkey(), &usdc_mint);
        mint_tokens(&mut svm, &user, &usdc_mint, &owner, &user_usdc, 1_000);

        let add_data = borsh::to_vec(&ClmmInstruction::AddLiquidity {
            tick_lower: -10 + i,
            tick_upper: 10 + i,
            usdc_amount: 100,
        })
        .unwrap();

        let ix = Instruction {
            program_id: PROGRAM_ID,
            accounts: vec![
                AccountMeta::new(user.pubkey(), true),
                AccountMeta::new(pool_account, false),
            ],
            data: add_data,
        };

        let blockhash = svm.latest_blockhash();
        let tx =
            Transaction::new_signed_with_payer(&[ix], Some(&user.pubkey()), &[&user], blockhash);

        svm.send_transaction(tx).unwrap_or_else(|e| {
            panic!("Failed to add position {}: {:#?}", i, e);
        });
    }

    // Verify we have 10 positions
    let pool = svm.get_account(&pool_account).unwrap();
    let pool_data: &Pool = bytemuck::from_bytes(&pool.data[..Pool::LEN]);
    assert_eq!(pool_data.positions_count, 10);

    // Try to add 11th position - should fail
    let user11 = Keypair::new();
    svm.airdrop(&user11.pubkey(), 100_000_000_000).unwrap();

    let add_data = borsh::to_vec(&ClmmInstruction::AddLiquidity {
        tick_lower: -5,
        tick_upper: 5,
        usdc_amount: 100,
    })
    .unwrap();

    let ix = Instruction {
        program_id: PROGRAM_ID,
        accounts: vec![
            AccountMeta::new(user11.pubkey(), true),
            AccountMeta::new(pool_account, false),
        ],
        data: add_data,
    };

    let blockhash = svm.latest_blockhash();
    let tx =
        Transaction::new_signed_with_payer(&[ix], Some(&user11.pubkey()), &[&user11], blockhash);

    let result = svm.send_transaction(tx);
    assert!(result.is_err(), "Should fail when adding 11th position");
}
// use borsh::{BorshDeserialize, BorshSerialize};
// use litesvm::LiteSVM;

// use mini_clmm::{
//     processor::{self, ClmmInstruction},
//     state::Pool,
// };

// use solana_instruction::{AccountMeta, Instruction};
// use solana_keypair::Keypair;
// use solana_pubkey::{pubkey, Pubkey};
// use solana_signer::Signer;
// // use solana_program_pa

// mod helpers;
// use helpers::*;

// const PROGRAM_ID: Pubkey = pubkey!("FmBWcVKgRj8RqdQx1MZ3g6arqJtx8q1UDqSGSiKPy9oV");

// fn setup_clmm() -> (
//     LiteSVM,
//     Keypair,
//     Keypair,
//     Pubkey,
//     Pubkey,
//     Pubkey,
//     Pubkey,
//     Pubkey,
//     Pubkey,
//     Pubkey,
// ) {
//     let mut svm = LiteSVM::new();

//     svm.add_program_from_file(PROGRAM_ID, "../target/deploy/mini_clmm.so")
//         .unwrap();

//     assert!(svm.get_account(&PROGRAM_ID).unwrap().executable);

//     let owner = Keypair::new();
//     let trader = Keypair::new();

//     svm.airdrop(&owner.pubkey(), 100_000_000_000).unwrap();
//     svm.airdrop(&trader.pubkey(), 100_000_000_000).unwrap();

//     // Create mints
//     let usdc_mint = create_mint(&mut svm, 6, &owner);
//     let sol_mint = create_mint(&mut svm, 6, &owner);

//     // Create token accounts for owner
//     let owner_usdc = create_token_account(&mut svm, &owner, &owner.pubkey(), &usdc_mint);
//     let owner_sol = create_token_account(&mut svm, &owner, &owner.pubkey(), &sol_mint);

//     // Create token accounts for trader
//     let trader_usdc = create_token_account(&mut svm, &trader, &trader.pubkey(), &usdc_mint);
//     let trader_sol = create_token_account(&mut svm, &trader, &trader.pubkey(), &sol_mint);

//     mint_tokens(
//         &mut svm,
//         &owner,
//         &usdc_mint,
//         &owner,
//         &owner_usdc,
//         1_000_000_000,
//     );
//     mint_tokens(
//         &mut svm,
//         &owner,
//         &sol_mint,
//         &owner,
//         &owner_sol,
//         1_000_000_000,
//     );
//     mint_tokens(
//         &mut svm,
//         &trader,
//         &usdc_mint,
//         &trader,
//         &trader_usdc,
//         1_000_000_000,
//     );
//     mint_tokens(
//         &mut svm,
//         &trader,
//         &sol_mint,
//         &trader,
//         &trader_sol,
//         1_000_000_000,
//     );

//     // Create pool account
//     let pool_account = Keypair::new();
//     let space = Pool::LEN;
//     let lamports = svm.minimum_balance_for_rent_exemption(space);

//     let ix_data = CreateAcco
// }

#[test]
fn debug_layout() {
    use mini_clmm::state::{Pool, Position, MAX_POSITIONS, MAX_TICKS};

    println!("Position size: {}", std::mem::size_of::<Position>());
    println!("Position align: {}", std::mem::align_of::<Position>());
    println!("Pool size: {}", std::mem::size_of::<Pool>());
    println!("Pool align: {}", std::mem::align_of::<Pool>());

    // Expected:
    // 5 Pubkeys = 160
    // 2 u128 = 32
    // 1 i32 = 4
    // 1 u16 = 2
    // 1 u8 = 1
    // 1 u8 padding = 1
    // ticks: 201 * 16 = 3216
    // positions: 10 * 96 = 960
    // Total expected: 160 + 32 + 4 + 2 + 1 + 1 + 3216 + 960 = 4376

    println!("Expected size: {}", 160 + 32 + 4 + 2 + 1 + 1 + 3216 + 960);
}
