use solana_program::{
    account_info::{next_account_info, AccountInfo},
    entrypoint::ProgramResult,
    msg,
    program::{invoke, invoke_signed},
    program_error::ProgramError,
    program_pack::{IsInitialized, Pack},
    pubkey::Pubkey,
    rent::Rent,
    sysvar::Sysvar,
};

use crate::{error::EscrowError, instructions::EscrowInstruction, state::Escrow};

use spl_token::state::Account as TokenAccount;

pub struct Processor;

impl Processor {
    pub fn process(
        program_id: &Pubkey,
        accounts: &[AccountInfo],
        instruction_data: &[u8],
    ) -> ProgramResult {
        let instruction = EscrowInstruction::unpack(instruction_data)?;
        match instruction {
            EscrowInstruction::InitEscrow { amount } => {
                msg!("Instruction: InitEscrow");
                Self::process_init_escrow(accounts, amount, program_id)
            }
            EscrowInstruction::Exchange { amount } => {
                msg!("Instruction: Exchange");
                Self::process_exchange(accounts, amount, program_id)
            }
        }
    }

    fn process_init_escrow(
        accounts: &[AccountInfo],
        amount: u64,
        program_id: &Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        // An account(or person) who first made the escrow (in our example, Alice is an initializer)
        let initializer = next_account_info(account_info_iter)?;

        if !initializer.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        let temp_token_account = next_account_info(account_info_iter)?;
        let token_to_receive_account = next_account_info(account_info_iter)?;
        // Alice's Token Y account should be owned by SPL-Token program
        if *token_to_receive_account.owner != spl_token::id() {
            return Err(ProgramError::IncorrectProgramId);
        }

        let escrow_account = next_account_info(account_info_iter)?;
        // To sustain a 'state data' in our account, we have to pay the 'rent' for the space we are using.
        // Or else, our account will be destroyed.
        // In recent version of Solana-program crate, you don't need to pass an additional account
        // for using sysvar like Rent.
        let rent = &Rent::from_account_info(next_account_info(account_info_iter)?)?;

        // The threshold of balance which is rent-exempt is calculated from the length of data.
        if !rent.is_exempt(escrow_account.lamports(), escrow_account.data_len()) {
            return Err(EscrowError::NotRentExempt.into());
        }

        // since the data in account are just &[u8], we need to deserialize(in other words, unpack) it.
        let mut escrow_info = Escrow::unpack_unchecked(&escrow_account.try_borrow_data()?)?;
        if escrow_info.is_initialized() {
            return Err(ProgramError::AccountAlreadyInitialized);
        }

        escrow_info.is_initialized = true;
        escrow_info.initializer_pubkey = *initializer.key;
        escrow_info.temp_token_account_pubkey = *temp_token_account.key;
        escrow_info.initializer_token_to_receive_account_pubkey = *token_to_receive_account.key;
        escrow_info.expected_amount = amount;

        // This will internally call `pack_into_slice()`
        Escrow::pack(escrow_info, &mut escrow_account.try_borrow_mut_data()?)?;

        // Unlike normal Solana account, PDA account has no private key, because it's not on the elliptic curve.
        // We make it with (program id, seed word)
        let (pda, _bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id);

        let token_program = next_account_info(account_info_iter)?;
        // Make an instruction that changes the ownership from temp token account to PDA
        let owner_change_ix = spl_token::instruction::set_authority(
            token_program.key,      // Tell token program to move authority
            temp_token_account.key, // from temp token account
            Some(&pda),             // to escrow's derived account.
            spl_token::instruction::AuthorityType::AccountOwner,
            initializer.key,     // Alice own's this
            &[&initializer.key], // Alice will sign this
        )?;

        msg!("Calling the token program to transfer account ownership ...");
        // We are using other program(a token program) from our escrow program!
        // This is called 'Cross-Program Invocation'.
        invoke(
            &owner_change_ix,
            &[
                temp_token_account.clone(),
                initializer.clone(),
                token_program.clone(),
            ],
        )?;

        Ok(())
    }

