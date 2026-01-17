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

    if pool_account.data_len() < Pool::LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Zero-copy: directly modify account data without stack allocation
    let mut data = pool_account.data.borrow_mut();
    let pool = bytemuck::from_bytes_mut::<Pool>(&mut data[..Pool::LEN]);

    pool.initialize(
        token_0_mint,
        token_1_mint,
        token_0_vault,
        token_1_vault,
        *payer.key,
    );

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

    if !pool_account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    if pool_account.data_len() < Pool::LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Zero-copy access
    let mut data = pool_account.data.borrow_mut();
    let pool = bytemuck::from_bytes_mut::<Pool>(&mut data[..Pool::LEN]);

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
    // Note: sqrt values are in Q64 format (fixed-point with 64 fractional bits)
    // sqrt_lower and sqrt_upper are both around 2^64, so their product would overflow
    // We need to divide before multiplying to avoid overflow
    // Simplified approach: L ≈ amount * sqrt_price^2 / diff
    // Since sqrt_price ≈ 2^64 and diff is small, we simplify to:
    // L = amount * (sqrt_lower / diff) * sqrt_upper / 2^64
    
    // First: amount * sqrt_lower (fits in u128 since amount is u64 and sqrt_lower is ~2^64)
    let step1 = (usdc_amount as u128)
        .checked_mul(sqrt_lower)
        .ok_or_else(|| {
            msg!("Math overflow: step1");
            ClmmError::MathOverflow
        })?;
    
    // Divide by diff to get a reasonable intermediate value
    let step2 = step1 / diff;
    
    // Multiply by sqrt_upper and divide by Q64 to get final liquidity
    // step2 * sqrt_upper could overflow, so we do: (step2 / 2^32) * (sqrt_upper / 2^32)
    let liquidity = (step2 >> 32)
        .checked_mul(sqrt_upper >> 32)
        .ok_or_else(|| {
            msg!("Math overflow: liquidity calculation");
            ClmmError::MathOverflow
        })?;

    msg!(
        "Calculated liquidity: {} from {} USDC",
        liquidity,
        usdc_amount
    );

    let usdc_needed = pool
        .add_liquidity(*user.key, tick_lower, tick_upper, liquidity)
        .map_err(|e| {
            msg!("Failed to add liquidity: {}", e);
            ProgramError::InvalidInstructionData
        })?;

    msg!(
        "Added {} liquidity (expected {} USDC, got {})",
        liquidity,
        usdc_needed,
        usdc_amount
    );

    Ok(())
}

fn buy_sol(accounts: &[AccountInfo], usdc_amount: u64) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();

    let user = next_account_info(accounts_iter)?;
    let pool_account = next_account_info(accounts_iter)?;

    if !pool_account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    if pool_account.data_len() < Pool::LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Zero-copy access
    let mut data = pool_account.data.borrow_mut();
    let pool = bytemuck::from_bytes_mut::<Pool>(&mut data[..Pool::LEN]);

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
    // new_tick = (sqrt_price - Q64) / 3277, clamped to valid range
    let sqrt_q64 = 1u128 << 64;
    let new_tick = if new_sqrt_price >= sqrt_q64 {
        let diff = new_sqrt_price - sqrt_q64;
        (diff / 3277) as i32
    } else {
        let diff = sqrt_q64 - new_sqrt_price;
        -((diff / 3277) as i32)
    };
    // Clamp to valid tick range
    let new_tick = new_tick.max(-100).min(100);
    pool.update_tick_if_crossed(new_tick);

    msg!(
        "User {} bought {} SOL with {} USDC",
        user.key,
        sol_output,
        usdc_amount
    );

    Ok(())
}

fn remove_liquidity(accounts: &[AccountInfo], liquidity: u128) -> ProgramResult {
    let accounts_iter = &mut accounts.iter();

    let user = next_account_info(accounts_iter)?;
    let pool_account = next_account_info(accounts_iter)?;

    if !pool_account.is_writable {
        return Err(ProgramError::InvalidAccountData);
    }

    if pool_account.data_len() < Pool::LEN {
        return Err(ProgramError::AccountDataTooSmall);
    }

    // Zero-copy access
    let mut data = pool_account.data.borrow_mut();
    let pool = bytemuck::from_bytes_mut::<Pool>(&mut data[..Pool::LEN]);

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

    Ok(())
}
