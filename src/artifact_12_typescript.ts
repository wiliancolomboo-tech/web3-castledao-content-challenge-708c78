import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { Castledao } from "../target/types/castledao";
import {
  Keypair,
  PublicKey,
  SystemProgram,
  SYSVAR_INSTRUCTIONS_PUBKEY,
  Ed25519Program,
  Transaction,
  sendAndConfirmTransaction,
} from "@solana/web3.js";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { expect } from "chai";
import * as nacl from "tweetnacl";

describe("CastleDAO Fortress Engine Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.Castledao as Program<Castledao>;

  let treasuryMint: PublicKey;
  let authority: Keypair;
  let warrior: Keypair;
  let creator: Keypair;
  let oracleSigner: Keypair;

  let warriorAta: PublicKey;
  let creatorAta: PublicKey;

  const guildId = new anchor.BN(42);
  const minStake = new anchor.BN(100_000_000); // 100 tokens

  let fortressPda: PublicKey;
  let fortressBump: number;
  let fortressVaultPda: PublicKey;

  before(async () => {
    authority = Keypair.generate();
    warrior = Keypair.generate();
    creator = Keypair.generate();
    oracleSigner = Keypair.generate();

    // Airdrop SOL
    for (const kp of [authority, warrior, creator]) {
      const sig = await provider.connection.requestAirdrop(kp.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL);
      const latestBlockhash = await provider.connection.getLatestBlockhash();
      await provider.connection.confirmTransaction({
        signature: sig,
        ...latestBlockhash,
      });
    }

    // Initialize Mint
    treasuryMint = await createMint(
      provider.connection,
      authority,
      authority.publicKey,
      null,
      6
    );

    // Derive PDAs
    [fortressPda, fortressBump] = PublicKey.findProgramAddressSync(
      [Buffer.from("fortress"), guildId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [fortressVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), fortressPda.toBuffer()],
      program.programId
    );

    // ATAs
    warriorAta = await createAccount(provider.connection, warrior, treasuryMint, warrior.publicKey);
    creatorAta = await createAccount(provider.connection, creator, treasuryMint, creator.publicKey);

    // Fund Accounts
    await mintTo(provider.connection, authority, treasuryMint, warriorAta, authority, 5_000_000_000);
  });

  it("1. Initializes Castle Fortress PDA and Collateral Vault", async () => {
    const slashBps = 500; // 5%

    await program.methods
      .initializeFortress(guildId, minStake, slashBps)
      .accounts({
        authority: authority.publicKey,
        treasuryMint: treasuryMint,
        fortressState: fortressPda,
        fortressVault: fortressVaultPda,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([authority])
      .rpc();

    const state = await program.account.fortressState.fetch(fortressPda);
    expect(state.guildId.toString()).to.equal(guildId.toString());
    expect(state.healthPoints).to.equal(10000);
    expect(state.totalStaked.toNumber()).to.equal(0);
  });

  it("2. Warrior Stakes into Fortress to Boost Fortifications", async () => {
    const stakeAmount = new anchor.BN(200_000_000);
    const [warriorStakePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("warrior"), fortressPda.toBuffer(), warrior.publicKey.toBuffer()],
      program.programId
    );

    await program.methods
      .stakeWarrior(stakeAmount, 150)
      .accounts({
        warrior: warrior.publicKey,
        fortressState: fortressPda,
        fortressVault: fortressVaultPda,
        warriorTokenAccount: warriorAta,
        warriorStake: warriorStakePda,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: SystemProgram.programId,
      })
      .signers([warrior])
      .rpc();

    const vaultAccount = await getAccount(provider.connection, fortressVaultPda);
    expect(vaultAccount.amount.toString()).to.equal(stakeAmount.toString());

    const fortress = await program.account.fortressState.fetch(fortressPda);
    expect(fortress.totalStaked.toString()).to.equal(stakeAmount.toString());
  });

  it("3. Verifies Content Submission with Atomic Ed25519 Introspection", async () => {
    const challengeId = new anchor.BN(1001);
    const rewardAmount = new anchor.BN(50_000_000);
    const contentMessage = Buffer.from("CastleDAO-Content-Submission-Hash-0xDeadBeef");
    const contentHash = anchor.utils.sha256.hash(contentMessage);
    const contentHashBytes = Buffer.from(contentHash, "hex");

    // Off-chain Ed25519 Signature by Adjudicator Oracle
    const sig = nacl.sign.detached(contentHashBytes, oracleSigner.secretKey);

    const [bountyRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("bounty"), fortressPda.toBuffer(), challengeId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    // Precompile instruction
    const ed25519Instruction = Ed25519Program.createInstructionWithPublicKey({
      publicKey: oracleSigner.publicKey.toBytes(),
      message: contentHashBytes,
      signature: sig,
    });

    const verifyIx = await program.methods
      .verifyContentChallenge(challengeId, rewardAmount, Array.from(contentHashBytes), Array.from(sig))
      .accounts({
        creator: creator.publicKey,
        fortressState: fortressPda,
        bountyRecord: bountyRecordPda,
        instructionsSysvar: SYSVAR_INSTRUCTIONS_PUBKEY,
        systemProgram: SystemProgram.programId,
      })
      .instruction();

    const tx = new Transaction().add(ed25519Instruction).add(verifyIx);
    await sendAndConfirmTransaction(provider.connection, tx, [creator]);

    const bounty = await program.account.bountyRecord.fetch(bountyRecordPda);
    expect(bounty.rewardAmount.toString()).to.equal(rewardAmount.toString());
    expect(bounty.status).to.deep.equal({ approved: {} });
  });

  it("4. Releases Escrowed Bounty to Content Creator", async () => {
    const challengeId = new anchor.BN(1001);
    const [bountyRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("bounty"), fortressPda.toBuffer(), challengeId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    const initialBal = (await getAccount(provider.connection, creatorAta)).amount;

    await program.methods
      .claimBounty()
      .accounts({
        creator: creator.publicKey,
        fortressState: fortressPda,
        fortressVault: fortressVaultPda,
        bountyRecord: bountyRecordPda,
        creatorTokenAccount: creatorAta,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([creator])
      .rpc();

    const updatedBal = (await getAccount(provider.connection, creatorAta)).amount;
    expect((updatedBal - initialBal).toString()).to.equal("50000000");

    const bounty = await program.account.bountyRecord.fetch(bountyRecordPda);
    expect(bounty.status).to.deep.equal({ claimed: {} });
  });

  it("5. Rejects Double-Claim Exploits on Claimed Bounty", async () => {
    const challengeId = new anchor.BN(1001);
    const [bountyRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("bounty"), fortressPda.toBuffer(), challengeId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    try {
      await program.methods
        .claimBounty()
        .accounts({
          creator: creator.publicKey,
          fortressState: fortressPda,
          fortressVault: fortressVaultPda,
          bountyRecord: bountyRecordPda,
          creatorTokenAccount: creatorAta,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([creator])
        .rpc();
      expect.fail("Double-claim exploit should have failed.");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("InvalidBountyStatus");
    }
  });

  it("6. Executes Autonomous Fortress Decay Crank", async () => {
    await program.methods
      .decayFortress()
      .accounts({
        cranker: warrior.publicKey,
        fortressState: fortressPda,
      })
      .signers([warrior])
      .rpc();

    const fortress = await program.account.fortressState.fetch(fortressPda);
    expect(fortress.healthPoints).to.be.at.most(10000);
  });
});