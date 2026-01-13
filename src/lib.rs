pub mod error;

use solana_program_entrypoint::entrypoint;
use solana_account_info::AccountInfo;
use solana_program_error::ProgramResult;
use solana_pubkey::Pubkey;
use solana_msg::msg;

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8]
) -> ProgramResult{
    msg!("CLMM MVP: Processing instruction");
    //ToDo
}