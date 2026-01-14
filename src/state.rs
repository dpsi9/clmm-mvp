use solana_pubkey::Pubkey;

#[derive(Debug, Clone)]
pub struct Pool {
    pub token_0_mint: Pubkey,
    pub token_1_mint: Pubkey,

    pub token_0_vault: Pubkey,
    pub token_1_vault: Pubkey,

    pub sqrt_price: u128,
    pub liquidity: u128,
    pub tick_current: i32,

    pub ticks: [i128; 201], // liquidity net at each tick

    pub fee_bps: u16, // Fee in basis points

    pub positions: Vec<Position>,

    pub creator: Pubkey,
}

#[derive(Debug, Clone)]
pub struct Position {
    pub owner: Pubkey,
    pub liquidity: u128,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub tokens_owed_0: u64, // Unclaimed USDC fees
    pub tokens_owed_1: u64, // Unclaimed SOL fees
}

impl Pool {
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
            positions: Vec::new(),
            creator,
        }
    }

    pub fn add_liquidity(
        &mut self,
        owner: Pubkey,
        tick_lower: i32,
        tick_upper: i32,
        liquidity: u128,
    ) -> u64 {
        // Calculate how much USDC(token0) this liquidity requires
        let sqrt_lower = crate::math::get_sqrt_price_at_tick(tick_lower).unwrap();
        let sqrt_upper = crate::math::get_sqrt_price_at_tick(tick_upper).unwrap();
        let diff = sqrt_upper - sqrt_lower;

        //USDC needed = L * (√upper - √lower) * 2^64 / (√lower * √upper)
        let numerator = liquidity
            .checked_mul(diff)
            .and_then(|x| x.checked_mul(crate::math::Q64))
            .unwrap();
        let denominator = sqrt_lower.checked_mul(sqrt_upper).unwrap();
        let usdc_needed = (numerator / denominator) as u64;

        //update ticks
        let lower_idx = (tick_lower + 100) as usize;
        let upper_idx = (tick_upper + 100) as usize;

        self.ticks[lower_idx] += liquidity as i128;
        self.ticks[upper_idx] -= liquidity as i128;

        if tick_lower <= self.tick_current && self.tick_current < tick_upper {
            self.liquidity += liquidity;
        }

        // add user's position to the pool's position array
        self.positions.push(Position {
            owner,
            liquidity,
            tick_lower,
            tick_upper,
            tokens_owed_0: 0,
            tokens_owed_1: 1,
        });

        usdc_needed
    }

    pub fn update_tick_if_crossed(&mut self, new_tick: i32) {
        //Only update if tick is crossed
        if new_tick != self.tick_current {
            let new_idx = (new_tick + 100) as usize;

            // Add liquidity net of the tick we just crossed
            self.liquidity = (self.liquidity as i128 + self.ticks[new_idx]) as u128;

            self.tick_current = new_tick;
        }
    }

    pub fn get_position(&self, owner: &Pubkey) -> Option<&Position> {
        self.positions.iter().find(|p| &p.owner == owner)
    }

    pub fn get_position_mut(&mut self, owner: &Pubkey) -> Option<&mut Position> {
        self.positions.iter_mut().find(|p| &p.owner == owner)
    }
}
