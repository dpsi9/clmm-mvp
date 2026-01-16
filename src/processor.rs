use borsh::{BorshDeserialize, BorshSerialize};
use solana_account_info::{AccountInfo, next_account_info};
use solana_msg::msg;
use solana_program_entrypoint::ProgramResult;
use solana_program_error::ProgramError;
use solana_pubkey::Pubkey;

use crate::{
    error::ClmmError,
    math::{self, get_sqrt_price_at_tick},
    state::Pool,
};

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
    _program_id: &Pubkey,
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
        .map_err(|_| {
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
    let accounts_iter = &mut accounts.iter();

    let user = next_account_info(accounts_iter)?;
    let pool_account = next_account_info(accounts_iter)?;

    if !pool_account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    let pool_data = pool_account.data.borrow();
    let mut pool =
        Pool::try_from_slice(&pool_data).map_err(|_| ProgramError::InvalidAccountData)?;

    if pool.liquidity == 0 {
        msg!("Pool has no liquidity");
        return Err(ClmmError::PoolNotInitialized.into());
    }

    let new_sqrt_price =
        math::get_next_sqrt_price_buy_sol(pool.sqrt_price, pool.liquidity, usdc_amount)
            .map_err(|_| ProgramError::InvalidInstructionData)?;

    let sol_output = math::get_sol_output(pool.liquidity, pool.sqrt_price, new_sqrt_price)
        .map_err(|_| ProgramError::InvalidInstructionData)?;

    pool.sqrt_price = new_sqrt_price;

    // Update tick if crossed
    let new_tick = ((new_sqrt_price >> 64) - (1 << 64)) / 3277;
    pool.update_tick_if_crossed(new_tick as i32);

    msg!(
        "User {} bought {} SOL with {} USDC",
        user.key,
        sol_output,
        usdc_amount
    );

    // Save pool
    let mut pool_data_mut = pool_account.data.borrow_mut();
    pool.serialize(&mut &mut pool_data_mut[..])
        .map_err(|_| ProgramError::BorshIoError)?;

    Ok(())
}

fn remove_liquidity(accounts: &[AccountInfo], liquidity: u128) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();

    let user = next_account_info(accounts_iter)?;
    let pool_account = next_account_info(accounts_iter)?;

    if !pool_account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    let pool_data = &mut &pool_account.data.borrow();
    let mut pool =
        Pool::try_from_slice(&pool_data).map_err(|_| ProgramError::InvalidAccountData)?;

    let position = pool
        .get_position_mut(user.key)
        .ok_or(ClmmError::PositionNotFound)?;

    if position.liquidity < liquidity {
        return Err(ClmmError::InsufficientLiquidity.into());
    }

    let usdc_return = (liquidity / 1000) as u64;
    let sol_return = (liquidity / 2000) as u64;

    // Update position and capture tick values before releasing mutable borrow
    position.liquidity -= liquidity;
    let tick_lower = position.tick_lower;
    let tick_upper = position.tick_upper;
    let position_liquidity = position.liquidity;

    // Update pool liquidity if active
    if tick_lower <= pool.tick_current && pool.tick_current < tick_upper {
        pool.liquidity -= liquidity;
    }

    // Update ticks
    let lower_idx = (tick_lower + 100) as usize;
    let upper_idx = (tick_upper + 100) as usize;
    pool.ticks[lower_idx] -= liquidity as i128;
    pool.ticks[upper_idx] += liquidity as i128;

    // Remove if empty
    if position_liquidity == 0 {
        pool.remove_position(user.key).map_err(|e| {
            msg!("Error removing position: {}", e);
            ProgramError::InvalidInstructionData
        })?;
    }

    msg!("Returns: {} USDC, {} SOL", usdc_return, sol_return);

    // Save pool
    let mut pool_data_mut = pool_account.data.borrow_mut();
    pool.serialize(&mut &mut pool_data_mut[..])
        .map_err(|_| ProgramError::BorshIoError)?;

    Ok(())
}
