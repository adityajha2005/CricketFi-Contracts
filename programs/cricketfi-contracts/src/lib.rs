use anchor_lang::prelude::*;

declare_id!("6F9BXG9UXFix5yJ2sZfsbXAg7KcrT3oW2YxjorWik1Bo");

#[program]
pub mod cricketfi_contracts {
    use super::*;

    pub fn initialize(ctx: Context<InitializePlatform>) -> Result<()> {
        msg!("Greetings from: {:?}", ctx.program_id);
        Ok(())
    }
    pub fn placebet(ctx:Context<PlaceBet>,match_id:String,amount:u64,team:u8) -> Result<()>{
        let bet_account = &mut ctx.accounts.bet_account;
        let match_account = &mut ctx.accounts.match_account;
        let platform = &mut ctx.accounts.platform;

        //match status check, min and max bet amount check, team check
        require!(match_account.status == MatchStatus::Active,CricketFiError::InvalidMatchStatus)
        require!(amount>=match_account.min_bet_amount && amount<= match_account.max_bet_amount, 
        CricketFiError::InvalidBetAmount)
        require!(team==0 || team==1,CricketFiError::InvalidTeam)
        //populate bet amount
        bet.better = better.key();
        bet.match_id = match_id;
        bet.amount = amount;
        bet.team = team;
        bet.bet_time = Clock::get()?.unix_timestamp;
        bet.claimed = false;
        bet.odds_at_bet = match_account.odds_team1;
        
        //updating match pool
        if team==0{
            match_account.team1_pool_amount += amount;
        }
        else{
            match_account.team2_pool_amount += amount;
        }
        platform.total_bets += 1;
        platform.treasury_balance += amount * platform.fee_percentage / 100;

        Ok(())
    }
}

#[derive(Accounts)]
pub fn initialize(ctx: Context<InitializePlatform>) -> Result<()> {
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