import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { CastleDaoEngine } from "../target/types/castle_dao_engine";
import {
  createMint,
  createAccount,
  mintTo,
  getAccount,
} from "@solana/spl-token";
import { expect } from "chai";
import crypto from "crypto";

describe("CastleDAO Sovereign Realm & Autonomous Content Test Suite", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.CastleDaoEngine as Program<CastleDaoEngine>;
  const connection = provider.connection;

  // Keypairs
  const authority = anchor.web3.Keypair.generate();
  const player = anchor.web3.Keypair.generate();
  const creator = anchor.web3.Keypair.generate();

  let mint: anchor.web3.PublicKey;
  let realmPda: anchor.web3.PublicKey;
  let vaultAuthorityPda: anchor.web3.PublicKey;
  let realmVaultKeypair: anchor.web3.Keypair;

  let playerAta: anchor.web3.PublicKey;
  let creatorAta: anchor.web3.PublicKey;
  let playerStakePda: anchor.web3.PublicKey;

  const INITIAL_TREASURY_SUPPLY = 10_000_000_000_000n; // 10,000 Tokens (6 decimals)
  const PLAYER_INITIAL_DEPOSIT = 1_000_000_000n; // 1,000 Tokens
  const BASE_RATE = new anchor.BN(10_000_000); // 0.01 tokens per sec scaled
  const DEFENSE_RATING = 5000; // 50% defense buff
  const CURATION_THRESHOLD = new anchor.BN(20); // ~400 staked needed: sqrt(400) = 20
  const TIMELOCK_SECONDS = new anchor.BN(2); // Short timelock for test run

  before(async () => {
    // Fund test wallets
    for (const kp of [authority, player, creator]) {
      const sig = await connection.requestAirdrop(
        kp.publicKey,
        5 * anchor.web3.LAMPORTS_PER_SOL
      );
      await connection.confirmTransaction(sig, "confirmed");
    }

    // Create Token Mint
    mint = await createMint(
      connection,
      authority,
      authority.publicKey,
      null,
      6
    );

    // Derive PDAs
    [realmPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("castle_realm"), authority.publicKey.toBuffer()],
      program.programId
    );

    [vaultAuthorityPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [Buffer.from("vault_authority"), realmPda.toBuffer()],
      program.programId
    );

    realmVaultKeypair = anchor.web3.Keypair.generate();

    // User Token Accounts
    playerAta = await createAccount(connection, player, mint, player.publicKey);
    creatorAta = await createAccount(
      connection,
      creator,
      mint,
      creator.publicKey
    );

    // Mint tokens to player
    await mintTo(
      connection,
      authority,
      mint,
      playerAta,
      authority,
      PLAYER_INITIAL_DEPOSIT
    );

    [playerStakePda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("recruit_stake"),
        realmPda.toBuffer(),
        player.publicKey.toBuffer(),
      ],
      program.programId
    );
  });

  it("1. Initializes Castle Realm Citadel and PDA Vault", async () => {
    await program.methods
      .initializeRealm(
        DEFENSE_RATING,
        BASE_RATE,
        CURATION_THRESHOLD,
        TIMELOCK_SECONDS
      )
      .accounts({
        realm: realmPda,
        tokenMint: mint,
        realmVault: realmVaultKeypair.publicKey,
        vaultAuthority: vaultAuthorityPda,
        authority: authority.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .signers([authority, realmVaultKeypair])
      .rpc();

    // Fund Realm Vault Treasury
    await mintTo(
      connection,
      authority,
      mint,
      realmVaultKeypair.publicKey,
      authority,
      INITIAL_TREASURY_SUPPLY
    );

    const realmAccount = await program.account.realm.fetch(realmPda);
    expect(realmAccount.defenseRating).to.equal(DEFENSE_RATING);
    expect(realmAccount.authority.toBase58()).to.equal(
      authority.publicKey.toBase58()
    );
  });

  it("2. Stakes Recruits into Defense Matrix", async () => {
    const stakeAmount = new anchor.BN(400_000_000); // 400 recruits

    await program.methods
      .stakeRecruit(stakeAmount)
      .accounts({
        realm: realmPda,
        recruitStake: playerStakePda,
        realmVault: realmVaultKeypair.publicKey,
        userTokenAccount: playerAta,
        user: player.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .signers([player])
      .rpc();

    const stakeData = await program.account.recruitStake.fetch(playerStakePda);
    expect(stakeData.stakedAmount.toString()).to.equal(stakeAmount.toString());

    const realmData = await program.account.realm.fetch(realmPda);
    expect(realmData.totalStaked.toString()).to.equal(stakeAmount.toString());
  });

  it("3. Accrues and Claims Quest Yield", async () => {
    // Wait 3 seconds to accumulate yield
    await new Promise((resolve) => setTimeout(resolve, 3000));

    const preBalance = (await getAccount(connection, playerAta)).amount;

    await program.methods
      .claimQuestYield()
      .accounts({
        realm: realmPda,
        recruitStake: playerStakePda,
        realmVault: realmVaultKeypair.publicKey,
        vaultAuthority: vaultAuthorityPda,
        userTokenAccount: playerAta,
        user: player.publicKey,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .signers([player])
      .rpc();

    const postBalance = (await getAccount(connection, playerAta)).amount;
    expect(postBalance > preBalance).to.be.true;
  });

  it("4. Submits Community Lore / Content Bounty", async () => {
    const contentPayload = Buffer.from(
      "CastleDAO Chapter 1: The Barons of the Solana Citadel"
    );
    const hash = crypto.createHash("sha256").update(contentPayload).digest();
    const hashArray = Array.from(new Uint8Array(hash));

    const bountyId = new anchor.BN(0);
    const [bountyPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("content_bounty"),
        realmPda.toBuffer(),
        bountyId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    const requestedBounty = new anchor.BN(50_000_000); // 50 Tokens
    const metadataUri = "https://arweave.net/castle_lore_01.json";

    await program.methods
      .submitContent(hashArray, requestedBounty, metadataUri)
      .accounts({
        realm: realmPda,
        contentBounty: bountyPda,
        creator: creator.publicKey,
        systemProgram: anchor.web3.SystemProgram.programId,
      })
      .signers([creator])
      .rpc();

    const bountyData = await program.account.contentBounty.fetch(bountyPda);
    expect(bountyData.id.toNumber()).to.equal(0);
    expect(bountyData.metadataUri).to.equal(metadataUri);
    expect(bountyData.state).to.deep.equal({ pendingReview: {} });
  });

  it("5. Curates Content via Quadratic Staking Votes", async () => {
    const bountyId = new anchor.BN(0);
    const [bountyPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("content_bounty"),
        realmPda.toBuffer(),
        bountyId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    // Player votes to approve content
    await program.methods
      .curateContent(true)
      .accounts({
        realm: realmPda,
        contentBounty: bountyPda,
        recruitStake: playerStakePda,
        curator: player.publicKey,
      })
      .signers([player])
      .rpc();

    const bountyData = await program.account.contentBounty.fetch(bountyPda);
    // sqrt(400,000,000) = 20,000
    expect(bountyData.approvalVotes.toNumber()).to.equal(20000);
  });

  it("6. Finalizes Bounty and Pays Creator Post-Timelock", async () => {
    // Wait for timelock to expire (2 seconds configured)
    await new Promise((resolve) => setTimeout(resolve, 2500));

    const bountyId = new anchor.BN(0);
    const [bountyPda] = anchor.web3.PublicKey.findProgramAddressSync(
      [
        Buffer.from("content_bounty"),
        realmPda.toBuffer(),
        bountyId.toArrayLike(Buffer, "le", 8),
      ],
      program.programId
    );

    const preBalance = (await getAccount(connection, creatorAta)).amount;

    await program.methods
      .finalizeContentBounty()
      .accounts({
        realm: realmPda,
        contentBounty: bountyPda,
        realmVault: realmVaultKeypair.publicKey,
        vaultAuthority: vaultAuthorityPda,
        creatorTokenAccount: creatorAta,
        caller: authority.publicKey,
        tokenProgram: anchor.utils.token.TOKEN_PROGRAM_ID,
      })
      .signers([authority])
      .rpc();

    const postBalance = (await getAccount(connection, creatorAta)).amount;
    const bountyData = await program.account.contentBounty.fetch(bountyPda);

    expect(bountyData.state).to.deep.equal({ accepted: {} });
    expect(postBalance - preBalance).to.equal(50_000_000n);
  });
});