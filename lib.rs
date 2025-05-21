use anchor_lang::{
    prelude::*,
    system_program::{transfer, Transfer},
};
use pyth_solana_receiver_sdk::price_update::{get_feed_id_from_hex, PriceUpdateV2};

declare_id!("optnUT2hjddEs5BTojYaVE7V2A6st6sSnTVnsJpdY5F");

#[cfg(feature = "devnet")]
pub const PRICE_MAXIMUM_AGE: u64 = 7200; // 2 hours
#[cfg(not(feature = "devnet"))]
pub const PRICE_MAXIMUM_AGE: u64 = 5; // 5 seconds

pub const MULTIPLIER_PRECISION: u128 = 1_000_000_000;

pub const HOUSE_AUTHORITY_PREFIX: &'static [u8] = b"house_authority";

#[program]
pub mod optn {
    use super::*;

    pub fn create_bet(
        ctx: Context<CreateBet>,
        direction: Direction,
        lamports: u64,
        expiration: i64,
    ) -> Result<()> {
        // validate wager amount
        require_gte!(lamports, ctx.accounts.house.min_wager, OptnError::MinWager);
        require_gte!(ctx.accounts.house.max_wager, lamports, OptnError::MaxWager);

        // validate expiration
        require_gte!(
            expiration,
            ctx.accounts.house.min_expiration,
            OptnError::MinExpiration
        );
        require_gte!(
            ctx.accounts.house.max_expiration,
            expiration,
            OptnError::MaxExpiration
        );

        // validate available liquidity
        let profit_amount = u64::try_from(
            lamports as u128 * ctx.accounts.house.multiplier as u128 / MULTIPLIER_PRECISION,
        )
        .unwrap();
        let reserved_liquidity = ctx.accounts.house.reserved_liquidity + profit_amount;
        require_gte!(
            ctx.accounts.house.liquidity,
            reserved_liquidity,
            OptnError::InsufficientLiquidity
        );

        ctx.accounts.market.reserved_liquidity += profit_amount;
        ctx.accounts.market.total_wagered += lamports;
        ctx.accounts.market.active_bets += 1;

        ctx.accounts.house.reserved_liquidity = reserved_liquidity;
        ctx.accounts.house.total_wagered += lamports;
        ctx.accounts.house.active_bets += 1;

        let clock = Clock::get()?;

        ctx.accounts.bet.set_inner(Bet {
            market: ctx.accounts.market.key(),
            authority: ctx.accounts.user.key(),
            wagered_amount: lamports,
            profit_amount,
            final_payout: 0,
            entry_price: ctx
                .accounts
                .price_update
                .get_price_no_older_than(
                    &clock,
                    PRICE_MAXIMUM_AGE,
                    &get_feed_id_from_hex(&ctx.accounts.market.feed_id[..])?,
                )?
                .price
                .try_into()
                .unwrap(),
            settled_price: 0,
            direction,
            status: Status::Pending,
            created_at: clock.unix_timestamp,
            expires_at: clock.unix_timestamp + expiration,
            settled_at: None,
        });

        emit_bet_created(&ctx.accounts.bet.key(), &ctx.accounts.bet);
        emit_market_updated(&ctx.accounts.market.key(), &ctx.accounts.market);
        emit_house_updated(&ctx.accounts.house.key(), &ctx.accounts.house);

        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.user.to_account_info(),
                    to: ctx.accounts.house_authority.to_account_info(),
                },
            ),
            lamports,
        )
    }

    /// Only admin can provide settled_price at the expired time for the bet
    /// https://hermes.pyth.network/v2/updates/price/[expires_at]?ids%5B%5D=[feed_id]
    pub fn settle_bet(ctx: Context<SettleBet>, settled_price: u64) -> Result<()> {
        // validate status
        require!(
            ctx.accounts.bet.status == Status::Pending,
            OptnError::BetSettled
        );

        // validate expiry
        let current_time = Clock::get()?.unix_timestamp;
        require_gte!(
            current_time,
            ctx.accounts.bet.expires_at,
            OptnError::BetNotExpired
        );

        let won = match ctx.accounts.bet.direction {
            Direction::Buy => settled_price > ctx.accounts.bet.entry_price,
            Direction::Sell => ctx.accounts.bet.entry_price > settled_price,
        };

        if won {
            ctx.accounts.bet.status = Status::Won;
            ctx.accounts.bet.final_payout =
                ctx.accounts.bet.wagered_amount + ctx.accounts.bet.profit_amount;

            ctx.accounts.house.liquidity -= ctx.accounts.bet.profit_amount; // house lose
        } else {
            ctx.accounts.bet.status = Status::Lose;

            // TODO: cut % of house profit for revenue
            ctx.accounts.house.liquidity += ctx.accounts.bet.wagered_amount; // house profit
        }

        ctx.accounts.house.active_bets -= 1;
        ctx.accounts.house.settled_bets += 1;
        ctx.accounts.house.total_wagered -= ctx.accounts.bet.wagered_amount;
        ctx.accounts.house.reserved_liquidity -= ctx.accounts.bet.profit_amount;

        ctx.accounts.market.active_bets -= 1;
        ctx.accounts.market.settled_bets += 1;
        ctx.accounts.market.total_wagered -= ctx.accounts.bet.wagered_amount;
        ctx.accounts.market.reserved_liquidity -= ctx.accounts.bet.profit_amount;

        ctx.accounts.bet.settled_price = settled_price;
        ctx.accounts.bet.settled_at = Some(current_time);

        emit_bet_updated(&ctx.accounts.bet.key(), &ctx.accounts.bet);
        emit_market_updated(&ctx.accounts.market.key(), &ctx.accounts.market);
        emit_house_updated(&ctx.accounts.house.key(), &ctx.accounts.house);

        Ok(())
    }

    /// close bet account and reclaim rent fee back to the user
    pub fn close_bet(ctx: Context<CloseBet>) -> Result<()> {
        // validate status
        require!(
            ctx.accounts.bet.status != Status::Pending,
            OptnError::BetPending
        );

        if ctx.accounts.bet.final_payout > 0 {
            transfer(
                CpiContext::new(
                    ctx.accounts.system_program.to_account_info(),
                    Transfer {
                        from: ctx.accounts.house_authority.to_account_info(),
                        to: ctx.accounts.authority.to_account_info(),
                    },
                )
                .with_signer(&[&[
                    HOUSE_AUTHORITY_PREFIX,
                    &ctx.accounts.house.key().to_bytes(),
                    &[ctx.accounts.house.authority_bump],
                ]]),
                ctx.accounts.bet.final_payout,
            )?;
        }

        Ok(())
    }

    /// anyone can deposit liquidity
    pub fn deposit_liquidity(ctx: Context<DepositLiquidity>, lamports: u64) -> Result<()> {
        ctx.accounts.house.liquidity += lamports;
        ctx.accounts.house.total_deposits += lamports;

        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.payer.to_account_info(),
                    to: ctx.accounts.house_authority.to_account_info(),
                },
            ),
            lamports,
        )
    }

    /// beneficiary can withdraw liquidity
    pub fn withdraw_liquidity(ctx: Context<WithdrawLiquidity>, lamports: u64) -> Result<()> {
        let available_liquidity =
            ctx.accounts.house.liquidity - ctx.accounts.house.reserved_liquidity;
        require_gte!(
            available_liquidity,
            lamports,
            OptnError::InsufficientLiquidity
        );

        ctx.accounts.house.liquidity -= lamports;
        ctx.accounts.house.total_withdrawals += lamports;

        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.house_authority.to_account_info(),
                    to: ctx.accounts.beneficiary.to_account_info(),
                },
            )
            .with_signer(&[&[
                HOUSE_AUTHORITY_PREFIX,
                &ctx.accounts.house.key().to_bytes(),
                &[ctx.accounts.house.authority_bump],
            ]]),
            lamports,
        )
    }

    /// beneficiary can claim for profit
    pub fn claim_profit(ctx: Context<WithdrawLiquidity>) -> Result<()> {
        require_gt!(ctx.accounts.house.total_profit, 0, OptnError::NoProfit);

        ctx.accounts.house.claimed_profits += ctx.accounts.house.total_profit;
        ctx.accounts.house.total_profit = 0;

        transfer(
            CpiContext::new(
                ctx.accounts.system_program.to_account_info(),
                Transfer {
                    from: ctx.accounts.house_authority.to_account_info(),
                    to: ctx.accounts.beneficiary.to_account_info(),
                },
            )
            .with_signer(&[&[
                HOUSE_AUTHORITY_PREFIX,
                &ctx.accounts.house.key().to_bytes(),
                &[ctx.accounts.house.authority_bump],
            ]]),
            ctx.accounts.house.total_profit,
        )
    }

    pub fn create_house(
        ctx: Context<CreateHouse>,
        beneficiary: Pubkey,
        min_wager: u64,
        max_wager: u64,
        min_expiration: i64,
        max_expiration: i64,
        multiplier: u64,
        fee_basis_points: u16,
    ) -> Result<()> {
        require_gt!(min_wager, 0);
        require_gt!(max_wager, min_wager);

        require_gt!(min_expiration, 0);
        require_gt!(max_expiration, min_expiration);

        ctx.accounts.house.set_inner(House {
            admin: ctx.accounts.admin.key(),
            beneficiary,
            total_deposits: 0,
            total_withdrawals: 0,
            liquidity: 0,
            reserved_liquidity: 0,
            total_wagered: 0,
            total_profit: 0,
            claimed_profits: 0,
            active_bets: 0,
            settled_bets: 0,
            canceled_bets: 0,
            min_wager,
            max_wager,
            min_expiration,
            max_expiration,
            multiplier,
            fee_basis_points,
            authority_bump: ctx.bumps.house_authority,
        });

        Ok(())
    }

    pub fn update_wager_limits(
        ctx: Context<UpdateHouse>,
        min_wager: u64,
        max_wager: u64,
    ) -> Result<()> {
        require_gt!(min_wager, 0);
        require_gt!(max_wager, min_wager);

        ctx.accounts.house.min_wager = min_wager;
        ctx.accounts.house.max_wager = max_wager;

        Ok(())
    }

    pub fn create_market(ctx: Context<CreateMarket>, feed_id: String) -> Result<()> {
        ctx.accounts.market.set_inner(Market {
            house: ctx.accounts.house.key(),
            reserved_liquidity: 0,
            total_wagered: 0,
            active_bets: 0,
            settled_bets: 0,
            canceled_bets: 0,
            decimals: ctx
                .accounts
                .price_update
                .get_price_no_older_than(
                    &Clock::get()?,
                    PRICE_MAXIMUM_AGE,
                    &get_feed_id_from_hex(&feed_id[..])?,
                )?
                .exponent
                .abs()
                .try_into()
                .unwrap(),
            price_update: ctx.accounts.price_update.key(),
            feed_id,
        });

        Ok(())
    }
}

