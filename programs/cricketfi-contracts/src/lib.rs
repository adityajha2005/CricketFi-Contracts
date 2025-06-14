use anchor_lang::prelude::*;
use anchor_lang::solana_program::{
    program::{invoke, invoke_signed},
    system_instruction,
};

declare_id!("6F9BXG9UXFix5yJ2sZfsbXAg7KcrT3oW2YxjorWik1Bo");

#[program]
pub mod cricketfi_contracts {
    use super::*;

    pub fn initialize(ctx: Context<InitializePlatform>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        let platform = &mut ctx.accounts.platform;
        platform.authority = ctx.accounts.authority.key();
        platform.total_matches = 0;
        platform.total_bets = 0;
        platform.treasury_balance = 0;
        platform.fee_percentage = 2; // Default 2% fee
        Ok(())
    }

    pub fn placebet(ctx: Context<PlaceBet>, match_id: [u8; 32], amount: u64, team: u8) -> Result<()> {
        let bet_account = &mut ctx.accounts.bet_account;
        let match_account = &mut ctx.accounts.match_account;
        let platform = &mut ctx.accounts.platform;
        let match_vault = &ctx.accounts.match_vault;
        let user_bet_tracker = &mut ctx.accounts.user_bet_tracker;

        // Add match ID validation
        require!(
            match_account.match_id == match_id,
            CricketFiError::InvalidMatch
        );

        // Match status check, min and max bet amount check, team check
        require!(
            match_account.status == MatchStatus::Active,
            CricketFiError::InvalidMatchStatus
        );

        // Check if match has already started
        let current_time = Clock::get()?.unix_timestamp;
        require!(
            current_time < match_account.start_time,
            CricketFiError::BettingClosed
        );

        // Check if user has already bet on this match
        require!(
            !user_bet_tracker.has_bet,
            CricketFiError::UserAlreadyBet
        );

        require!(
            amount >= match_account.min_bet_amount && amount <= match_account.max_bet_amount,
            CricketFiError::InvalidBetAmount
        );
        require!(team == 0 || team == 1, CricketFiError::InvalidTeam);

        // Validate odds are reasonable (between 1.01x and 10.00x, i.e., 101 to 1000 basis points)
        let odds_to_use = if team == 0 { 
            match_account.odds_team1 
        } else { 
            match_account.odds_team2 
        };
        require!(
            odds_to_use >= 101 && odds_to_use <= 1000,
            CricketFiError::InvalidOdds
        );

        // Mark that this user has bet on this match
        user_bet_tracker.user = ctx.accounts.better.key();
        user_bet_tracker.match_id = match_id;
        user_bet_tracker.has_bet = true;

        //Calculate fee & principal
        let fee = amount
            .checked_mul(platform.fee_percentage as u64)
            .ok_or(ErrorCode::NumericalOverflow)?
            .checked_div(100)
            .ok_or(ErrorCode::NumericalOverflow)?;
        
        require!(fee <= u32::MAX as u64, ErrorCode::NumericalOverflow);
        let principal = amount.checked_sub(fee).ok_or(ErrorCode::NumericalOverflow)?;

        // Populate bet account
        bet_account.better = ctx.accounts.better.key();
        bet_account.match_id = match_id;
        bet_account.amount = principal;
        bet_account.team = team;
        bet_account.bet_time = Clock::get()?.unix_timestamp;
        bet_account.claimed = false;
        // Store odds at bet placement (frozen) - odds in basis points (e.g., 150 = 1.5x payout)
        bet_account.odds_at_bet = if team == 0 { 
            match_account.odds_team1 
        } else { 
            match_account.odds_team2 
        };
        // Updating match pool
        if team == 0 {
            match_account.team1_pool_amount = match_account
                .team1_pool_amount
                .checked_add(principal)
                .ok_or(ErrorCode::NumericalOverflow)?;
        } else {
            match_account.team2_pool_amount = match_account
                .team2_pool_amount
                .checked_add(principal)
                .ok_or(ErrorCode::NumericalOverflow)?;
        }
        match_account.total_pool_amount = match_account
            .total_pool_amount
            .checked_add(principal)
            .ok_or(ErrorCode::NumericalOverflow)?;

        platform.total_bets = platform
            .total_bets
            .checked_add(1)
            .ok_or(ErrorCode::NumericalOverflow)?;
        platform.treasury_balance = platform
            .treasury_balance
            .checked_add(fee as u32)
            .ok_or(ErrorCode::NumericalOverflow)?;

        // Transfer principal to match-specific vault
        invoke(
            &system_instruction::transfer(
                &ctx.accounts.better.key(),
                &match_vault.key(),
                principal,
            ),
            &[
                ctx.accounts.better.to_account_info(),
                match_vault.to_account_info(),
            ],
        )?;

        // Transfer fee to platform treasury
        invoke(
            &system_instruction::transfer(
                &ctx.accounts.better.key(),
                &ctx.accounts.platform.key(),
                fee,
            ),
            &[
                ctx.accounts.better.to_account_info(),
                ctx.accounts.platform.to_account_info(),
            ],
        )?;
        Ok(())
    }

