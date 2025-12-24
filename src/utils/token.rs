use solana_client::nonblocking::rpc_client::RpcClient;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::program_pack::Pack;
use spl_token::state::Account as TokenAccount;
use spl_token::state::Mint;
use crate::error::{Result, AppError};

pub async fn get_token_balance(rpc_client: &RpcClient, wallet: &Pubkey, mint: &Pubkey) -> Result<u64> {
    // Derive ATA
    let ata_address = spl_associated_token_account::get_associated_token_address(wallet, mint);

    // Fetch Account
    match rpc_client.get_account(&ata_address).await {
        Ok(account) => {
            // Check if it's initialized and correct
            let token_account = TokenAccount::unpack(&account.data)
                .map_err(|e| AppError::Parse(format!("Failed to unpack token account: {}", e)))?;

            Ok(token_account.amount)
        },
        Err(_) => {
            // If account lookup fails (not found), check balance via helper or assume 0
            // get_token_account_balance returns UiTokenAmount

            match rpc_client.get_token_account_balance(&ata_address).await {
                Ok(balance) => {
                    balance.amount.parse::<u64>()
                        .map_err(|e| AppError::Parse(format!("Invalid balance amount: {}", e)))
                },
                Err(_) => {
                    // Assuming not found means 0 balance
                    Ok(0)
                }
            }
        }
    }
}

pub async fn get_decimals(rpc_client: &RpcClient, mint: &Pubkey) -> Result<u8> {
    let account = rpc_client.get_account(mint).await
        .map_err(|e| AppError::Rpc(format!("Failed to fetch mint: {}", e)))?;

    let mint_data = Mint::unpack(&account.data)
        .map_err(|e| AppError::Parse(format!("Failed to unpack mint: {}", e)))?;

    Ok(mint_data.decimals)
}