#[derive(Accounts)]
pub struct CreateBet<'info> {
    #[account(mut)]
    pub user: Signer<'info>,

    #[account(zero)]
    pub bet: Account<'info, Bet>,

    pub price_update: Account<'info, PriceUpdateV2>,

    #[account(mut, has_one = house, has_one = price_update)]
    pub market: Account<'info, Market>,

    #[account(mut)]
    pub house: Account<'info, House>,

    /// CHECK: OK
    #[account(mut,
        seeds = [HOUSE_AUTHORITY_PREFIX, &house.key().to_bytes()],
        bump = house.authority_bump,
    )]
    pub house_authority: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SettleBet<'info> {
    pub admin: Signer<'info>,

    #[account(mut, has_one = market)]
    pub bet: Account<'info, Bet>,

    #[account(mut, has_one = house)]
    pub market: Account<'info, Market>,

    #[account(mut, has_one = admin)]
    pub house: Account<'info, House>,
}

#[derive(Accounts)]
pub struct CloseBet<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,

    #[account(mut, has_one = market, has_one = authority, close = authority)]
    pub bet: Account<'info, Bet>,

    #[account(mut, has_one = house)]
    pub market: Account<'info, Market>,

    #[account(mut)]
    pub house: Account<'info, House>,

    /// CHECK: OK
    #[account(mut,
        seeds = [HOUSE_AUTHORITY_PREFIX, &house.key().to_bytes()],
        bump = house.authority_bump,
    )]
    pub house_authority: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct DepositLiquidity<'info> {
    #[account(mut)]
    pub payer: Signer<'info>,

    #[account(mut)]
    pub house: Account<'info, House>,

    /// CHECK: OK
    #[account(mut,
        seeds = [HOUSE_AUTHORITY_PREFIX, &house.key().to_bytes()],
        bump = house.authority_bump,
    )]
    pub house_authority: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct WithdrawLiquidity<'info> {
    #[account(mut)]
    pub beneficiary: Signer<'info>,

    #[account(mut, has_one = beneficiary)]
    pub house: Account<'info, House>,

    /// CHECK: OK
    #[account(mut,
        seeds = [HOUSE_AUTHORITY_PREFIX, &house.key().to_bytes()],
        bump = house.authority_bump,
    )]
    pub house_authority: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateHouse<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(zero)]
    pub house: Account<'info, House>,

    /// CHECK: OK
    #[account(mut,
        seeds = [HOUSE_AUTHORITY_PREFIX, &house.key().to_bytes()],
        bump,
    )]
    pub house_authority: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct UpdateHouse<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    #[account(mut, has_one = admin)]
    pub house: Account<'info, House>,
}

