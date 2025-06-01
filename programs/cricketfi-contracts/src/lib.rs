use anchor_lang::prelude::*;
use anchor_lang::solana_program::{system_instruction, program::invoke};

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

    pub fn placebet(ctx: Context<PlaceBet>, match_id: String, amount: u64, team: u8) -> Result<()> {
        let bet_account = &mut ctx.accounts.bet_account;
        let match_account = &mut ctx.accounts.match_account;
        let platform = &mut ctx.accounts.platform;

        // Match status check, min and max bet amount check, team check
        require!(
            match_account.status == MatchStatus::Active,
            CricketFiError::InvalidMatchStatus
        );
        require!(
            amount >= match_account.min_bet_amount && amount <= match_account.max_bet_amount,
            CricketFiError::InvalidBetAmount
        );
        require!(team == 0 || team == 1, CricketFiError::InvalidTeam);

        // Populate bet account
        bet_account.better = ctx.accounts.better.key();
        bet_account.match_id = match_id;
        bet_account.amount = amount;
        bet_account.team = team;
        bet_account.bet_time = Clock::get()?.unix_timestamp;
        bet_account.claimed = false;
        bet_account.odds_at_bet = match_account.odds_team1;
        
        // Updating match pool
        if team == 0 {
            match_account.team1_pool_amount += amount;
        } else {
            match_account.team2_pool_amount += amount;
        }
        match_account.total_pool_amount += amount;
        platform.total_bets += 1;
        platform.treasury_balance += amount * platform.fee_percentage / 100;

        Ok(())
    }

    pub fn resolve_match(
        ctx: Context<ResolveMatch>, 
        // match_id: String, 
        winner: String
        )->Result<()>{
        let match_account = &mut ctx.accounts.match_account;
        let platform = &mut ctx.accounts.platform;

        require_keys_eq!(platform.authority, ctx.accounts.authority.key(), CricketFiError::UnauthorizedAccess);
        require!(match_account.status == MatchStatus::Active, CricketFiError::InvalidMatchStatus);

        match_account.status = MatchStatus::Completed;
        match_account.winner = Some(winner);
        Ok(())
    }

    pub fn claim_winnings(ctx: Context<ClaimWinnings>, 
        // match_id: String, 
        team: u8)->
        Result<()>
        {
        let bet = &mut ctx.accounts.bet_account;
        let match_account = &mut ctx.accounts.match_account;
        let platform = &mut ctx.accounts.platform;

        require!(bet.claimed == false, CricketFiError::WinningsAlreadyClaimed);
        require!(match_account.status == MatchStatus::Completed, CricketFiError::InvalidMatchStatus);

        let winning_team = if team==0{
            match_account.team1_pool_amount
        } else{
            match_account.team2_pool_amount
        };

        let winnings = match_account.total_pool_amount * match_account.odds_team1 / winning_team;
        let platform_fee = winnings * platform.fee_percentage / 100;
        let winnings_to_claim = winnings - platform_fee;
        //update bet account
        bet.claimed= true;
        //updating match account
        match_account.total_pool_amount -= winnings;
        //updating platform accounts
        platform.treasury_balance+= platform_fee;
        //transfering claimed winnings to the better
        invoke(
            &system_instruction::transfer(
                &ctx.accounts.platform.key(),
                &ctx.accounts.better.key(),
                winnings_to_claim,
            ),
            &[
                ctx.accounts.platform.to_account_info(),
                ctx.accounts.better.to_account_info(),
            ],
        )?;        Ok(())
    }
}

#[derive(Accounts)]
pub struct InitializePlatform<'info> {
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(
        init,
        payer = authority,
        space = 8 + 32 + 8 + 8 + 8 + 8
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
        space = 8 + 32 + 32 + 32 + 32 + 32 + 32 + 8 + 8 + 8 + 1 + 32 + 8 + 8 + 8 + 8
    )]
    pub match_account: Account<'info,Match>,
    pub system_program: Program<'info,System>,
}

#[derive(Accounts)]
pub struct PlaceBet<'info>{
    #[account(mut)]
    pub better: Signer<'info>,
    #[account(mut)]
    pub platform: Account<'info,CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info,Match>,
    #[account(
        init,
        payer= better,
        space = 8 + 32 + 32 + 8 + 1 + 8 + 1 + 8
    )]
    pub bet_account: Account<'info,Bet>,
    pub system_program: Program<'info,System>,
}

#[derive(Accounts)]
pub struct ResolveMatch<'info>{
    #[account(mut)]
    pub authority: Signer<'info>,
    #[account(mut)]
    pub platform: Account<'info,CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info,Match>,
    pub system_program: Program<'info,System>,
}

#[derive(Accounts)]
pub struct ClaimWinnings<'info>{
    #[account(mut)]
    pub better: Signer<'info>,
    #[account(mut)]
    pub platform: Account<'info,CricketFiContract>,
    #[account(mut)]
    pub match_account: Account<'info, Match>,
    #[account(mut)]
    pub bet_account: Account<'info,Bet>,
    pub system_program: Program<'info,System>,
}

#[account]
pub struct CricketFiContract {
    pub authority: Pubkey, //Admin of the Project
    pub total_matches: u64, //total matches played on the platform
    pub total_bets: u64, //total bets
    pub treasury_balance: u64, //platform fee accumulated
    pub fee_percentage: u64 //platform fee percentage
}

#[account]
pub struct Match{
    pub match_id: String,
    pub team1: String,
    pub team2: String,
    pub match_date: String,
    pub start_time: String,
    pub end_time: String,
    pub total_pool_amount: u64,
    pub team1_pool_amount: u64,
    pub team2_pool_amount: u64,
    pub status: MatchStatus,    //Match status (Created/Active/Completed/Cancelled)
    pub winner: Option<String>,
    pub odds_team1: u64,
    pub odds_team2: u64,
    pub min_bet_amount: u64,
    pub max_bet_amount: u64,
}

#[account]
pub struct Bet{
    pub better : Pubkey,
    pub match_id: String,
    pub amount: u64, //bet amount in lamports
    pub team: u8, //team either 0 or 1
    pub bet_time: i64, //when bet was placed
    pub claimed: bool, //if bet has been claimed or not
    pub odds_at_bet: u64, //odds at the time of placing the bet
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
}