    pub fn resolve_match(
        ctx: Context<ResolveMatch>, 
        winner: [u8; 32]
        )->Result<()>{
        let match_account = &mut ctx.accounts.match_account;
        let platform = &mut ctx.accounts.platform;

        require_keys_eq!(platform.authority, ctx.accounts.authority.key(), CricketFiError::UnauthorizedAccess);
        require!(match_account.status == MatchStatus::Active, CricketFiError::InvalidMatchStatus);

        match_account.status = MatchStatus::Completed;
        match_account.winner = Some(winner);
        Ok(())
    }

    pub fn claim_winnings(ctx: Context<ClaimWinnings>, team: u8) -> Result<()> {
        let bet = &mut ctx.accounts.bet_account;
        let match_account = &mut ctx.accounts.match_account;
        let match_vault = &ctx.accounts.match_vault;

        require!(bet.claimed == false, CricketFiError::WinningsAlreadyClaimed);
        require!(match_account.status == MatchStatus::Completed, CricketFiError::InvalidMatchStatus);

        // Validate that the team parameter matches the bet team
        require!(team == bet.team, CricketFiError::InvalidTeam);

        // Verify the claimed team is the winner
        let winner_team = if team == 0 { 
            match_account.team1 
        } else { 
            match_account.team2 
        };
        require!(
            match_account.winner.as_ref().unwrap() == &winner_team,
            CricketFiError::InvalidTeam
        );

        // Calculate winnings using decimal odds (stored as basis points, e.g., 150 = 1.5x)
        // Formula: winnings = (bet_amount * odds_at_bet) / 100
        let winnings = bet
            .amount
            .checked_mul(bet.odds_at_bet)
            .ok_or(ErrorCode::NumericalOverflow)?
            .checked_div(100)
            .ok_or(ErrorCode::NumericalOverflow)?;

        // Ensure vault has sufficient funds
        require!(
            match_vault.lamports() >= winnings,
            CricketFiError::InsufficientVaultFunds
        );

        // Mark bet as claimed before transfer to prevent reentrancy
        bet.claimed = true;

        // Update pool amount - subtract only the excess payout (winnings - principal)
        let excess_payout = winnings.checked_sub(bet.amount).ok_or(ErrorCode::NumericalOverflow)?;
        match_account.total_pool_amount = match_account
            .total_pool_amount
            .checked_sub(excess_payout)
            .ok_or(ErrorCode::NumericalOverflow)?;

        let match_account_key = match_account.key();
        let vault_seeds = &[
            b"match_vault",
            match_account_key.as_ref(),
            &[ctx.bumps.match_vault],
        ];

        // Transfer winnings from vault to the bettor
        invoke_signed(
            &system_instruction::transfer(
                &match_vault.key(),
                &ctx.accounts.better.key(),
                winnings,
            ),
            &[
                match_vault.to_account_info(),
                ctx.accounts.better.to_account_info(),
            ],
            &[vault_seeds],
        )?;

        Ok(())
    }