#[derive(Accounts)]
pub struct CreateMarket<'info> {
    #[account(mut)]
    pub admin: Signer<'info>,

    pub price_update: Account<'info, PriceUpdateV2>,

    #[account(zero)]
    pub market: Account<'info, Market>,

    #[account(has_one = admin)]
    pub house: Account<'info, House>,
}

#[account]
pub struct House {
    pub admin: Pubkey,
    pub beneficiary: Pubkey,

    /// Total amount deposited into the house liquidity pool
    pub total_deposits: u64,
    /// Total amount withdrawn from the house liquidity pool
    pub total_withdrawals: u64,

    /// Available liquidity for new bets
    pub liquidity: u64,
    /// Liquidity locked in active positions
    pub reserved_liquidity: u64,
    /// Total amount wagered across all bets
    pub total_wagered: u64,

    /// House profit metrics
    pub total_profit: u64, // Profits from lost bets & fees
    pub claimed_profits: u64, // Profits withdrawn by the beneficiary

    pub active_bets: u32,
    pub settled_bets: u32,
    pub canceled_bets: u32,

    pub min_wager: u64,
    pub max_wager: u64,

    pub min_expiration: i64,
    pub max_expiration: i64,

    /// Precision: 9 decimal places
    pub multiplier: u64,
    pub fee_basis_points: u16,