    fn process_exchange(
        accounts: &[AccountInfo],
        amount_expected_by_taker: u64,
        program_id: &Pubkey,
    ) -> ProgramResult {
        let account_info_iter = &mut accounts.iter();
        let taker = next_account_info(account_info_iter)?;

        // This time, Bob is the signer
        if !taker.is_signer {
            return Err(ProgramError::MissingRequiredSignature);
        }

        // Bob's Token Y account
        let takers_sending_token_account = next_account_info(account_info_iter)?;
        // Bob's Token X account
        let takers_token_to_receive_account = next_account_info(account_info_iter)?;
        // Alice's temp Token X account
        let pdas_temp_token_account = next_account_info(account_info_iter)?;
        let pdas_temp_token_account_info =
            TokenAccount::unpack(&pdas_temp_token_account.try_borrow_data()?)?;
        // Recreate PDA with seed word and programId
        let (pda, bump_seed) = Pubkey::find_program_address(&[b"escrow"], program_id);

        // The amount that Alice wants and Bob willing to send should be the same
        if amount_expected_by_taker != pdas_temp_token_account_info.amount {
            return Err(EscrowError::ExpectedAmountMismatch.into());
        }

        // Alice's account
        let initializers_main_account = next_account_info(account_info_iter)?;
        // Alice's Token Y account
        let initializers_token_to_receive_account = next_account_info(account_info_iter)?;
        // Escrow state account
        let escrow_account = next_account_info(account_info_iter)?;

        // Deserialize the escrow data
        let escrow_info = Escrow::unpack(&escrow_account.try_borrow_data()?)?;

        // Check if the temp account address stored in escrow account
        // is same as one we recreated with seed word and programId
        if escrow_info.temp_token_account_pubkey != *pdas_temp_token_account.key {
            return Err(ProgramError::InvalidAccountData);
        }

        // Check if the initializer(Alice) stored in escrow account
        // is same as one Bob is said to be Alice.
        if escrow_info.initializer_pubkey != *initializers_main_account.key {
            return Err(ProgramError::InvalidAccountData);
        }

        // Check if the Token Y account for Alice stored in escrow account
        // is same as one Bob is said to be Alice's Token Y account.
        if escrow_info.initializer_token_to_receive_account_pubkey
            != *initializers_token_to_receive_account.key
        {
            return Err(ProgramError::InvalidAccountData);
        }

        // The token program
        let token_program = next_account_info(account_info_iter)?;

        // Instruction that transfers amount of token to initializer(Alice)
        let transfer_to_initializer_ix = spl_token::instruction::transfer(
            token_program.key,                // Tell token program to transfer Y tokens
            takers_sending_token_account.key, // From Bob's Token Y account
            initializers_token_to_receive_account.key, // To Alice's Token Y account
            taker.key,                        // Authorized by Bob's main account
            &[&taker.key],                    // Signed by Bob's main account
            escrow_info.expected_amount,
        )?;

        msg!("Calling the token program to transfer tokens to escrow's initializer...");

        // invoke token program to execute transfer ix
        invoke(
            &transfer_to_initializer_ix,
            &[
                takers_sending_token_account.clone(),
                initializers_token_to_receive_account.clone(),
                taker.clone(),
                token_program.clone(),
            ],
        )?;

        // Temp Token X account for Alice
        let pda_account = next_account_info(account_info_iter)?;

        //
        let transfer_to_taker_ix = spl_token::instruction::transfer(
            token_program.key,                   // Tell token program to transfer Token X
            pdas_temp_token_account.key,         // From Alice's temp Token X account
            takers_token_to_receive_account.key, // To Bob's Token X account
            &pda,                                // authorized by Temp Token X account
            &[&pda],                             // signed by Temp Token X account
            pdas_temp_token_account_info.amount, // for this amount
        )?;

        msg!("Calling the token program to transfer tokens to the taker...");

        // Since the signer is PDA which has no private key,
        // we have to use `invoke_signed` and give the seed and bump seed.
        invoke_signed(
            &transfer_to_taker_ix,
            &[
                token_program.clone(),
                pdas_temp_token_account.clone(),
                takers_token_to_receive_account.clone(),
                pda_account.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]], // this will be used to recreate the PDA
        )?;

        // Token X's are all sent. We don't need temp Token X account anymore.
        // We should close it.
        let close_pdas_temp_acc_ix = spl_token::instruction::close_account(
            token_program.key,             // tell token program to close
            pdas_temp_token_account.key,   // temp Token X account
            initializers_main_account.key, // And the remaining balance should be sent to Alice
            &pda,                          // authorized by pda
            &[&pda],                       // signed by pda
        )?;

        msg!("Calling the token program to close pda's temp account...");

        // Closing the account requires signing from escrow account
        invoke_signed(
            &close_pdas_temp_acc_ix,
            &[
                token_program.clone(),
                pdas_temp_token_account.clone(),
                takers_token_to_receive_account.clone(),
                pda_account.clone(),
            ],
            &[&[&b"escrow"[..], &[bump_seed]]],
        )?;

        msg!("Closing the escrow account...");

        // A bit of Rust smart pointer knowledge!
        // -------------------------------------
        // How to assign `amount: u64` into `balance: Rc<RefCell<&mut u64>>`?
        // 1. use `borrow_mut()` to unwrap Rc   -> RefMut<&mut u64>
        // 2. then deref RefMut using           -> &mut u64
        // 3. deref one more time!              -> u64
        // So it should be: `**balance.borrow_mut() = amount;`

        // Transfer lamports remaining in escrow's balance to Alice's balance
        **initializers_main_account.lamports.borrow_mut() = initializers_main_account
            .lamports()
            .checked_add(escrow_account.lamports()) // this is cryptographically safe addition!
            .ok_or(EscrowError::AmountOverflow)?; // Option to Result

        // Empty the escrow's balance
        // The Solana runtime will watch accounts will zero balance and delete them.
        **escrow_account.lamports.borrow_mut() = 0;

        // Empty the escrow's data section
        *escrow_account.try_borrow_mut_data()? = &mut [];

        Ok(())
    }
}