    pub fn refund_bet(ctx: Context<RefundBet>) -> Result<()> {
        let bet = &mut ctx.accounts.bet_account;
        let match_account = &mut ctx.accounts.match_account;
        let match_vault = &ctx.accounts.match_vault;

        require!(match_account.status == MatchStatus::Cancelled, CricketFiError::MatchNotCancelled);
        require!(!bet.claimed, CricketFiError::WinningsAlreadyClaimed);
        require!(bet.match_id == match_account.match_id, CricketFiError::InvalidMatch);

        bet.claimed = true; //refund claimed

        let match_account_key = match_account.key();
        let vault_seeds = &[
            b"match_vault",
            match_account_key.as_ref(),
            &[ctx.bumps.match_vault],
        ];

        Ok(invoke_signed(
            &system_instruction::transfer(
                &match_vault.key(),
                &ctx.accounts.better.key(),
                bet.amount,
            ),
            &[
                match_vault.to_account_info(),
                ctx.accounts.better.to_account_info(),
            ],
            &[vault_seeds],
        )?)
    }

    // ----------------- ADMIN -----------------

    /// Creates a new match on-chain before betting opens.
    pub fn create_match(
        ctx: Context<CreateMatch>,
        api_match_id: [u8; 32],
        team1: [u8; 32],
        team2: [u8; 32],
        start_ts: i64,
        min_bet: u64,
        max_bet: u64,
        odds_team1: u64,
        odds_team2: u64,
    ) -> Result<()> {
        require!(odds_team1 >= 101 && odds_team1 <= 1000, CricketFiError::InvalidOdds);
        require!(odds_team2 >= 101 && odds_team2 <= 1000, CricketFiError::InvalidOdds);
        let m = &mut ctx.accounts.match_account;
        let p = &mut ctx.accounts.platform;

        m.match_id = api_match_id;
        m.team1 = team1;
        m.team2 = team2;
        m.start_time = start_ts;
        m.end_time = 0;
        m.total_pool_amount = 0;
        m.team1_pool_amount = 0;
        m.team2_pool_amount = 0;
        m.status = MatchStatus::Created;
        m.winner = None;
        m.odds_team1 = odds_team1;
        m.odds_team2 = odds_team2;
        m.min_bet_amount = min_bet;
        m.max_bet_amount = max_bet;

        p.total_matches = p.total_matches.checked_add(1).ok_or(ErrorCode::NumericalOverflow)?;
        Ok(())
    }

    /// Admin can activate a match to allow betting.
    pub fn activate_match(ctx: Context<ActivateMatch>) -> Result<()> {
        let m = &mut ctx.accounts.match_account;
        require!(m.status == MatchStatus::Created, CricketFiError::InvalidMatchStatus);
        m.status = MatchStatus::Active;
        Ok(())
    }

    /// Admin can update odds until the match becomes Active.
    pub fn set_odds(ctx: Context<SetOdds>, odds_team1: u64, odds_team2: u64) -> Result<()> {
        require!(odds_team1 >= 101 && odds_team1 <= 1000, CricketFiError::InvalidOdds);
        require!(odds_team2 >= 101 && odds_team2 <= 1000, CricketFiError::InvalidOdds);
        let m = &mut ctx.accounts.match_account;
        require!(m.status == MatchStatus::Created || m.status == MatchStatus::Active, CricketFiError::InvalidMatchStatus);
        m.odds_team1 = odds_team1;
        m.odds_team2 = odds_team2;
        Ok(())
    }

    /// Admin can cancel a match before it is resolved.
    pub fn cancel_match(ctx: Context<CancelMatch>) -> Result<()> {
        let m = &mut ctx.accounts.match_account;
        require!(m.status == MatchStatus::Created || m.status == MatchStatus::Active, CricketFiError::InvalidMatchStatus);
        m.status = MatchStatus::Cancelled;
        Ok(())
    }

