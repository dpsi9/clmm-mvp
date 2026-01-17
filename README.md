# mini-clmm

A minimal Concentrated Liquidity Market Maker (CLMM) implementation on Solana.

## Overview

Educational implementation of Uniswap v3-style concentrated liquidity on Solana. Built to understand tick-based AMM mechanics.

## Features

- **Concentrated Liquidity** - LPs provide liquidity within custom price ranges
- **Tick-based Pricing** - Q64 fixed-point math for price calculations
- **Zero-copy State** - Uses bytemuck to avoid Solana's 4KB stack limit

## Instructions

| Instruction | Description |
|-------------|-------------|
| `InitializePool` | Create a new liquidity pool |
| `AddLiquidity` | Add liquidity within a tick range |
| `RemoveLiquidity` | Remove liquidity from a position |
| `BuySol` | Swap USDC for SOL |

## Quick Start

```bash
# Clone and build
git clone https://github.com/dpsi9/clmm-mvp
cd mini-clmm
cargo build-sbf

# Run tests
cargo test

# Deploy (requires local validator)
solana program deploy target/deploy/mini_clmm.so
```

## Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      Pool Account                       │
├─────────────────────────────────────────────────────────┤
│  token_0_mint, token_1_mint (vaults)                    │
│  sqrt_price (Q64)  │  tick_current  │  liquidity        │
├─────────────────────────────────────────────────────────┤
│  ticks[201]        │  Active liquidity per tick         │
├─────────────────────────────────────────────────────────┤
│  positions[10]     │  LP positions with tick ranges     │
└─────────────────────────────────────────────────────────┘

Price = (sqrt_price / 2^64)²
Tick i → sqrt_price = 1.0001^(i/2) * 2^64
```

## Structure

```
src/
├── lib.rs        # Entrypoint
├── processor.rs  # Instruction handlers
├── state.rs      # Pool & Position structs
├── math.rs       # Tick & liquidity math
└── error.rs      # Custom errors
```

## Constraints

- Max 10 positions per pool
- Tick range: -100 to 100
- Fixed pool size: ~4.4KB

