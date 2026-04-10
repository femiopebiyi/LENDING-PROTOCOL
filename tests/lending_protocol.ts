import * as anchor from "@coral-xyz/anchor";
import { Program, BN } from "@coral-xyz/anchor";
import { LendingProtocol } from "../target/types/lending_protocol";
import { PublicKey, Keypair } from "@solana/web3.js";
import {
  createMint,
  createAssociatedTokenAccount,
  getOrCreateAssociatedTokenAccount,
  mintTo,
  getAccount,
  TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { assert } from "chai";

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

async function getTokenBalance(
  provider: anchor.AnchorProvider,
  ata: PublicKey
): Promise<bigint> {
  const account = await getAccount(provider.connection, ata);
  return account.amount;
}

function randomSeed(): BN {
  return new BN(Math.floor(Math.random() * 1_000_000_000));
}

async function assertFails(promise: Promise<unknown>, substring: string): Promise<void> {
  try {
    await promise;
    assert.fail(`Expected failure with "${substring}" but succeeded`);
  } catch (err: any) {
    if (err?.message?.includes("assert.fail")) throw err;
    const msg = err?.message ?? err?.toString() ?? "";
    assert.include(msg, substring, `Expected "${substring}" in: ${msg}`);
  }
}

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

// Pool risk parameters
const LIQUIDATION_THRESHOLD = new BN(80);  // 80%
const LIQUIDATION_BONUS = new BN(5);   // 5%
const INTEREST_RATE = new BN(500); // 5% annual
const MAX_LTV = new BN(75);  // 75%

// Oracle prices — scaled by PRICE_SCALE = 1_000_000
// $100 per collateral token
const INITIAL_PRICE = new BN(100_000_000);
// $60 per collateral token — triggers liquidation
const CRASHED_PRICE = new BN(60_000_000);

// ─────────────────────────────────────────────────────────────────────────────
// Test suite
// ─────────────────────────────────────────────────────────────────────────────

describe("lending_protocol", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.LendingProtocol as Program<LendingProtocol>;
  const wallet = provider.wallet as anchor.Wallet;

  // shared mints — created once
  let collateralMint: PublicKey;
  let borrowMint: PublicKey;

  // shared user token accounts
  let userAtaCollateral: PublicKey;
  let userAtaBorrow: PublicKey;

  before("create mints and fund user", async () => {
    collateralMint = await createMint(
      provider.connection, wallet.payer, wallet.publicKey, null, 6
    );
    borrowMint = await createMint(
      provider.connection, wallet.payer, wallet.publicKey, null, 6
    );

    userAtaCollateral = await createAssociatedTokenAccount(
      provider.connection, wallet.payer, collateralMint, wallet.publicKey
    );
    userAtaBorrow = await createAssociatedTokenAccount(
      provider.connection, wallet.payer, borrowMint, wallet.publicKey
    );

    // fund user with collateral to deposit
    await mintTo(
      provider.connection, wallet.payer, collateralMint,
      userAtaCollateral, wallet.payer, 100_000_000
    );
  });

  // ── Shared helpers ────────────────────────────────────────────────────────

  async function setupOracle(seed: BN, price: BN) {
    const [oracle] = PublicKey.findProgramAddressSync(
      [Buffer.from("stuboracle"), wallet.publicKey.toBuffer(), seed.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    await program.methods
      .initializeOracle(seed, price)
      .accountsPartial({
        owner: wallet.publicKey,
        oracle,
      })
      .rpc();

    return oracle;
  }

  async function setupPool(seed: BN, oracle: PublicKey) {
    const [pool] = PublicKey.findProgramAddressSync(
      [Buffer.from("lendingpool"), wallet.publicKey.toBuffer(), seed.toArrayLike(Buffer, "le", 8)],
      program.programId
    );

    await program.methods
      .initializePool(
        seed,
        LIQUIDATION_THRESHOLD,
        LIQUIDATION_BONUS,
        INTEREST_RATE,
        MAX_LTV,
      )
      .accountsPartial({
        owner: wallet.publicKey,
        pool,
        collateralMint,
        borrowMint,
        oracle,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();

    const collateralVault = (await getOrCreateAssociatedTokenAccount(
      provider.connection, wallet.payer, collateralMint, pool, true
    )).address;

    const borrowVault = (await getOrCreateAssociatedTokenAccount(
      provider.connection, wallet.payer, borrowMint, pool, true
    )).address;

    return { pool, collateralVault, borrowVault };
  }

  async function getUserPosition(pool: PublicKey, user: PublicKey): Promise<PublicKey> {
    const [userPosition] = PublicKey.findProgramAddressSync(
      [Buffer.from("userposition"), pool.toBuffer(), user.toBuffer()],
      program.programId
    );
    return userPosition;
  }

  async function depositCollateral(pool: PublicKey, collateralVault: PublicKey, amount: BN) {
    const userPosition = await getUserPosition(pool, wallet.publicKey);
    await program.methods
      .depositCollateral(amount)
      .accountsPartial({
        depositor: wallet.publicKey,
        pool,
        collateralMint,
        collateralVault,
        userAtaCollateral,
        userPosition,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
  }

  async function borrowTokens(pool: PublicKey, borrowVault: PublicKey, oracle: PublicKey, amount: BN) {
    const userPosition = await getUserPosition(pool, wallet.publicKey);
    await program.methods
      .borrow(amount)
      .accountsPartial({
        borrower: wallet.publicKey,
        pool,
        borrowMint,
        borrowVault,
        userAtaBorrow,
        userPosition,
        oracle,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
  }

  async function repayTokens(pool: PublicKey, borrowVault: PublicKey, amount: BN) {
    const userPosition = await getUserPosition(pool, wallet.publicKey);
    await program.methods
      .repay(amount)
      .accountsPartial({
        repayer: wallet.publicKey,
        pool,
        borrowMint,
        borrowVault,
        userAtaBorrow,
        userPosition,
        tokenProgram: TOKEN_PROGRAM_ID,
      })
      .rpc();
  }

  async function setPrice(oracle: PublicKey) {
    await program.methods
      .setOraclePrice()
      .accountsPartial({
        setter: wallet.publicKey,
        oracle,
      })
      .rpc();
  }

  // ─────────────────────────────────────────────────────────────────────────
  // Oracle tests
  // ─────────────────────────────────────────────────────────────────────────
  describe("oracle", () => {
    let oracle: PublicKey;
    const seed = randomSeed();

    before(async () => {
      oracle = await setupOracle(seed, INITIAL_PRICE);
    });

    it("initializes oracle with correct price", async () => {
      const state = await program.account.stubOracle.fetch(oracle);
      assert.equal(state.price.toString(), INITIAL_PRICE.toString());
      assert.equal(state.authority.toBase58(), wallet.publicKey.toBase58());
    });

    it("updates oracle price correctly", async () => {
      const newPrice = new BN(150_000_000);
      await setPrice(oracle);
      const state = await program.account.stubOracle.fetch(oracle);
      assert.equal(state.price.toString(), newPrice.toString());
    });

    it("rejects zero price", async () => {
      await assertFails(
        setPrice(oracle),
        "InvalidAmount"
      );
    });

    it("rejects price update from wrong authority", async () => {
      const attacker = Keypair.generate();
      await assertFails(
        program.methods
          .setOraclePrice()
          .accountsPartial({
            setter: attacker.publicKey,
            oracle,
          })
          .signers([attacker])
          .rpc(),
        "CredentialMismatch"
      );
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // Initialize pool tests
  // ─────────────────────────────────────────────────────────────────────────
  describe("initialize_pool", () => {
    let oracle: PublicKey;
    let pool: PublicKey;
    const seed = randomSeed();

    before(async () => {
      oracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const result = await setupPool(seed, oracle);
      pool = result.pool;
    });

    it("creates pool with correct state", async () => {
      const state = await program.account.lendingPool.fetch(pool);
      assert.equal(state.owner.toBase58(), wallet.publicKey.toBase58());
      assert.equal(state.collateralMint.toBase58(), collateralMint.toBase58());
      assert.equal(state.borrowMint.toBase58(), borrowMint.toBase58());
      assert.equal(state.liquidationThreshold.toString(), LIQUIDATION_THRESHOLD.toString());
      assert.equal(state.liquidationBonus.toString(), LIQUIDATION_BONUS.toString());
      assert.equal(state.interestRate.toString(), INTEREST_RATE.toString());
      assert.equal(state.maxLtv.toString(), MAX_LTV.toString());
      assert.equal(state.totalCollateral.toString(), "0");
      assert.equal(state.totalBorrowed.toString(), "0");
    });

    it("rejects same mint for collateral and borrow", async () => {
      const badSeed = randomSeed();
      const [badPool] = PublicKey.findProgramAddressSync(
        [Buffer.from("lendingpool"), wallet.publicKey.toBuffer(), badSeed.toArrayLike(Buffer, "le", 8)],
        program.programId
      );
      await assertFails(
        program.methods
          .initializePool(badSeed, LIQUIDATION_THRESHOLD, LIQUIDATION_BONUS, INTEREST_RATE, MAX_LTV)
          .accountsPartial({
            owner: wallet.publicKey,
            pool: badPool,
            collateralMint,
            borrowMint: collateralMint, // same as collateral
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc(),
        "InvalidParameter"
      );
    });

    it("rejects max_ltv >= liquidation_threshold", async () => {
      const badSeed = randomSeed();
      const [badPool] = PublicKey.findProgramAddressSync(
        [Buffer.from("lendingpool"), wallet.publicKey.toBuffer(), badSeed.toArrayLike(Buffer, "le", 8)],
        program.programId
      );
      await assertFails(
        program.methods
          .initializePool(badSeed, new BN(80), LIQUIDATION_BONUS, INTEREST_RATE, new BN(80))
          .accountsPartial({
            owner: wallet.publicKey,
            pool: badPool,
            collateralMint,
            borrowMint,
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc(),
        "InvalidParameter"
      );
    });

    it("rejects zero interest rate", async () => {
      const badSeed = randomSeed();
      const [badPool] = PublicKey.findProgramAddressSync(
        [Buffer.from("lendingpool"), wallet.publicKey.toBuffer(), badSeed.toArrayLike(Buffer, "le", 8)],
        program.programId
      );
      await assertFails(
        program.methods
          .initializePool(badSeed, LIQUIDATION_THRESHOLD, LIQUIDATION_BONUS, new BN(0), MAX_LTV)
          .accountsPartial({
            owner: wallet.publicKey,
            pool: badPool,
            collateralMint,
            borrowMint,
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc(),
        "InvalidParameter"
      );
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // Deposit collateral tests
  // ─────────────────────────────────────────────────────────────────────────
  describe("deposit_collateral", () => {
    let oracle: PublicKey;
    let pool: PublicKey;
    let collateralVault: PublicKey;
    let userPosition: PublicKey;
    const seed = randomSeed();

    before(async () => {
      oracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const result = await setupPool(seed, oracle);
      pool = result.pool;
      collateralVault = result.collateralVault;
      userPosition = await getUserPosition(pool, wallet.publicKey);
    });

    it("first deposit initializes position correctly", async () => {
      const amount = new BN(10_000_000);
      await depositCollateral(pool, collateralVault, amount);

      const position = await program.account.userPosition.fetch(userPosition);
      assert.equal(position.owner.toBase58(), wallet.publicKey.toBase58());
      assert.equal(position.pool.toBase58(), pool.toBase58());
      assert.equal(position.collateralDeposited.toString(), amount.toString());
      assert.equal(position.borrowedAmount.toString(), "0");
      assert.isTrue(position.isOpen);
    });

    it("subsequent deposit accumulates collateral", async () => {
      const additionalAmount = new BN(5_000_000);
      const positionBefore = await program.account.userPosition.fetch(userPosition);
      const before = positionBefore.collateralDeposited;

      await depositCollateral(pool, collateralVault, additionalAmount);

      const positionAfter = await program.account.userPosition.fetch(userPosition);
      assert.equal(
        positionAfter.collateralDeposited.toString(),
        before.add(additionalAmount).toString()
      );
    });

    it("increments pool total_collateral", async () => {
      const poolBefore = await program.account.lendingPool.fetch(pool);
      const amount = new BN(1_000_000);

      await depositCollateral(pool, collateralVault, amount);

      const poolAfter = await program.account.lendingPool.fetch(pool);
      assert.equal(
        poolAfter.totalCollateral.sub(poolBefore.totalCollateral).toString(),
        amount.toString()
      );
    });

    it("rejects zero amount", async () => {
      await assertFails(
        depositCollateral(pool, collateralVault, new BN(0)),
        "InvalidAmount"
      );
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // Borrow tests
  // ─────────────────────────────────────────────────────────────────────────
  describe("borrow", () => {
    let oracle: PublicKey;
    let pool: PublicKey;
    let collateralVault: PublicKey;
    let borrowVault: PublicKey;
    let userPosition: PublicKey;
    const seed = randomSeed();
    const depositAmount = new BN(10_000_000); // 10 tokens at $100 = $1000 collateral

    before(async () => {
      oracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const result = await setupPool(seed, oracle);
      pool = result.pool;
      collateralVault = result.collateralVault;
      borrowVault = result.borrowVault;
      userPosition = await getUserPosition(pool, wallet.publicKey);

      // fund the borrow vault so there is liquidity to lend
      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        borrowVault, wallet.payer, 100_000_000
      );

      // deposit collateral
      await depositCollateral(pool, collateralVault, depositAmount);
    });

    it("1. borrow up to LTV limit succeeds", async () => {
      // collateral = 10 tokens * $100 = $1000
      // max LTV = 75% → max borrow = $750
      // borrow 700 tokens (at $1 each assuming 1:1 for simplicity)
      // Note: adjust amount based on your borrow token price assumptions
      const borrowAmount = new BN(7_000_000); // 70% LTV — within limit

      const userBalBefore = await getTokenBalance(provider, userAtaBorrow);

      await borrowTokens(pool, borrowVault, oracle, borrowAmount);

      const userBalAfter = await getTokenBalance(provider, userAtaBorrow);
      assert.equal(userBalAfter - userBalBefore, BigInt(borrowAmount.toNumber()));

      const position = await program.account.userPosition.fetch(userPosition);
      assert.equal(position.borrowedAmount.toString(), borrowAmount.toString());
    });

    it("2. borrow beyond LTV limit fails", async () => {
      // already borrowed 7_000_000, trying to borrow 2_000_000 more
      // total would be 9_000_000 = 90% LTV which exceeds 75% max
      await assertFails(
        borrowTokens(pool, borrowVault, oracle, new BN(2_000_000)),
        "ExceedsMaxLtv"
      );
    });

    it("rejects borrow with no collateral", async () => {
      const emptyOracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const emptySeed = randomSeed();
      const emptyResult = await setupPool(emptySeed, emptyOracle);

      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        emptyResult.borrowVault, wallet.payer, 10_000_000
      );

      const emptyPosition = await getUserPosition(emptyResult.pool, wallet.publicKey);

      // initialize empty position first
      await depositCollateral(emptyResult.pool, emptyResult.collateralVault, new BN(1_000_000));

      // withdraw it all — now position has no collateral
      const posState = await program.account.userPosition.fetch(emptyPosition);
      await program.methods
        .withdrawCollateral(posState.collateralDeposited)
        .accountsPartial({
          withdrawer: wallet.publicKey,
          pool: emptyResult.pool,
          collateralMint,
          collateralVault: emptyResult.collateralVault,
          userAtaCollateral,
          userPosition: emptyPosition,
          oracle: emptyOracle,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      await assertFails(
        borrowTokens(emptyResult.pool, emptyResult.borrowVault, emptyOracle, new BN(100_000)),
        "InsufficientCollateral"
      );
    });

    it("6. wrong oracle passed to borrow fails", async () => {
      const wrongOracle = await setupOracle(randomSeed(), new BN(200_000_000));
      await assertFails(
        borrowTokens(pool, borrowVault, wrongOracle, new BN(100_000)),
        "ConstraintSeeds"
      );
    });

    it("rejects zero borrow amount", async () => {
      await assertFails(
        borrowTokens(pool, borrowVault, oracle, new BN(0)),
        "InvalidAmount"
      );
    });

    it("rejects borrow when vault has insufficient liquidity", async () => {
      const dryOracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const drySeed = randomSeed();
      const dryResult = await setupPool(drySeed, dryOracle);

      // deposit collateral but do NOT fund the borrow vault
      await depositCollateral(dryResult.pool, dryResult.collateralVault, new BN(10_000_000));

      await assertFails(
        borrowTokens(dryResult.pool, dryResult.borrowVault, dryOracle, new BN(1_000_000)),
        "InsufficientLiquidity"
      );
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // Repay tests
  // ─────────────────────────────────────────────────────────────────────────
  describe("repay", () => {
    let oracle: PublicKey;
    let pool: PublicKey;
    let collateralVault: PublicKey;
    let borrowVault: PublicKey;
    let userPosition: PublicKey;
    const seed = randomSeed();
    const depositAmount = new BN(10_000_000);
    const borrowAmount = new BN(5_000_000);

    before(async () => {
      oracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const result = await setupPool(seed, oracle);
      pool = result.pool;
      collateralVault = result.collateralVault;
      borrowVault = result.borrowVault;
      userPosition = await getUserPosition(pool, wallet.publicKey);

      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        borrowVault, wallet.payer, 100_000_000
      );

      // fund user borrow ATA so they can repay
      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        userAtaBorrow, wallet.payer, 50_000_000
      );

      await depositCollateral(pool, collateralVault, depositAmount);
      await borrowTokens(pool, borrowVault, oracle, borrowAmount);
    });

    it("partial repay reduces borrowed amount", async () => {
      const repayAmount = new BN(2_000_000);
      const positionBefore = await program.account.userPosition.fetch(userPosition);
      const debtBefore = positionBefore.borrowedAmount;

      await repayTokens(pool, borrowVault, repayAmount);

      const positionAfter = await program.account.userPosition.fetch(userPosition);
      assert.isTrue(
        positionAfter.borrowedAmount.lt(debtBefore),
        "borrowed amount should decrease after repay"
      );
    });

    it("7. full repay closes position correctly", async () => {
      // repay entire remaining debt
      const position = await program.account.userPosition.fetch(userPosition);
      const totalDebt = position.borrowedAmount.add(position.interestAccrued);
      const repayAmount = totalDebt.addn(1_000); // slight buffer for any accrued interest

      await repayTokens(pool, borrowVault, repayAmount);

      const positionAfter = await program.account.userPosition.fetch(userPosition);
      assert.equal(positionAfter.borrowedAmount.toString(), "0");
      assert.equal(positionAfter.interestAccrued.toString(), "0");
    });

    it("rejects repay with nothing owed", async () => {
      // position is already fully repaid from previous test
      await assertFails(
        repayTokens(pool, borrowVault, new BN(1_000_000)),
        "InsufficientBorrow"
      );
    });

    it("rejects zero repay amount", async () => {
      await assertFails(
        repayTokens(pool, borrowVault, new BN(0)),
        "InvalidAmount"
      );
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // Withdraw collateral tests
  // ─────────────────────────────────────────────────────────────────────────
  describe("withdraw_collateral", () => {
    let oracle: PublicKey;
    let pool: PublicKey;
    let collateralVault: PublicKey;
    let borrowVault: PublicKey;
    let userPosition: PublicKey;
    const seed = randomSeed();
    const depositAmount = new BN(10_000_000);
    const borrowAmount = new BN(5_000_000);

    before(async () => {
      oracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const result = await setupPool(seed, oracle);
      pool = result.pool;
      collateralVault = result.collateralVault;
      borrowVault = result.borrowVault;
      userPosition = await getUserPosition(pool, wallet.publicKey);

      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        borrowVault, wallet.payer, 100_000_000
      );
      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        userAtaBorrow, wallet.payer, 50_000_000
      );

      await depositCollateral(pool, collateralVault, depositAmount);
      await borrowTokens(pool, borrowVault, oracle, borrowAmount);
    });

    it("3. withdraw collateral with active debt that breaks health fails", async () => {
      // deposited 10_000_000 at $100 = $1000 collateral
      // borrowed  5_000_000 at max ltv 75%
      // withdrawing 8_000_000 would leave $200 collateral vs $500 debt — unhealthy
      await assertFails(
        program.methods
          .withdrawCollateral(new BN(8_000_000))
          .accountsPartial({
            withdrawer: wallet.publicKey,
            pool,
            collateralMint,
            collateralVault,
            userAtaCollateral,
            userPosition,
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc(),
        "InsufficientCollateral"
      );
    });

    it("partial withdrawal that keeps health factor safe succeeds", async () => {
      // withdrawing 1_000_000 leaves 9_000_000 collateral
      // health = (9_000_000 * $100 * 80%) / 5_000_000 = 1.44 — safe
      const withdrawAmount = new BN(1_000_000);
      const userBalBefore = await getTokenBalance(provider, userAtaCollateral);
      const positionBefore = await program.account.userPosition.fetch(userPosition);

      await program.methods
        .withdrawCollateral(withdrawAmount)
        .accountsPartial({
          withdrawer: wallet.publicKey,
          pool,
          collateralMint,
          collateralVault,
          userAtaCollateral,
          userPosition,
          oracle,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      const userBalAfter = await getTokenBalance(provider, userAtaCollateral);
      const positionAfter = await program.account.userPosition.fetch(userPosition);

      assert.equal(userBalAfter - userBalBefore, BigInt(withdrawAmount.toNumber()));
      assert.equal(
        positionBefore.collateralDeposited.sub(positionAfter.collateralDeposited).toString(),
        withdrawAmount.toString()
      );
    });

    it("full withdrawal after full repay closes position", async () => {
      // repay all debt first
      const position = await program.account.userPosition.fetch(userPosition);
      const totalDebt = position.borrowedAmount.add(position.interestAccrued).addn(1_000);
      await repayTokens(pool, borrowVault, totalDebt);

      // now withdraw all collateral
      const positionAfterRepay = await program.account.userPosition.fetch(userPosition);
      await program.methods
        .withdrawCollateral(positionAfterRepay.collateralDeposited)
        .accountsPartial({
          withdrawer: wallet.publicKey,
          pool,
          collateralMint,
          collateralVault,
          userAtaCollateral,
          userPosition,
          oracle,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      const positionClosed = await program.account.userPosition.fetch(userPosition);
      assert.equal(positionClosed.collateralDeposited.toString(), "0");
      assert.isFalse(positionClosed.isOpen);
    });

    it("rejects zero withdrawal amount", async () => {
      await assertFails(
        program.methods
          .withdrawCollateral(new BN(0))
          .accountsPartial({
            withdrawer: wallet.publicKey,
            pool,
            collateralMint,
            collateralVault,
            userAtaCollateral,
            userPosition,
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc(),
        "InvalidAmount"
      );
    });
  });

  // ─────────────────────────────────────────────────────────────────────────
  // Liquidation tests
  // ─────────────────────────────────────────────────────────────────────────
  describe("liquidate", () => {
    let oracle: PublicKey;
    let pool: PublicKey;
    let collateralVault: PublicKey;
    let borrowVault: PublicKey;
    let borrowerPosition: PublicKey;
    let liquidatorAtaCollateral: PublicKey;
    let liquidatorAtaBorrow: PublicKey;
    const seed = randomSeed();
    const depositAmount = new BN(10_000_000);
    const borrowAmount = new BN(7_000_000); // 70% LTV — safe at $100

    before(async () => {
      oracle = await setupOracle(randomSeed(), INITIAL_PRICE);
      const result = await setupPool(seed, oracle);
      pool = result.pool;
      collateralVault = result.collateralVault;
      borrowVault = result.borrowVault;
      borrowerPosition = await getUserPosition(pool, wallet.publicKey);

      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        borrowVault, wallet.payer, 100_000_000
      );
      await mintTo(
        provider.connection, wallet.payer, borrowMint,
        userAtaBorrow, wallet.payer, 50_000_000
      );

      // liquidator uses same wallet in tests — separate keypair in production
      liquidatorAtaCollateral = (await getOrCreateAssociatedTokenAccount(
        provider.connection, wallet.payer, collateralMint, wallet.publicKey
      )).address;
      liquidatorAtaBorrow = userAtaBorrow;

      // borrower deposits and borrows at 70% LTV
      await depositCollateral(pool, collateralVault, depositAmount);
      await borrowTokens(pool, borrowVault, oracle, borrowAmount);
    });

    it("5. liquidate healthy position fails", async () => {
      // price is still $100 — position is healthy at 70% LTV
      await assertFails(
        program.methods
          .liquidate(new BN(1_000_000))
          .accountsPartial({
            liquidator: wallet.publicKey,
            borrower: wallet.publicKey,
            pool,
            collateralMint,
            borrowMint,
            collateralVault,
            borrowVault,
            liquidatorAtaCollateral,
            liquidatorAtaBorrow,
            userPosition: borrowerPosition,
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc(),
        "PositionHealthy"
      );
    });

    it("4. price drop makes position unhealthy and liquidation succeeds", async () => {
      // crash price from $100 to $60
      // collateral value = 10_000_000 * $60 = $600_000_000
      // effective collateral = $600_000_000 * 80% = $480_000_000
      // debt = $700_000_000
      // health = 480 / 700 = 0.686 — unhealthy, liquidatable
      await setPrice(oracle);

      const collateralBefore = await getTokenBalance(provider, liquidatorAtaCollateral);
      const repayAmount = new BN(3_000_000);

      await program.methods
        .liquidate(repayAmount)
        .accountsPartial({
          liquidator: wallet.publicKey,
          borrower: wallet.publicKey,
          pool,
          collateralMint,
          borrowMint,
          collateralVault,
          borrowVault,
          liquidatorAtaCollateral,
          liquidatorAtaBorrow,
          userPosition: borrowerPosition,
          oracle,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      const collateralAfter = await getTokenBalance(provider, liquidatorAtaCollateral);

      // liquidator should have received collateral + 5% bonus
      assert.isTrue(
        collateralAfter > collateralBefore,
        "liquidator should receive collateral"
      );

      const position = await program.account.userPosition.fetch(borrowerPosition);
      assert.isTrue(
        position.borrowedAmount.lt(borrowAmount),
        "borrower debt should decrease after liquidation"
      );
    });

    it("liquidation reduces pool total_borrowed and total_collateral", async () => {
      const poolBefore = await program.account.lendingPool.fetch(pool);
      const repayAmount = new BN(1_000_000);

      await program.methods
        .liquidate(repayAmount)
        .accountsPartial({
          liquidator: wallet.publicKey,
          borrower: wallet.publicKey,
          pool,
          collateralMint,
          borrowMint,
          collateralVault,
          borrowVault,
          liquidatorAtaCollateral,
          liquidatorAtaBorrow,
          userPosition: borrowerPosition,
          oracle,
          tokenProgram: TOKEN_PROGRAM_ID,
        })
        .rpc();

      const poolAfter = await program.account.lendingPool.fetch(pool);
      assert.isTrue(poolAfter.totalBorrowed.lt(poolBefore.totalBorrowed));
      assert.isTrue(poolAfter.totalCollateral.lt(poolBefore.totalCollateral));
    });

    it("rejects liquidation with zero repay amount", async () => {
      await assertFails(
        program.methods
          .liquidate(new BN(0))
          .accountsPartial({
            liquidator: wallet.publicKey,
            borrower: wallet.publicKey,
            pool,
            collateralMint,
            borrowMint,
            collateralVault,
            borrowVault,
            liquidatorAtaCollateral,
            liquidatorAtaBorrow,
            userPosition: borrowerPosition,
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .rpc(),
        "InvalidAmount"
      );
    });

    it("rejects self liquidation", async () => {
      // generate a second keypair as borrower to test self-liquidation check
      const selfBorrower = Keypair.generate();
      await assertFails(
        program.methods
          .liquidate(new BN(1_000_000))
          .accountsPartial({
            liquidator: selfBorrower.publicKey,
            borrower: selfBorrower.publicKey,
            pool,
            collateralMint,
            borrowMint,
            collateralVault,
            borrowVault,
            liquidatorAtaCollateral,
            liquidatorAtaBorrow,
            userPosition: borrowerPosition,
            oracle,
            tokenProgram: TOKEN_PROGRAM_ID,
          })
          .signers([selfBorrower])
          .rpc(),
        "CannotLiquidateSelf"
      );
    });
  });
});