    /// Withdraw accumulated platform fees to the authority wallet.
    pub fn withdraw_treasury(ctx: Context<WithdrawTreasury>, amount: u32) -> Result<()> {
        {
            // limit lifetime of mutable borrow
            let platform = &mut ctx.accounts.platform;
            require_keys_eq!(platform.authority, ctx.accounts.authority.key(), CricketFiError::UnauthorizedAccess);
            require!(platform.treasury_balance >= amount, CricketFiError::InsufficientVaultFunds);

            platform.treasury_balance = platform
                .treasury_balance
                .checked_sub(amount)
                .ok_or(ErrorCode::NumericalOverflow)?;
        }

        // Prepare seeds after mutable borrow has ended
        let auth_key = ctx.accounts.authority.key();
        let seeds: &[&[u8]] = &[
            b"platform",
            auth_key.as_ref(),   // same seeds as PDA init
            &[ctx.bumps.platform],
        ];

        invoke_signed(
            &system_instruction::transfer(
                &ctx.accounts.platform.key(),
                &ctx.accounts.authority.key(),
                amount as u64,
            ),
            &[
                ctx.accounts.platform.to_account_info(),
                ctx.accounts.authority.to_account_info(),
            ],
            &[seeds],
        )?;
        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializePlatform<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = 8 + 32 + 2 + 2 + 4 + 1, // discriminator + authority + total_matches + total_bets + treasury_balance + fee_percentage
        seeds = [b"platform", authority.key().as_ref()],
        bump
    )]
    pub platform: Account<'info, CricketFiContract>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CreateMatch<'info>{
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        constraint = platform.authority == authority.key() @CricketFiError::UnauthorizedAccess
    )]
    pub platform: Account<'info,CricketFiContract>,
    #[account(
        init,
        payer = authority,
        space = 8 + 32 + 32 + 32 + 8 + 8 + 8 + 8 + 8 + 1 + 1 + 32 + 8 + 8 + 8 + 8 // discriminator + match_id + team1 + team2 + start_time + end_time + 3 pool amounts + status + winner option + 2 odds + min/max bet
    )]
    pub match_account: Account<'info,Match>,
    pub system_program: Program<'info,System>,
}

#[derive(Accounts)]
pub struct PlaceBet<'info>{
    #[account(mut)]
    pub better: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", platform.authority.as_ref()],
        bump
    )]
    pub platform: Account<'info,CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info,Match>,
    /// PDA that stores principal for this match
    #[account(
        mut,
        seeds = [b"match_vault", match_account.key().as_ref()],
        bump
    )]
    pub match_vault: SystemAccount<'info>,
    /// PDA to track if user has already bet on this match
    #[account(
        init,
        payer = better,
        space = 8 + 32 + 32 + 1, // discriminator + user + match_id + has_bet
        seeds = [b"user_bet", better.key().as_ref(), match_account.key().as_ref()],
        bump
    )]
    pub user_bet_tracker: Account<'info, UserBetTracker>,
    #[account(
        init,
        payer= better,
        space = 8 + 32 + 32 + 8 + 1 + 8 + 1 + 8 // discriminator + better + match_id + amount + team + bet_time + claimed + odds_at_bet
    )]
    pub bet_account: Account<'info,Bet>,
    pub system_program: Program<'info,System>,
}

#[derive(Accounts)]
pub struct ResolveMatch<'info>{
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", authority.key().as_ref()],
        bump
    )]
    pub platform: Account<'info,CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info,Match>,
    pub system_program: Program<'info,System>,
}

#[derive(Accounts)]
pub struct ClaimWinnings<'info>{
    #[account(mut)]
    pub better: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", platform.authority.as_ref()],
        bump
    )]
    pub platform: Account<'info,CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info, Match>,
    #[account(
        mut,
        seeds = [b"match_vault", match_account.key().as_ref()],
        bump
    )]
    pub match_vault: SystemAccount<'info>,
    #[account(
        mut,
        has_one = better,
        constraint = bet_account.match_id == match_account.match_id @CricketFiError::InvalidMatch
    )]
    pub bet_account: Account<'info, Bet>,
    pub system_program: Program<'info,System>,
}

#[derive(Accounts)]
pub struct RefundBet<'info> {
    #[account(mut)]
    pub better: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", platform.authority.as_ref()],
        bump
    )]
    pub platform: Account<'info, CricketFiContract>,
    #[account(
        mut,
        seeds = [b"match_vault", match_account.key().as_ref()],
        bump
    )]
    pub match_vault: SystemAccount<'info>,
    #[account(
        mut,
        has_one = better,
        constraint = match_account.status == MatchStatus::Cancelled @CricketFiError::MatchNotCancelled,
    )]
    pub bet_account: Account<'info, Bet>,
    #[account(mut)]
    pub match_account: Account<'info, Match>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetOdds<'info>{
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", authority.key().as_ref()],
        bump,
        constraint = platform.authority == authority.key() @CricketFiError::UnauthorizedAccess
    )]
    pub platform: Account<'info, CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info, Match>,
}

