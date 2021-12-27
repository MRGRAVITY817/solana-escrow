import { AccountLayout, Token, TOKEN_PROGRAM_ID } from "@solana/spl-token";
import {
  Account,
  Connection,
  PublicKey,
  SystemProgram,
  SYSVAR_RENT_PUBKEY,
  Transaction,
  TransactionInstruction,
} from "@solana/web3.js";
import BN from "bn.js";
import { ESCROW_ACCOUNT_DATA_LAYOUT, EscrowLayout } from "./layout";

const connection = new Connection("http://localhost:8899", "singleGossip");

export const initEscrow = async (
  privateKeyByteArray: string,
  initializerXTokenAccountPubkeyString: string,
  amountXTokensToSendToEscrow: number,
  initializerReceivingTokenAccountPubkeyString: string,
  expectedAmount: number,
  escrowProgramIdString: string
) => {
  const initializerXTokenAccountPubkey = new PublicKey(
    initializerXTokenAccountPubkeyString
  );

  const parsedData = (
    await connection.getParsedAccountInfo(
      initializerXTokenAccountPubkey,
      "singleGossip"
    )
  ).value!.data;

  const XTokenMintAccountPubkey = new PublicKey(
    //@ts-expect-error
    parsedData.parsed.info.mint
  );

  const privateKeyDecoded = privateKeyByteArray
    .split(",")
    .map((s) => parseInt(s));
  const initializerAccount = new Account(privateKeyDecoded);

  // 1. create empty account owned by token program
  const tempTokenAccount = new Account();
  const createTempTokenAccountIx = SystemProgram.createAccount({
    programId: TOKEN_PROGRAM_ID, // owned by token program
    space: AccountLayout.span, // space allocation for storing data
    lamports: await connection.getMinimumBalanceForRentExemption(
      AccountLayout.span,
      "singleGossip"
    ), // transfer enough balance to avoid rent
    fromPubkey: initializerAccount.publicKey, // Transfer balance from Alice's account
    newAccountPubkey: tempTokenAccount.publicKey, // To new temp X token account
  });
  // 2. initialize temp account to become 'Token program' owned account, which will have several token-related apis
  const initTempAccountIx = Token.createInitAccountInstruction(
    TOKEN_PROGRAM_ID,
    XTokenMintAccountPubkey, // Set the token mint account for this temp X token account (tell which kind of token it uses)
    tempTokenAccount.publicKey, // Tell token program which account to target
    initializerAccount.publicKey // Set the owner of this temp account
  );
  // 3. transfer X tokens from Alice's main X token account to her temp X token account.
  const transferXTokensToTempAccIx = Token.createTransferInstruction(
    TOKEN_PROGRAM_ID, // Tell token program to transfer X token
    initializerXTokenAccountPubkey, // From Alice's main X token account
    tempTokenAccount.publicKey, // To Alice's temp X token account
    initializerAccount.publicKey, // The owner is Alice
    [],
    amountXTokensToSendToEscrow // the amount to transfer
  );

  // 4. create empty account owned by escrow program
  const escrowAccount = new Account();
  const escrowProgramId = new PublicKey(escrowProgramIdString);

  const createEscrowAccountIx = SystemProgram.createAccount({
    space: ESCROW_ACCOUNT_DATA_LAYOUT.span, // space allocation for storing escrow data
    lamports: await connection.getMinimumBalanceForRentExemption(
      // send enough balance to retain the account
      ESCROW_ACCOUNT_DATA_LAYOUT.span,
      "singleGossip"
    ),
    fromPubkey: initializerAccount.publicKey, // send balance from Alice's main account
    newAccountPubkey: escrowAccount.publicKey, // to escrow account
    programId: escrowProgramId, // this account will be owned by our escrow program
  });

  // 5. initialize empty account as escrow state and transfer temporary X token account ownership to PDA
  const initEscrowIx = new TransactionInstruction({
    programId: escrowProgramId,
    keys: [
      // Account 0: Alice, who initializes and signs escrow
      {
        pubkey: initializerAccount.publicKey,
        isSigner: true,
        isWritable: false,
      },
      // Account 1: Temp X token account, which is writable for token related data
      { pubkey: tempTokenAccount.publicKey, isSigner: false, isWritable: true },
      // Account 2: Alice's main Y token account. The amount of token Y will be changed later, but not in this transaction
      {
        pubkey: new PublicKey(initializerReceivingTokenAccountPubkeyString),
        isSigner: false,
        isWritable: false,
      },
      // Account 3: Escrow account, which is writable for escrow data
      { pubkey: escrowAccount.publicKey, isSigner: false, isWritable: true },
      // Account 4: Account for using read-only data from system variables. Unnecessary for current version of Solana.
      { pubkey: SYSVAR_RENT_PUBKEY, isSigner: false, isWritable: false },
      // Account 5: Token program that we use a lot :)
      { pubkey: TOKEN_PROGRAM_ID, isSigner: false, isWritable: false },
    ],
    data: Buffer.from(
      // We put `0` on the first index, to tag this ix to be initEscrow
      // Since the expectedAmount can exceed the limitation of Javascript number, we use BigNum library
      Uint8Array.of(0, ...new BN(expectedAmount).toArray("le", 8))
    ),
  });

  // Almost done! Create new transaction that holds all the ixs' we've defined so far
  // CAUTION! Be sure that it's added in same order of definition, cause the transaction will process ixs in given order.
  const tx = new Transaction().add(
    createTempTokenAccountIx,
    initTempAccountIx,
    transferXTokensToTempAccIx,
    createEscrowAccountIx,
    initEscrowIx
  );
  // Finally, send the transaction to the Solana network
  await connection.sendTransaction(
    tx,
    [initializerAccount, tempTokenAccount, escrowAccount],
    { skipPreflight: false, preflightCommitment: "singleGossip" }
  );

  // Wait for a second for saving changes in Solana blockchain...
  await new Promise((resolve) => setTimeout(resolve, 1000));

  // Test with querying the escrow account's info from the blockchain
  const encodedEscrowState = (await connection.getAccountInfo(
    escrowAccount.publicKey,
    "singleGossip"
  ))!.data;
  // The data should be in Buffer-like format(serialized), so we should decode(deserialize) with our schema(layout)
  const decodedEscrowState = ESCROW_ACCOUNT_DATA_LAYOUT.decode(
    encodedEscrowState
  ) as EscrowLayout;
  // Return the result of our transaction
  return {
    escrowAccountPubkey: escrowAccount.publicKey.toBase58(),
    isInitialized: !!decodedEscrowState.isInitialized,
    initializerAccountPubkey: new PublicKey(
      decodedEscrowState.initializerPubkey
    ).toBase58(),
    XTokenTempAccountPubkey: new PublicKey(
      decodedEscrowState.initializerTempTokenAccountPubkey
    ).toBase58(),
    initializerYTokenAccount: new PublicKey(
      decodedEscrowState.initializerReceivingTokenAccountPubkey
    ).toBase58(),
    expectedAmount: new BN(
      decodedEscrowState.expectedAmount,
      10,
      "le"
    ).toNumber(),
  };
};
