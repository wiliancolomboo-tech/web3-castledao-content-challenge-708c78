import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { CastleDao } from "../target/types/castle_dao";
import {
  TOKEN_PROGRAM_ID,
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { expect } from "chai";
import * as crypto from "crypto";

describe("CastleDAO Core Protocol QA Test Suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.CastleDao as Program<CastleDao>;
  const connection = provider.connection;

  // Keypairs
  const authority = anchor.web3.Keypair.generate();
  const lord = anchor.web3.Keypair.generate();
  const contender = anchor.web3.Keypair.generate();

  let mint: anchor.web3.PublicKey;
  let lordTokenAccount: anchor.web3.PublicKey;
  let contenderTokenAccount: anchor.web3.PublicKey;

  const realmName = "Avalon-Alpha";
  const castleId = new anchor.BN(101);
  const challengeId = new anchor.BN(5001);

  let realmPda: anchor.web3.PublicKey;
  let castlePda: anchor.web3.PublicKey;
  let castleVaultPda: anchor.web3.PublicKey;
  let challengePda: anchor.web3.PublicKey;
  let entryPda: anchor.web3.PublicKey;

  before(async () => {
    // AirDrop SOL to all entities
    for (const kp of [authority, lord, contender]) {
      const sig = await connection.requestAirdrop(
        kp.publicKey,
        10 * anchor.web3.LAMPORTS_PER_SOL
      );
      const latestBlockhash = await connection.getLatestBlockhash();
      await connection.confirmTransaction({
        signature: sig,
        ...latestBlockhash,
      });
    }

    // Initialize Mint
    mint = await createMint(
      connection,
      authority,
      authority.publicKey,
      null,
      6
    );

    // Create Token Accounts
    lordTokenAccount = await createAccount(
      connection,
      lord,
      mint,
      lord.publicKey
    );
    contenderTokenAccount = await createAccount(
      connection,
      contender,
      mint,
      contender.publicKey
    );

    // Fund Lord with 10,000 SPL tokens
    await mintTo(
      connection,
      authority,
      mint,
      lordTokenAccount,
      authority,
      10_000_000_000
    );

    // Derive PDAs
    [realmPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("realm"), Buffer.from(realmName)],
      program.programId
    );

    [castlePda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("castle"),
        realmPda.toBuffer(),
        castleId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    [castleVaultPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault"), castlePda.toBuffer()],
      program.programId
    );

    [challengePda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("challenge"),
        castlePda.toBuffer(),
        challengeId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    [entryPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("entry"),
        challengePda.toBuffer(),
        contender.publicKey.toBuffer(),
      ],
      program.programId
    );
  });

  it("1. Initializes Realm Domain successfully", async () => {
    const feeBps = 250; // 2.5%
    await program.methods
      .initializeRealm(realmName, feeBps)
      .accountsStrict({
        realm: realmPda,
        mint: mint,
        authority: authority.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([authority])
      .rpc();

    const realmAccount = await program.account.realmConfig.fetch(realmPda);
    expect(realmAccount.realmName).to.equal(realmName);
    expect(realmAccount.authority.toBase58()).to.equal(
      authority.publicKey.toBase58()
    );
    expect(realmAccount.castleCount.toNumber()).to.equal(0);
    expect(realmAccount.protocolFeeBps).to.equal(feeBps);
  });

  it("2. Deploys Castle territory and associated Vault PDA", async () => {
    const defenseCapacity = new anchor.BN(5_000_000_000); // 5,000 tokens

    await program.methods
      .createCastle(castleId, defenseCapacity)
      .accountsStrict({
        realm: realmPda,
        castle: castlePda,
        castleVault: castleVaultPda,
        mint: mint,
        lord: lord.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([lord])
      .rpc();

    const castle = await program.account.castleAccount.fetch(castlePda);
    expect(castle.lord.toBase58()).to.equal(lord.publicKey.toBase58());
    expect(castle.castleId.toNumber()).to.equal(castleId.toNumber());
    expect(castle.defensePower.toNumber()).to.equal(0);
    expect(castle.vault.toBase58()).to.equal(castleVaultPda.toBase58());

    const realm = await program.account.realmConfig.fetch(realmPda);
    expect(realm.castleCount.toNumber()).to.equal(1);
  });

  it("3. Stakes tokens into Defense Vault", async () => {
    const stakeAmount = new anchor.BN(1_500_000_000); // 1,500 tokens

    await program.methods
      .stakeDefense(stakeAmount)
      .accountsStrict({
        castle: castlePda,
        castleVault: castleVaultPda,
        lordTokenAccount: lordTokenAccount,
        lord: lord.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([lord])
      .rpc();

    const vaultTokenAccount = await getAccount(connection, castleVaultPda);
    expect(Number(vaultTokenAccount.amount)).to.equal(stakeAmount.toNumber());

    const castle = await program.account.castleAccount.fetch(castlePda);
    expect(castle.defensePower.toNumber()).to.equal(stakeAmount.toNumber());
  });

  it("4. Launches Content Challenge with escrowed bounty", async () => {
    const bountyAmount = new anchor.BN(500_000_000); // 500 tokens
    const durationSlots = new anchor.BN(1000);
    const targetScoreThreshold = 85;

    await program.methods
      .launchChallenge(
        challengeId,
        bountyAmount,
        durationSlots,
        targetScoreThreshold
      )
      .accountsStrict({
        castle: castlePda,
        challenge: challengePda,
        castleVault: castleVaultPda,
        creatorTokenAccount: lordTokenAccount,
        creator: lord.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([lord])
      .rpc();

    const challenge = await program.account.challengeAccount.fetch(challengePda);
    expect(challenge.bountyAmount.toNumber()).to.equal(bountyAmount.toNumber());
    expect(challenge.settled).to.be.false;
    expect(challenge.targetScoreThreshold).to.equal(targetScoreThreshold);
  });

  it("5. Submits Content Entry with 32-byte cryptographic digest", async () => {
    // Generate deterministic sha256 of IPFS CID "QmTestCastleDaoMetaverseContentPayload"
    const rawContent = "ipfs://QmTestCastleDaoMetaverseContentPayload";
    const contentHashBuffer = crypto
      .createHash("sha256")
      .update(rawContent)
      .digest();
    const contentHash = Array.from(contentHashBuffer);

    await program.methods
      .submitContentEntry(contentHash)
      .accountsStrict({
        challenge: challengePda,
        entry: entryPda,
        contender: contender.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([contender])
      .rpc();

    const entry = await program.account.entryAccount.fetch(entryPda);
    expect(entry.contender.toBase58()).to.equal(contender.publicKey.toBase58());
    expect(entry.evaluated).to.be.false;
    expect(Buffer.from(entry.contentHash).toString("hex")).to.equal(
      contentHashBuffer.toString("hex")
    );
  });

  it("6. Settles Challenge and delivers PDA reward payout", async () => {
    const passingScore = 95;

    await program.methods
      .settleAndDistribute(passingScore)
      .accountsStrict({
        castle: castlePda,
        challenge: challengePda,
        entry: entryPda,
        castleVault: castleVaultPda,
        contenderTokenAccount: contenderTokenAccount,
        verifier: lord.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .signers([lord])
      .rpc();

    const entry = await program.account.entryAccount.fetch(entryPda);
    expect(entry.evaluated).to.be.true;
    expect(entry.score).to.equal(passingScore);

    const challenge = await program.account.challengeAccount.fetch(challengePda);
    expect(challenge.settled).to.be.true;
    expect(challenge.winner.toBase58()).to.equal(
      contender.publicKey.toBase58()
    );

    const contenderAta = await getAccount(connection, contenderTokenAccount);
    expect(Number(contenderAta.amount)).to.equal(500_000_000);
  });

  it("7. Fails gracefully when settling below target score threshold", async () => {
    const failingChallengeId = new anchor.BN(5002);
    const [failChallengePda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("challenge"),
        castlePda.toBuffer(),
        failingChallengeId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );
    const [failEntryPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("entry"),
        failChallengePda.toBuffer(),
        contender.publicKey.toBuffer(),
      ],
      program.programId
    );

    // Launch second challenge with threshold = 90
    await program.methods
      .launchChallenge(
        failingChallengeId,
        new anchor.BN(100_000_000),
        new anchor.BN(500),
        90
      )
      .accountsStrict({
        castle: castlePda,
        challenge: failChallengePda,
        castleVault: castleVaultPda,
        creatorTokenAccount: lordTokenAccount,
        creator: lord.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([lord])
      .rpc();

    const dummyHash = Array.from(Buffer.alloc(32, 7));
    await program.methods
      .submitContentEntry(dummyHash)
      .accountsStrict({
        challenge: failChallengePda,
        entry: failEntryPda,
        contender: contender.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([contender])
      .rpc();

    // Settle with score 70 (below 90) -> expect failure
    try {
      await program.methods
        .settleAndDistribute(70)
        .accountsStrict({
          castle: castlePda,
          challenge: failChallengePda,
          entry: failEntryPda,
          castleVault: castleVaultPda,
          contenderTokenAccount: contenderTokenAccount,
          verifier: lord.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .signers([lord])
        .rpc();
      expect.fail("Settlement should have failed on ScoreBelowThreshold");
    } catch (err: any) {
      expect(err.error.errorCode.code).to.equal("ScoreBelowThreshold");
    }
  });
});