    pub authority_bump: u8,
}

#[account]
pub struct Market {
    pub house: Pubkey,

    pub reserved_liquidity: u64,
    pub total_wagered: u64,

    pub active_bets: u32,
    pub settled_bets: u32,
    pub canceled_bets: u32,

    pub decimals: u8,
    pub price_update: Pubkey,
    pub feed_id: String,
}

#[account]
pub struct Bet {
    pub market: Pubkey,
    pub authority: Pubkey,

    pub wagered_amount: u64,
    pub profit_amount: u64,
    pub final_payout: u64,

    pub entry_price: u64,
    pub settled_price: u64,
    pub direction: Direction,
    pub status: Status,

    pub created_at: i64,
    pub expires_at: i64,
    pub settled_at: Option<i64>,
}

#[repr(u8)]
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Buy = 0,  // Long (Call)
    Sell = 1, // Short (Put)
}

#[repr(u8)]
#[derive(AnchorSerialize, AnchorDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Pending = 0,
    Won = 1,
    Lose = 2,
    Canceled = 3, // Settlement failed due to missing price
}

#[error_code]
pub enum OptnError {
    #[msg("Wager amount exceeds the allowed maximum limit")]
    MaxWager,

    #[msg("Wager amount is below the allowed minimum limit")]
    MinWager,

    #[msg("Expiration time exceeds the maximum allowed limit")]
    MaxExpiration,

