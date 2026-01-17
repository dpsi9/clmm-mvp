use bytemuck::{Pod, Zeroable};
use solana_pubkey::Pubkey;

// Fixed-size pool for MVP
pub const MAX_POSITIONS: usize = 10; // Only 10 positions max
pub const MAX_TICKS: usize = 201;

/// Position stored in the pool - uses a flag instead of Option for bytemuck compatibility
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct Position {
    pub owner: Pubkey,      // 32 bytes
    pub liquidity: u128,    // 16 bytes (needs 16-byte alignment, 32 is 16-aligned)
    pub tick_lower: i32,    // 4 bytes
    pub tick_upper: i32,    // 4 bytes
    pub tokens_owed_0: u64, // 8 bytes
    pub tokens_owed_1: u64, // 8 bytes
    pub is_active: u8,      // 1 = active, 0 = empty (1 byte)
    pub _padding: [u8; 15], // padding to reach 16-byte boundary (total: 96 bytes)
}

// Safety: Position is #[repr(C)] with no padding issues and all fields are Pod
unsafe impl Pod for Position {}
unsafe impl Zeroable for Position {}

impl Position {
    pub fn is_active(&self) -> bool {
        self.is_active == 1
    }

    pub fn new(owner: Pubkey, liquidity: u128, tick_lower: i32, tick_upper: i32) -> Self {
        Self {
            owner,
            liquidity,
            tick_lower,
            tick_upper,
            tokens_owed_0: 0,
            tokens_owed_1: 0,
            is_active: 1,
            _padding: [0; 15],
        }
    }

    pub fn clear(&mut self) {
        self.owner = Pubkey::default();
        self.liquidity = 0;
        self.tick_lower = 0;
        self.tick_upper = 0;
        self.tokens_owed_0 = 0;
        self.tokens_owed_1 = 0;
        self.is_active = 0;
    }
}

/// Pool state - zero-copy for efficient access without stack allocation
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct Pool {
    pub token_0_mint: Pubkey,                 // 32 bytes (offset 0)
    pub token_1_mint: Pubkey,                 // 32 bytes (offset 32)
    pub token_0_vault: Pubkey,                // 32 bytes (offset 64)
    pub token_1_vault: Pubkey,                // 32 bytes (offset 96)
    pub creator: Pubkey,                      // 32 bytes (offset 128)
    pub sqrt_price: u128,                     // 16 bytes (offset 160)
    pub liquidity: u128,                      // 16 bytes (offset 176)
    pub tick_current: i32,                    // 4 bytes  (offset 192)
    pub fee_bps: u16,                         // 2 bytes  (offset 196)
    pub positions_count: u8,                  // 1 byte   (offset 198)
    pub _padding: [u8; 9],                    // 9 bytes to reach 16-byte alignment (offset 199-207)
    pub ticks: [i128; MAX_TICKS],             // 16 * 201 = 3216 bytes (offset 208)
    pub positions: [Position; MAX_POSITIONS], // 96 * 10 = 960 bytes (offset 3424)
} // Total: 3424 + 960 = 4384 bytes

// Safety: Pool is #[repr(C)] with explicit padding and all fields are Pod
unsafe impl Pod for Pool {}
unsafe impl Zeroable for Pool {}

impl Pool {
    pub const LEN: usize = std::mem::size_of::<Pool>();

    /// Initialize a new pool in place
    pub fn initialize(
        &mut self,
        token_0_mint: Pubkey,
        token_1_mint: Pubkey,
        token_0_vault: Pubkey,
        token_1_vault: Pubkey,
        creator: Pubkey,
    ) {
        self.token_0_mint = token_0_mint;
        self.token_1_mint = token_1_mint;
        self.token_0_vault = token_0_vault;
        self.token_1_vault = token_1_vault;
        self.creator = creator;
        self.sqrt_price = 1 << 64; // Q64 representation of 1.0
        self.liquidity = 0;
        self.tick_current = 0;
        self.fee_bps = 0;
        self.positions_count = 0;
        self._padding = [0; 9];
        // ticks and positions are already zeroed by Zeroable
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

        // Calculate USDC needed
        let sqrt_lower =
            crate::math::get_sqrt_price_at_tick(tick_lower).map_err(|_| "Invalid tick")?;
        let sqrt_upper =
            crate::math::get_sqrt_price_at_tick(tick_upper).map_err(|_| "Invalid tick")?;
        let diff = sqrt_upper.checked_sub(sqrt_lower).ok_or("Math overflow")?;

        // Scale down to avoid overflow - sqrt values are Q64
        let sqrt_lower_scaled = sqrt_lower >> 32;
        let sqrt_upper_scaled = sqrt_upper >> 32;
        let diff_scaled = diff >> 32;

        let numerator = liquidity.checked_mul(diff_scaled).ok_or("Math overflow")?;
        let denominator = sqrt_lower_scaled
            .checked_mul(sqrt_upper_scaled)
            .ok_or("Math overflow")?;
        let usdc_needed = if denominator > 0 {
            (numerator / denominator) as u64
        } else {
            0
        };

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
            if !slot.is_active() {
                *slot = Position::new(owner, liquidity, tick_lower, tick_upper);
                self.positions_count += 1;
                break;
            }
        }

        Ok(usdc_needed)
    }

    pub fn update_tick_if_crossed(&mut self, new_tick: i32) {
        if new_tick != self.tick_current {
            // Clamp to valid range
            let clamped_tick = new_tick.max(-100).min(100);
            let new_idx = (clamped_tick + 100) as usize;
            if new_idx < MAX_TICKS {
                self.liquidity = (self.liquidity as i128 + self.ticks[new_idx]) as u128;
            }
            self.tick_current = clamped_tick;
        }
    }

    pub fn get_position(&self, owner: &Pubkey) -> Option<&Position> {
        self.positions
            .iter()
            .filter(|p| p.is_active())
            .find(|p| &p.owner == owner)
    }

    pub fn get_position_mut(&mut self, owner: &Pubkey) -> Option<&mut Position> {
        self.positions
            .iter_mut()
            .filter(|p| p.is_active())
            .find(|p| &p.owner == owner)
    }

    pub fn remove_position(&mut self, owner: &Pubkey) -> Result<(), &'static str> {
        for slot in self.positions.iter_mut() {
            if slot.is_active() && &slot.owner == owner {
                slot.clear();
                self.positions_count -= 1;
                return Ok(());
            }
        }
        Err("Position not found")
    }
}
