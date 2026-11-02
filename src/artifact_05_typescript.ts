import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { CastledaoContent } from "../target/types/castledao_content";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { assert, expect } from "chai";

describe("castledao_content", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.CastledaoContent as Program<CastledaoContent>;

  const authority = anchor.web3.Keypair.generate();
  const creator = anchor.web3.Keypair.generate();
  const challenger = anchor.web3.Keypair.generate();

  let mint: anchor.web3.Pubkey;
  let realmPda: anchor.web3.Pubkey;
  let vaultPda: anchor.web3.Pubkey;
  let creatorProfilePda: anchor.web3.Pubkey;

  let creatorAta: anchor.web3.Pubkey;
  let challengerAta: anchor.web3.Pubkey;

  const realmId = new anchor.BN(42);
  const minBond = new anchor.BN(50 * 1e6); // 50 tokens
  const disputeWindow = new anchor.BN(3600); // 1 hour

  before(async () => {
    // Fund test wallets
    const airdropSigners = [authority, creator, challenger];
    for (const signer of airdropSigners) {
      const sig = await provider.connection.requestAirdrop(
        signer.publicKey,
        5 * anchor.web3.LAMPORTS_PER_SOL
      );
      await provider.connection.confirmTransaction(sig);
    }

    // Create Mint
    mint = await createMint(
      provider.connection,
      authority,
      authority.publicKey,
      null,
      6
    );

    // Derive PDAs
    [realmPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [Buffer.from("realm"), realmId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [vaultPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [Buffer.from("vault"), realmPda.toBuffer()],
      program.programId
    );

    [creatorProfilePda] = anchor.web3.Pubkey.findProgramAddressSync(
      [Buffer.from("profile"), creator.publicKey.toBuffer()],
      program.programId
    );

    // Setup Token Accounts
    creatorAta = await createAccount(
      provider.connection,
      creator,
      mint,
      creator.publicKey
    );

    challengerAta = await createAccount(
      provider.connection,
      challenger,
      mint,
      challenger.publicKey
    );

    // Mint test capital
    await mintTo(provider.connection, authority, mint, creatorAta, authority, 1_000 * 1e6);
    await mintTo(provider.connection, authority, mint, challengerAta, authority, 1_000 * 1e6);
  });

  it("1. Initializes CastleDAO Realm and Token Vault", async () => {
    const tx = await program.methods
      .initializeRealm(realmId, minBond, disputeWindow)
      .accounts({
        realm: realmPda,
        vault: vaultPda,
        mint: mint,
        authority: authority.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([authority])
      .rpc();

    const realmAccount = await program.account.realm.fetch(realmPda);
    assert.ok(realmAccount.authority.equals(authority.publicKey));
    assert.equal(realmAccount.minSubmissionBond.toNumber(), minBond.toNumber());
  });

  it("2. Registers Creator Profile and sets initial Recruit rank", async () => {
    await program.methods
      .initializeCreatorProfile()
      .accounts({
        creatorProfile: creatorProfilePda,
        creator: creator.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const profile = await program.account.creatorProfile.fetch(creatorProfilePda);
    assert.ok(profile.creator.equals(creator.publicKey));
    assert.equal(profile.experiencePoints.toNumber(), 0);
    assert.deepEqual(profile.rankTier, { recruit: {} });
  });

  it("3. Submits Content with bonded stake", async () => {
    const submissionId = new anchor.BN(1);
    const [submissionPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [
        Buffer.from("submission"),
        realmPda.toBuffer(),
        submissionId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    // Seed Treasury with reward liquidity
    await mintTo(provider.connection, authority, mint, vaultPda, authority, 500 * 1e6);

    const cidBuffer = Buffer.alloc(64);
    cidBuffer.write("QmCastleDAOExclusiveContentMetaHash1234567890abcdefghijklm");
    const cidArray = Array.from(cidBuffer);

    const requestedBounty = new anchor.BN(100 * 1e6);

    await program.methods
      .submitContent(cidArray, submissionId, requestedBounty)
      .accounts({
        realm: realmPda,
        submission: submissionPda,
        vault: vaultPda,
        creatorTokenAccount: creatorAta,
        creator: creator.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([creator])
      .rpc();

    const submission = await program.account.submission.fetch(submissionPda);
    assert.deepEqual(submission.status, { pending: {} });
    assert.equal(submission.bondAmount.toNumber(), minBond.toNumber());

    const vaultAccount = await getAccount(provider.connection, vaultPda);
    expect(Number(vaultAccount.amount)).to.be.at.least(minBond.toNumber());
  });

  it("4. Prevents early resolution before dispute window closes", async () => {
    const submissionId = new anchor.BN(1);
    const [submissionPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [
        Buffer.from("submission"),
        realmPda.toBuffer(),
        submissionId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    try {
      await program.methods
        .resolveUnchallenged()
        .accounts({
          realm: realmPda,
          submission: submissionPda,
          creatorProfile: creatorProfilePda,
          vault: vaultPda,
          creatorTokenAccount: creatorAta,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();
      assert.fail("Should have failed due to open dispute window");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("DisputeWindowStillOpen");
    }
  });

  it("5. Submits and challenges second submission", async () => {
    const submissionId = new anchor.BN(2);
    const [submissionPda] = anchor.web3.Pubkey.findProgramAddressSync(
      [
        Buffer.from("submission"),
        realmPda.toBuffer(),
        submissionId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    const [disputePda] = anchor.web3.Pubkey.findProgramAddressSync(
      [Buffer.from("dispute"), submissionPda.toBuffer()],
      program.programId
    );

    const dummyCid = Array.from(Buffer.alloc(64, 1));
    await program.methods
      .submitContent(dummyCid, submissionId, new anchor.BN(50 * 1e6))
      .accounts({
        realm: realmPda,
        submission: submissionPda,
        vault: vaultPda,
        creatorTokenAccount: creatorAta,
        creator: creator.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([creator])
      .rpc();

    const evidenceHash = Array.from(Buffer.alloc(32, 7));

    await program.methods
      .challengeSubmission(evidenceHash)
      .accounts({
        realm: realmPda,
        submission: submissionPda,
        dispute: disputePda,
        vault: vaultPda,
        challengerTokenAccount: challengerAta,
        challenger: challenger.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([challenger])
      .rpc();

    const sub = await program.account.submission.fetch(submissionPda);
    assert.deepEqual(sub.status, { challenged: {} });

    // Council Settle Dispute (Favor Challenger -> Slashing Creator Bond)
    await program.methods
      .settleDispute(false)
      .accounts({
        realm: realmPda,
        submission: submissionPda,
        dispute: disputePda,
        vault: vaultPda,
        creatorTokenAccount: creatorAta,
        challengerTokenAccount: challengerAta,
        authority: authority.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();

    const resolvedSub = await program.account.submission.fetch(submissionPda);
    assert.deepEqual(resolvedSub.status, { rejected: {} });
  });
});