    #[msg("Expiration time is below the minimum allowed limit")]
    MinExpiration,

    #[msg("Insufficient available liquidity to proceed")]
    InsufficientLiquidity,

    #[msg("Bet cannot be settled because it is still active")]
    BetNotExpired,

    #[msg("Bet cannot be settled again because it has already been resolved")]
    BetSettled,

    #[msg("Bet cannot be closed because its outcome is still pending")]
    BetPending,

    #[msg("No claimable profit available for the house")]
    NoProfit,
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct HouseUpdatedData {
    pub liquidity: u64,
    pub reserved_liquidity: u64,
    pub total_wagered: u64,
    pub active_bets: u32,
    pub settled_bets: u32,
    pub canceled_bets: u32,
}

#[event]
pub struct HouseUpdatedEvent {
    pub pubkey: Pubkey,
    pub data: HouseUpdatedData,
}

pub fn emit_house_updated(pubkey: &Pubkey, house: &House) {
    emit!(HouseUpdatedEvent {
        pubkey: pubkey.key(),
        data: HouseUpdatedData {
            liquidity: house.liquidity,
            reserved_liquidity: house.reserved_liquidity,
            total_wagered: house.total_wagered,
            active_bets: house.active_bets,
            settled_bets: house.settled_bets,
            canceled_bets: house.canceled_bets,
        },
    });
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct MarketUpdatedData {
    pub reserved_liquidity: u64,
    pub total_wagered: u64,
    pub active_bets: u32,
    pub settled_bets: u32,
    pub canceled_bets: u32,
}

#[event]
pub struct MarketUpdatedEvent {
    pub pubkey: Pubkey,
    pub data: MarketUpdatedData,
}

pub fn emit_market_updated(pubkey: &Pubkey, market: &Market) {
    emit!(MarketUpdatedEvent {
        pubkey: pubkey.key(),
        data: MarketUpdatedData {
            reserved_liquidity: market.reserved_liquidity,
            total_wagered: market.total_wagered,
            active_bets: market.active_bets,
            settled_bets: market.settled_bets,
            canceled_bets: market.canceled_bets,
        },
    });
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct BetCreatedData {
    pub market: Pubkey,
    pub authority: Pubkey,
    pub wagered_amount: u64,
    pub profit_amount: u64,
    pub final_payout: u64,
    pub entry_price: u64,
    pub settled_price: u64,
    pub direction: Direction,
    pub status: Status,
    pub created_at: i64,
    pub expires_at: i64,
    pub settled_at: Option<i64>,
}

#[event]
pub struct BetCreatedEvent {
    pub pubkey: Pubkey,
    pub data: BetCreatedData,
}

pub fn emit_bet_created(pubkey: &Pubkey, bet: &Bet) {
    emit!(BetCreatedEvent {
        pubkey: pubkey.key(),
        data: BetCreatedData {
            market: bet.market.clone(),
            authority: bet.authority.clone(),
            wagered_amount: bet.wagered_amount,
            profit_amount: bet.profit_amount,
            final_payout: bet.final_payout,
            entry_price: bet.entry_price,
            settled_price: bet.settled_price,
            direction: bet.direction.clone(),
            status: bet.status.clone(),
            created_at: bet.created_at,
            expires_at: bet.expires_at,
            settled_at: bet.settled_at.clone(),
        },
    });
}

#[derive(AnchorSerialize, AnchorDeserialize)]
pub struct BetUpdatedData {
    pub final_payout: u64,
    pub settled_price: u64,
    pub status: Status,
    pub settled_at: Option<i64>,
}

#[event]
pub struct BetUpdatedEvent {
    pub pubkey: Pubkey,
    pub data: BetUpdatedData,
}

pub fn emit_bet_updated(pubkey: &Pubkey, bet: &Bet) {
    emit!(BetUpdatedEvent {
        pubkey: pubkey.key(),
        data: BetUpdatedData {
            final_payout: bet.final_payout,
            settled_price: bet.settled_price,
            status: bet.status.clone(),
            settled_at: bet.settled_at.clone(),
        },
    });
}
