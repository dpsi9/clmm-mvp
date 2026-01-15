use borsh::{BorshDeserialize, BorshSerialize};
use solana_account_info::{AccountInfo, next_account_info};
use solana_msg::msg;
use solana_program_entrypoint::ProgramResult;
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;

use crate::{error::ClmmError, math::get_sqrt_price_at_tick, state::Pool};

#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub enum ClmmInstruction {
    InitializePool {
        token_0_mint: Pubkey,
        token_1_mint: Pubkey,
        token_0_vault: Pubkey,
        token_1_vault: Pubkey,
    },
    AddLiquidity {
        tick_lower: i32,
        tick_upper: i32,
        usdc_amount: u64,
    },
    BuySol {
        usdc_amount: u64,
    },
    RemoveLiquidity {
        liquidity: u128,
    },
}

pub fn process(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = ClmmInstruction::try_from_slice(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    msg!("Instruction: {:?}", instruction);

    match instruction {
        ClmmInstruction::InitializePool {
            token_0_mint,
            token_1_mint,
            token_0_vault,
            token_1_vault,
        } => {
            initialize_pool(
                program_id,
                accounts,
                token_0_mint,
                token_1_mint,
                token_0_vault,
                token_1_vault,
            )?;
        }
        ClmmInstruction::AddLiquidity {
            tick_lower,
            tick_upper,
            usdc_amount,
        } => {
            add_liquidity(accounts, tick_lower, tick_upper, usdc_amount)?;
        }
        ClmmInstruction::BuySol { usdc_amount } => {
            buy_sol(accounts, usdc_amount)?;
        }
        ClmmInstruction::RemoveLiquidity { liquidity } => {
            remove_liquidity(accounts, liquidity)?;
        }
    }

    Ok(())
}

fn initialize_pool(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    token_0_mint: Pubkey,
    token_1_mint: Pubkey,
    token_0_vault: Pubkey,
    token_1_vault: Pubkey,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();

    let payer = next_account_info(accounts_iter)?;
    let pool_account = next_account_info(accounts_iter)?;

    if !pool_account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    let pool = Pool::new(
        token_0_mint,
        token_1_mint,
        token_0_vault,
        token_1_vault,
        *payer.key,
    );

    // let pool_data = pool.try_to_vec
    if pool_account.data_len() < Pool::LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    pool.serialize(&mut &mut pool_account.data.borrow_mut()[..])
        .map_err(|_| ProgramError::BorshIoError)?;
    Ok(())
}

fn add_liquidity(
    accounts: &[AccountInfo],
    tick_lower: i32,
    tick_upper: i32,
    usdc_amount: u64,
) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();

    let user = next_account_info(accounts_iter)?;
    let pool_account = next_account_info(accounts_iter)?;

    let pool_data = pool_account.data.borrow();
    let mut pool = Pool::try_from_slice(&pool_data).map_err(|e| {
        msg!("Failed to deserialize pool: {:?}", e);
        ProgramError::InvalidAccountData
    })?;

    if tick_lower >= tick_upper || tick_lower < -100 || tick_upper > 100 {
        return Err(ClmmError::InvalidTickRange.into());
    }

    let sqrt_lower = get_sqrt_price_at_tick(tick_lower).map_err(|e| {
        msg!("Failed to get sqrt price at tick {}: {:?}", tick_lower, e);
        ProgramError::InvalidInstructionData
    })?;

    let sqrt_upper = get_sqrt_price_at_tick(tick_upper).map_err(|e| {
        msg!("Failed to get sqrt price at tick {}: {:?}", tick_upper, e);
        ProgramError::InvalidInstructionData
    })?;

    let diff = sqrt_upper.checked_sub(sqrt_lower).ok_or_else(|| {
        msg!("Math overflow: sqrt_upper - sqrt_lower");
        ClmmError::MathOverflow
    })?;

    // L = amount_0 * √lower * √upper / (√upper - √lower)
    let numerator = (usdc_amount as u128)
        .checked_mul(sqrt_lower)
        .and_then(|x| x.checked_mul(sqrt_upper))
        .ok_or_else(|| {
            msg!("Math overflow: numerator calculation");
            ClmmError::MathOverflow
        })?;

    let liquidity = numerator / diff;

    msg!(
        "Calculated liquidity: {} from {} USDC",
        liquidity,
        usdc_amount
    );

    let usdc_needed = pool
        .add_liquidity(*user.key, tick_lower, tick_upper, liquidity)
        .map_err(|e| {
            msg!("Failed to add liquidity: {}", liquidity);
            ProgramError::InvalidInstructionData
        })?;

    msg!(
        "Added {} liquidity (expected {} USDC, got {})",
        liquidity,
        usdc_needed,
        usdc_amount
    );

    pool.serialize(&mut &mut pool_account.data.borrow_mut()[..])
        .map_err(|_| ProgramError::BorshIoError)?;

    Ok(())
}

fn buy_sol(accounts: &[AccountInfo], usdc_amount: u64) -> ProgramResult {
    Ok(())
}

fn remove_liquidity(accounts: &[AccountInfo], liquidity: u128) -> ProgramResult {
    Ok(())
}