#[derive(Accounts)]
pub struct CancelMatch<'info>{
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", authority.key().as_ref()],
        bump,
        constraint = platform.authority == authority.key() @CricketFiError::UnauthorizedAccess
    )]
    pub platform: Account<'info, CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info, Match>,
}

#[derive(Accounts)]
pub struct WithdrawTreasury<'info>{
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", authority.key().as_ref()],
        bump,
        constraint = platform.authority == authority.key() @CricketFiError::UnauthorizedAccess
    )]
    pub platform: Account<'info, CricketFiContract>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct ActivateMatch<'info>{
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        mut,
        seeds = [b"platform", authority.key().as_ref()],
        bump,
        constraint = platform.authority == authority.key() @CricketFiError::UnauthorizedAccess
    )]
    pub platform: Account<'info, CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info, Match>,
}

#[account]
pub struct CricketFiContract {
    pub authority: Pubkey, //Admin of the Project
    pub total_matches: u16, //total matches played on the platform
    pub total_bets: u16, //total bets
    pub treasury_balance: u32, //platform fee accumulated
    pub fee_percentage: u8 //platform fee percentage
}

#[account]

//most efficient type for match strucst
pub struct Match{
    pub match_id: [u8; 32],
    pub team1: [u8; 32],
    pub team2: [u8; 32],
    // pub match_date: i64,
    pub start_time: i64,
    pub end_time: i64,
    pub total_pool_amount: u64,
    pub team1_pool_amount: u64,
    pub team2_pool_amount: u64,
    pub status: MatchStatus,    //Match status (Created/Active/Completed/Cancelled)
    pub winner: Option<[u8;32]>,
    pub odds_team1: u64,        // Odds in basis points (e.g., 150 = 1.5x payout)
    pub odds_team2: u64,        // Odds in basis points (e.g., 200 = 2.0x payout)
    pub min_bet_amount: u64,
    pub max_bet_amount: u64,
}

#[account]
pub struct Bet{
    pub better : Pubkey,
    pub match_id: [u8;32],
    pub amount: u64, //bet amount in lamports
    pub team: u8, //team either 0 or 1
    pub bet_time: i64, //when bet was placed
    pub claimed: bool, //if bet has been claimed or not
    pub odds_at_bet: u64, //odds at the time of placing the bet (basis points, e.g., 150 = 1.5x)
}

#[account]
pub struct UserStats{
    pub total_bets: u64,
    pub total_winnings: u64,
    pub wins: u64,
    pub losses: u64,
}

#[account]
pub struct UserBetTracker {
    pub user: Pubkey,
    pub match_id: [u8;32],
    pub has_bet: bool,
}

#[derive(AnchorSerialize, AnchorDeserialize, Clone, PartialEq)]
pub enum MatchStatus {
    Created,
    Active,
    Completed,
    Cancelled
}

#[error_code]
pub enum CricketFiError{
    #[msg("Unauthorized Access")]
    UnauthorizedAccess,

    #[msg("Invalid Match Status")]
    InvalidMatchStatus,

    #[msg("Invalid Bet Amount")]
    InvalidBetAmount,

    #[msg("Winnings already claimed")]
    WinningsAlreadyClaimed,

    #[msg("Invalid Team")]
    InvalidTeam,

    #[msg("Match not cancelled")]
    MatchNotCancelled,

    #[msg("Invalid Match")]
    InvalidMatch,

    #[msg("Invalid Start Time")]
    InvalidStartTime,

    #[msg("Betting Closed")]
    BettingClosed,

    #[msg("User Already Bet")]
    UserAlreadyBet,

    #[msg("Insufficient Vault Funds")]
    InsufficientVaultFunds,

    #[msg("Invalid Odds")]
    InvalidOdds,
}

/// Additional numerical error for checked math
#[error_code]
pub enum ErrorCode {
    #[msg("Numerical overflow during calculation")]
    NumericalOverflow,
}