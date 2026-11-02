import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import { CastledaoEngine } from "../target/types/castledao_engine";
import {
  TOKEN_PROGRAM_ID,
  getOrCreateAssociatedTokenAccount,
  getAccount,
} from "@solana/spl-token";
import { assert, expect } from "chai";
import { PublicKey, Keypair, SystemProgram, SYSVAR_CLOCK_PUBKEY } from "@solana/web3.js";

describe("CastleDAO Solana Engine Full Test Matrix", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.CastledaoEngine as Program<CastledaoEngine>;
  const admin = (provider.wallet as anchor.Wallet).payer;

  const lord = Keypair.generate();
  const attacker = Keypair.generate();

  const castleId = new anchor.BN(42);
  const baseRate = new anchor.BN(10_000); // 10,000 micro-units/sec
  const siegeDuration = new anchor.BN(2); // 2 seconds for test speed

  let realmConfigPda: PublicKey;
  let rubyMintPda: PublicKey;
  let castlePda: PublicKey;
  let siegePda: PublicKey;
  let escrowVaultPda: PublicKey;
  let lordTokenAccount: PublicKey;

  before(async () => {
    // Fund test wallets
    const airdrop1 = await provider.connection.requestAirdrop(lord.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL);
    const airdrop2 = await provider.connection.requestAirdrop(attacker.publicKey, 10 * anchor.web3.LAMPORTS_PER_SOL);
    await provider.connection.confirmTransaction(airdrop1);
    await provider.connection.confirmTransaction(airdrop2);

    [realmConfigPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("realm_config")],
      program.programId
    );

    [rubyMintPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("ruby_mint")],
      program.programId
    );

    [castlePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("castle"), castleId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [siegePda] = PublicKey.findProgramAddressSync(
      [Buffer.from("siege"), castleId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    [escrowVaultPda] = PublicKey.findProgramAddressSync(
      [Buffer.from("escrow"), castleId.toArrayLike(Buffer, "le", 8)],
      program.programId
    );
  });

  it("1. Initializes Realm with Dynamic Production Parameters and Mint PDA", async () => {
    await program.methods
      .initializeRealm(baseRate, siegeDuration)
      .accounts({
        realmConfig: realmConfigPda,
        rubyMint: rubyMintPda,
        admin: admin.publicKey,
        systemProgram: SystemProgram.programId,
        tokenProgram: TOKEN_PROGRAM_ID,
        rent: anchor.web3.SYSVAR_RENT_PUBKEY,
      })
      .rpc();

    const config = await program.account.realmConfig.fetch(realmConfigPda);
    assert.equal(config.admin.toBase58(), admin.publicKey.toBase58());
    assert.equal(config.baseProductionRate.toNumber(), 10_000);
  });

  it("2. Lord claims a sovereign Castle territory", async () => {
    await program.methods
      .claimCastle(castleId, new anchor.BN(2000))
      .accounts({
        castle: castlePda,
        lord: lord.publicKey,
        systemProgram: SystemProgram.programId,
        clock: SYSVAR_CLOCK_PUBKEY,
      })
      .signers([lord])
      .rpc();

    const castle = await program.account.castle.fetch(castlePda);
    assert.equal(castle.lord.toBase58(), lord.publicKey.toBase58());
    assert.equal(castle.level, 1);
    assert.equal(castle.defensePower.toNumber(), 2000);
    assert.isFalse(castle.underSiege);
  });

  it("3. Upgrades fortress defense rating and level", async () => {
    await program.methods
      .upgradeFortress(castleId)
      .accounts({
        castle: castlePda,
        lord: lord.publicKey,
      })
      .signers([lord])
      .rpc();

    const castle = await program.account.castle.fetch(castlePda);
    assert.equal(castle.level, 2);
    assert.equal(castle.defensePower.toNumber(), 2500);
  });

  it("4. Accrues and harvests $RUBY yield via differential clock calculation", async () => {
    // Create ATA for lord
    const ata = await getOrCreateAssociatedTokenAccount(
      provider.connection,
      lord,
      rubyMintPda,
      lord.publicKey
    );
    lordTokenAccount = ata.address;

    // Sleep 1.5 seconds to accumulate time delta
    await new Promise((resolve) => setTimeout(resolve, 1500));

    await program.methods
      .harvestResources(castleId)
      .accounts({
        realmConfig: realmConfigPda,
        castle: castlePda,
        rubyMint: rubyMintPda,
        userTokenAccount: lordTokenAccount,
        lord: lord.publicKey,
        tokenProgram: TOKEN_PROGRAM_ID,
        clock: SYSVAR_CLOCK_PUBKEY,
      })
      .signers([lord])
      .rpc();

    const tokenBal = await getAccount(provider.connection, lordTokenAccount);
    expect(Number(tokenBal.amount)).to.be.greaterThan(0);
  });

  it("5. Rejects unauthorized lord harvest attempt", async () => {
    try {
      await program.methods
        .harvestResources(castleId)
        .accounts({
          realmConfig: realmConfigPda,
          castle: castlePda,
          rubyMint: rubyMintPda,
          userTokenAccount: lordTokenAccount,
          lord: attacker.publicKey,
          tokenProgram: TOKEN_PROGRAM_ID,
          clock: SYSVAR_CLOCK_PUBKEY,
        })
        .signers([attacker])
        .rpc();
      assert.fail("Expected harvest to fail with unauthorized lord");
    } catch (err: any) {
      assert.include(err.toString(), "UnauthorizedLord");
    }
  });
});