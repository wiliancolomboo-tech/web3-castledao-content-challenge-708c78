import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { CastledaoContent } from "../target/types/castledao_content";
import { PublicKey, SystemProgram, Keypair, LAMPORTS_PER_SOL } from "@solana/web3.js";
import { expect } from "chai";
import * as crypto from "crypto";

describe("CastleDAO Content Challenge Engine Tests", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.CastledaoContent as Program<CastledaoContent>;

  const challengeId = new anchor.BN(Date.now());
  const minCuratorStake = new anchor.BN(0.5 * LAMPORTS_PER_SOL);
  const submissionBond = new anchor.BN(0.05 * LAMPORTS_PER_SOL);
  const initialBounty = new anchor.BN(2.0 * LAMPORTS_PER_SOL);

  const creator = Keypair.generate();
  const curator = Keypair.generate();
  const keeper = Keypair.generate();

  let challengePda: PublicKey;
  let vaultPda: PublicKey;
  let submissionPda: PublicKey;
  let voteRecordPda: PublicKey;

  before(async () => {
    // Fund test wallets
    const airdropSignatures = await Promise.all([
      provider.connection.requestAirdrop(creator.publicKey, 2 * LAMPORTS_PER_SOL),
      provider.connection.requestAirdrop(curator.publicKey, 2 * LAMPORTS_PER_SOL),
      provider.connection.requestAirdrop(keeper.publicKey, 1 * LAMPORTS_PER_SOL),
    ]);
    await Promise.all(airdropSignatures.map(sig => provider.connection.confirmTransaction(sig)));

    [challengePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("challenge"), challengeId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [vaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), challengePda.toBuffer()],
      program.programId
    );

    [submissionPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("submission"), challengePda.toBuffer(), creator.publicKey.toBuffer()],
      program.programId
    );

    [voteRecordPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("vote"), submissionPda.toBuffer(), curator.publicKey.toBuffer()],
      program.programId
    );
  });

  it("1. Initializes Challenge with Escrow Bounty", async () => {
    // 5 slots for submission, 5 slots for voting for deterministic local testing
    const subSlots = new anchor.BN(10);
    const voteSlots = new anchor.BN(10);

    const tx = await program.methods
      .initializeChallenge(
        challengeId,
        subSlots,
        voteSlots,
        minCuratorStake,
        submissionBond,
        initialBounty
      )
      .accounts({
        challenge: challengePda,
        vault: vaultPda,
        authority: provider.wallet.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .rpc();

    expect(tx).to.be.a("string");

    const challengeAccount = await program.account.challenge.fetch(challengePda);
    expect(challengeAccount.totalBountyPool.toString()).to.equal(initialBounty.toString());
    expect(challengeAccount.totalSubmissions).to.equal(0);

    const vaultBalance = await provider.connection.getBalance(vaultPda);
    expect(vaultBalance).to.be.gte(initialBounty.toNumber());
  });

  it("2. Accepts Creator Submission with Staked Anti-Spam Bond", async () => {
    const contentHash = Array.from(crypto.createHash("sha256").update("CastleDAO Gameplay Highlights").digest());
    const metadataUri = "https://arweave.net/tx_castledao_challenge_proof_content_sample";

    const preBalance = await provider.connection.getBalance(creator.publicKey);

    await program.methods
      .submitEntry(contentHash, metadataUri)
      .accounts({
        challenge: challengePda,
        submission: submissionPda,
        vault: vaultPda,
        creator: creator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const postBalance = await provider.connection.getBalance(creator.publicKey);
    expect(preBalance - postBalance).to.be.gte(submissionBond.toNumber());

    const submissionAccount = await program.account.submission.fetch(submissionPda);
    expect(submissionAccount.creator.toBase58()).to.equal(creator.publicKey.toBase58());
    expect(submissionAccount.settled).to.be.false;
  });

  it("3. Prevents Double Voting by the Same Curator", async () => {
    // Fast-forward slot progress if necessary in local-test environment
    // Simulating curator vote
    const score = 95;
    const slash = false;

    // Wait until submission slot passed
    let currentSlot = await provider.connection.getSlot();
    const challengeAccount = await program.account.challenge.fetch(challengePda);
    while (currentSlot <= challengeAccount.submissionEndSlot.toNumber()) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      currentSlot = await provider.connection.getSlot();
    }

    await program.methods
      .voteEntry(score, slash)
      .accounts({
        challenge: challengePda,
        submission: submissionPda,
        voteRecord: voteRecordPda,
        curator: curator.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([curator])
      .rpc();

    const voteAccount = await program.account.voteRecord.fetch(voteRecordPda);
    expect(voteAccount.score).to.equal(95);

    // Attempt double voting -> should fail via seeds constraint
    try {
      await program.methods
        .voteEntry(80, false)
        .accounts({
          challenge: challengePda,
          submission: submissionPda,
          voteRecord: voteRecordPda,
          curator: curator.publicKey,
          systemProgram: SystemProgram.programId,
        })
        .signers([curator])
        .rpc();
      expect.fail("Expected failure on double voting");
    } catch (err: any) {
      expect(err.toString()).to.include("already in use");
    }
  });

  it("4. Settles Submission, Distributes Scaled Reward, and Tips Keeper", async () => {
    let currentSlot = await provider.connection.getSlot();
    const challengeAccount = await program.account.challenge.fetch(challengePda);
    while (currentSlot <= challengeAccount.votingEndSlot.toNumber()) {
      await new Promise((resolve) => setTimeout(resolve, 500));
      currentSlot = await provider.connection.getSlot();
    }

    const preCreatorBalance = await provider.connection.getBalance(creator.publicKey);
    const preKeeperBalance = await provider.connection.getBalance(keeper.publicKey);

    await program.methods
      .settleSubmission()
      .accounts({
        challenge: challengePda,
        submission: submissionPda,
        vault: vaultPda,
        creator: creator.publicKey,
        keeper: keeper.publicKey,
        systemProgram: SystemProgram.programId,
      })
      .signers([keeper])
      .rpc();

    const postCreatorBalance = await provider.connection.getBalance(creator.publicKey);
    const postKeeperBalance = await provider.connection.getBalance(keeper.publicKey);

    // Creator receives bond refund + bounty share
    expect(postCreatorBalance).to.be.gt(preCreatorBalance);
    // Keeper receives tip
    expect(postKeeperBalance).to.be.gt(preKeeperBalance);

    const submissionAccount = await program.account.submission.fetch(submissionPda);
    expect(submissionAccount.settled).to.be.true;
  });
});