use crate::error::ClmmError;

pub const MIN_TICK: i32 = -100;
pub const MAX_TICK: i32 = 100;

pub const Q64: u128 = 1 << 64;

/// Here liquidity = If price moves by this much, how many tokens does that represent

/// price = 1.0001 ^ (tick / 2)
pub fn get_sqrt_price_at_tick(tick: i32) -> Result<u128, ClmmError> {
    if tick < MIN_TICK || tick > MAX_TICK {
        return Err(ClmmError::InvalidTickRange);
    }

    // Approximation: √1.0001 ≈ 1.00005
    let base = Q64;

    if tick == 0 {
        Ok(base)
    } else if tick > 0 {
        // for every tick there is approx 0.0005 change so 0.0005 * Q64 = 3277
        let increase = (tick as u128) * 3277;
        base.checked_add(increase).ok_or(ClmmError::MathOverflow)
    } else {
        let decrease = (-tick as u128) * 3277;
        base.checked_sub(decrease).ok_or(ClmmError::MathOverflow)
    }
}

/// How much the price changes if I buy sol using usdc
/// √P_new = √P + amount / L
pub fn get_next_sqrt_price_buy_sol(
    sqrt_price: u128,
    liquidity: u128,
    usdc_amount: u64,
) -> Result<u128, ClmmError> {
    if liquidity == 0 {
        return Ok(sqrt_price);
    }
    // amount / L in Q64.64 format
    let delta = ((usdc_amount as u128) << 64) / liquidity;
    sqrt_price.checked_add(delta).ok_or(ClmmError::MathOverflow)
}

///If I spent some USDC and the price moved from √P_old to √P_new,
/// how much SOL did I actually receive?
/// Calculate SOL output when buying with USDC
/// ΔSOL = L × (√P_new - √P_old)
pub fn get_sol_output(
    liquidity: u128,
    sqrt_price_old: u128,
    sqrt_price_new: u128,
) -> Result<u64, ClmmError> {
    let diff = sqrt_price_new
        .checked_sub(sqrt_price_old)
        .ok_or(ClmmError::MathOverflow)?;

    let sol_raw = (liquidity * diff) >> 64; // as it is in Q64, we need to convert back into normal number

    Ok(sol_raw as u64)
}
