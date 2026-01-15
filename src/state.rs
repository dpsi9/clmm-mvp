use borsh::{BorshDeserialize, BorshSerialize};
use solana_pubkey::Pubkey;

// Fixed-size pool for MVP
const MAX_POSITIONS: usize = 10; // Only 10 positions max

#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct Pool {
    pub token_0_mint: Pubkey,
    pub token_1_mint: Pubkey,
    pub token_0_vault: Pubkey,
    pub token_1_vault: Pubkey,
    pub sqrt_price: u128,
    pub liquidity: u128,
    pub tick_current: i32,
    pub ticks: [i128; 201],
    pub fee_bps: u16,

    // FIXED SIZE: Array of optional positions
    pub positions: [Option<Position>; MAX_POSITIONS],
    pub positions_count: u8, // How many positions are filled

    pub creator: Pubkey,
}

#[derive(BorshDeserialize, BorshSerialize, Debug, Clone)]
pub struct Position {
    pub owner: Pubkey,      // 32 bytes
    pub liquidity: u128,    // 16 bytes
    pub tick_lower: i32,    // 4 bytes
    pub tick_upper: i32,    // 4 bytes
    pub tokens_owed_0: u64, // 8 bytes
    pub tokens_owed_1: u64, // 8 bytes
      // Total: 72 bytes
}

impl Pool {
    pub const LEN: usize = 32 + 32 + 32 + 32 +    // 4 × Pubkey = 128
                          16 + 16 +               // 2 × u128 = 32  
                          4 +                     // i32 = 4
                          (16 * 201) +            // ticks array = 3216
                          2 +                     // u16 = 2
                          (72 * MAX_POSITIONS) +  // positions array = 720
                          1 +                     // u8 = 1
                          32; // creator Pubkey = 32
    // TOTAL: 128 + 32 + 4 + 3216 + 2 + 720 + 1 + 32 = 4135 bytes

    pub fn new(
        token_0_mint: Pubkey,
        token_1_mint: Pubkey,
        token_0_vault: Pubkey,
        token_1_vault: Pubkey,
        creator: Pubkey,
    ) -> Self {
        Self {
            token_0_mint,
            token_1_mint,
            token_0_vault,
            token_1_vault,
            sqrt_price: 1 << 64,
            liquidity: 0,
            tick_current: 0,
            ticks: [0; 201],
            fee_bps: 0,
            positions: [None, None, None, None, None, None, None, None, None, None], // 10 None
            positions_count: 0,
            creator,
        }
    }

    pub fn add_liquidity(
        &mut self,
        owner: Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        liquidity: u128,
    ) -> Result<u64, &'static str> {
        // Check if we have space
        if self.positions_count as usize >= MAX_POSITIONS {
            return Err("Maximum positions reached");
        }

        // Calculate USDC needed (your existing logic)
        let sqrt_lower =
            crate::math::get_sqrt_price_at_tick(tick_lower).map_err(|_| "Invalid tick")?;
        let sqrt_upper =
            crate::math::get_sqrt_price_at_tick(tick_upper).map_err(|_| "Invalid tick")?;
        let diff = sqrt_upper.checked_sub(sqrt_lower).ok_or("Math overflow")?;

        let numerator = liquidity
            .checked_mul(diff)
            .and_then(|x| x.checked_mul(crate::math::Q64))
            .ok_or("Math overflow")?;
        let denominator = sqrt_lower.checked_mul(sqrt_upper).ok_or("Math overflow")?;
        let usdc_needed = (numerator / denominator) as u64;

        // Update ticks
        let lower_idx = (tick_lower + 100) as usize;
        let upper_idx = (tick_upper + 100) as usize;

        self.ticks[lower_idx] += liquidity as i128;
        self.ticks[upper_idx] -= liquidity as i128;

        if tick_lower <= self.tick_current && self.tick_current < tick_upper {
            self.liquidity += liquidity;
        }

        // Add position to first available slot
        for slot in self.positions.iter_mut() {
            if slot.is_none() {
                *slot = Some(Position {
                    owner,
                    liquidity,
                    tick_lower,
                    tick_upper,
                    tokens_owed_0: 0,
                    tokens_owed_1: 0,
                });
                self.positions_count += 1;
                break;
            }
        }

        Ok(usdc_needed)
    }

    pub fn update_tick_if_crossed(&mut self, new_tick: i32) {
        if new_tick != self.tick_current {
            let new_idx = (new_tick + 100) as usize;
            self.liquidity = (self.liquidity as i128 + self.ticks[new_idx]) as u128;
            self.tick_current = new_tick;
        }
    }

    pub fn get_position(&self, owner: &Pubkey) -> Option<&Position> {
        self.positions
            .iter()
            .filter_map(|p| p.as_ref())
            .find(|p| &p.owner == owner)
    }

    pub fn get_position_mut(&mut self, owner: &Pubkey) -> Option<&mut Position> {
        self.positions
            .iter_mut()
            .filter_map(|p| p.as_mut())
            .find(|p| &p.owner == owner)
    }

    pub fn remove_position(&mut self, owner: &Pubkey) -> Result<(), &'static str> {
        for slot in self.positions.iter_mut() {
            if let Some(pos) = slot {
                if &pos.owner == owner {
                    *slot = None;
                    self.positions_count -= 1;
                    return Ok(());
                }
            }
        }
        Err("Position not found")
    }
}
