use litesvm::{types::TransactionResult, LiteSVM};
use solana_account::Account;
use solana_address::{address, Address};
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_message::Message;
use solana_signer::Signer;
use solana_transaction::Transaction;
use std::{path::PathBuf, process::Command, sync::Once};

const TOKEN_2022: Address = address!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
const SYSTEM: Address = Address::new_from_array([0; 32]);

const EXECUTE: [u8; 8] = [105, 37, 101, 197, 75, 251, 102, 26];
const INITIALIZE: [u8; 8] = [43, 34, 13, 49, 167, 88, 235, 235];

const DECIMALS: u8 = 6;
const MINT_LEN: usize = 234; // 165 + account type + TransferHook extension (4 + 64)
const TOKEN_ACCOUNT_LEN: usize = 171; // 165 + account type + TransferHookAccount (4 + 1)

const ERR_NOT_TRANSFERRING: &str = "Custom(0)";
const ERR_INVALID_TOKEN_ACCOUNT: &str = "Custom(1)";
const ERR_MINT_MISMATCH: &str = "Custom(2)";
const ERR_STATS_NOT_INITIALIZED: &str = "Custom(3)";
const ERR_INVALID_STATS: &str = "Custom(4)";
const ERR_METAS_NOT_INITIALIZED: &str = "Custom(5)";
const ERR_INVALID_MINT: &str = "Custom(6)";
const ERR_ALREADY_INITIALIZED: &str = "Custom(7)";
const ERR_INVALID_PDA: &str = "Custom(8)";

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn program_so() -> PathBuf {
    static BUILD: Once = Once::new();
    let out_dir = manifest_dir().join("target").join("deploy");
    BUILD.call_once(|| {
        let status = Command::new("cargo")
            .arg("build-sbf")
            .arg("--manifest-path")
            .arg(manifest_dir().join("Cargo.toml"))
            .arg("--sbf-out-dir")
            .arg(&out_dir)
            .status()
            .expect("failed to run cargo build-sbf");
        assert!(status.success(), "cargo build-sbf failed");
    });
    out_dir.join("transfer_stats_hook.so")
}

fn err_string(res: TransactionResult) -> String {
    format!("{:?}", res.expect_err("transaction should have failed").err)
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
struct Stats {
    count: u64,
    total: u128,
    largest: u64,
}

struct Env {
    svm: LiteSVM,
    payer: Keypair,
    authority: Keypair,
    mint: Keypair,
    program_id: Address,
}

impl Env {
    fn new() -> Self {
        let so = program_so();
        let mut svm = LiteSVM::new();
        let program_id = transfer_stats_hook::ID;
        svm.add_program_from_file(program_id, so).unwrap();
        let payer = Keypair::new();
        let authority = Keypair::new();
        svm.airdrop(&payer.pubkey(), 100_000_000_000).unwrap();
        svm.airdrop(&authority.pubkey(), 100_000_000_000).unwrap();
        Env {
            svm,
            payer,
            authority,
            mint: Keypair::new(),
            program_id,
        }
    }

    fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionResult {
        self.svm.expire_blockhash();
        let mut all: Vec<&Keypair> = vec![&self.payer];
        all.extend_from_slice(signers);
        let tx = Transaction::new(
            &all[..],
            Message::new(ixs, Some(&self.payer.pubkey())),
            self.svm.latest_blockhash(),
        );
        self.svm.send_transaction(tx)
    }

    fn stats_pda(&self) -> Address {
        Address::find_program_address(&[b"stats", self.mint.pubkey().as_ref()], &self.program_id).0
    }

    fn metas_pda(&self) -> Address {
        Address::find_program_address(
            &[b"extra-account-metas", self.mint.pubkey().as_ref()],
            &self.program_id,
        )
        .0
    }

    fn create_account_ix(&self, new: &Address, space: usize, owner: &Address) -> Instruction {
        let lamports = self.svm.minimum_balance_for_rent_exemption(space);
        let mut data = vec![0, 0, 0, 0];
        data.extend_from_slice(&lamports.to_le_bytes());
        data.extend_from_slice(&(space as u64).to_le_bytes());
        data.extend_from_slice(owner.as_ref());
        Instruction::new_with_bytes(
            SYSTEM,
            &data,
            vec![
                AccountMeta::new(self.payer.pubkey(), true),
                AccountMeta::new(*new, true),
            ],
        )
    }

    fn create_mint(&mut self) {
        let mint = self.mint.pubkey();
        let authority = self.authority.pubkey();
        let create = self.create_account_ix(&mint, MINT_LEN, &TOKEN_2022);

        let mut init_hook = vec![36, 0];
        init_hook.extend_from_slice(authority.as_ref());
        init_hook.extend_from_slice(self.program_id.as_ref());
        let init_hook = Instruction::new_with_bytes(
            TOKEN_2022,
            &init_hook,
            vec![AccountMeta::new(mint, false)],
        );

        let mut init_mint = vec![20, DECIMALS];
        init_mint.extend_from_slice(authority.as_ref());
        init_mint.push(0);
        let init_mint = Instruction::new_with_bytes(
            TOKEN_2022,
            &init_mint,
            vec![AccountMeta::new(mint, false)],
        );

        let mint_kp = Keypair::try_from(&self.mint.to_bytes()[..]).unwrap();
        self.send(&[create, init_hook, init_mint], &[&mint_kp])
            .expect("create mint");
    }

    fn create_token_account(&mut self, owner: &Address) -> Address {
        let kp = Keypair::new();
        let create = self.create_account_ix(&kp.pubkey(), TOKEN_ACCOUNT_LEN, &TOKEN_2022);
        let mut data = vec![18];
        data.extend_from_slice(owner.as_ref());
        let init = Instruction::new_with_bytes(
            TOKEN_2022,
            &data,
            vec![
                AccountMeta::new(kp.pubkey(), false),
                AccountMeta::new_readonly(self.mint.pubkey(), false),
            ],
        );
        self.send(&[create, init], &[&kp]).expect("create token account");
        kp.pubkey()
    }

    fn mint_to(&mut self, dest: &Address, amount: u64) {
        let mut data = vec![7];
        data.extend_from_slice(&amount.to_le_bytes());
        let ix = Instruction::new_with_bytes(
            TOKEN_2022,
            &data,
            vec![
                AccountMeta::new(self.mint.pubkey(), false),
                AccountMeta::new(*dest, false),
                AccountMeta::new_readonly(self.authority.pubkey(), true),
            ],
        );
        let authority = Keypair::try_from(&self.authority.to_bytes()[..]).unwrap();
        self.send(&[ix], &[&authority]).expect("mint_to");
    }

    fn initialize_ix(&self, authority: &Address) -> Instruction {
        let mut data = INITIALIZE.to_vec();
        data.extend_from_slice(&0u32.to_le_bytes());
        Instruction::new_with_bytes(
            self.program_id,
            &data,
            vec![
                AccountMeta::new(self.metas_pda(), false),
                AccountMeta::new_readonly(self.mint.pubkey(), false),
                AccountMeta::new(*authority, true),
                AccountMeta::new_readonly(SYSTEM, false),
                AccountMeta::new(self.stats_pda(), false),
            ],
        )
    }

    fn initialize(&mut self) -> TransactionResult {
        let ix = self.initialize_ix(&self.authority.pubkey());
        let authority = Keypair::try_from(&self.authority.to_bytes()[..]).unwrap();
        self.send(&[ix], &[&authority])
    }

    fn transfer(
        &mut self,
        src: &Address,
        dst: &Address,
        owner: &Keypair,
        amount: u64,
    ) -> TransactionResult {
        let mut data = vec![12];
        data.extend_from_slice(&amount.to_le_bytes());
        data.push(DECIMALS);
        let ix = Instruction::new_with_bytes(
            TOKEN_2022,
            &data,
            vec![
                AccountMeta::new(*src, false),
                AccountMeta::new_readonly(self.mint.pubkey(), false),
                AccountMeta::new(*dst, false),
                AccountMeta::new_readonly(owner.pubkey(), true),
                // Extra accounts, in the order the client appends them.
                AccountMeta::new(self.stats_pda(), false),
                AccountMeta::new_readonly(self.program_id, false),
                AccountMeta::new_readonly(self.metas_pda(), false),
            ],
        );
        self.send(&[ix], &[owner])
    }

    fn token_balance(&self, account: &Address) -> u64 {
        let acc = self.svm.get_account(account).unwrap();
        u64::from_le_bytes(acc.data[64..72].try_into().unwrap())
    }

    fn stats(&self) -> Stats {
        let acc = self.svm.get_account(&self.stats_pda()).unwrap();
        assert_eq!(acc.owner, self.program_id);
        parse_stats(&acc.data)
    }

    fn execute_ix(
        &self,
        src: &Address,
        dst: &Address,
        owner: &Address,
        amount: u64,
    ) -> Instruction {
        let mut data = EXECUTE.to_vec();
        data.extend_from_slice(&amount.to_le_bytes());
        Instruction::new_with_bytes(
            self.program_id,
            &data,
            vec![
                AccountMeta::new_readonly(*src, false),
                AccountMeta::new_readonly(self.mint.pubkey(), false),
                AccountMeta::new_readonly(*dst, false),
                AccountMeta::new_readonly(*owner, false),
                AccountMeta::new_readonly(self.metas_pda(), false),
                AccountMeta::new(self.stats_pda(), false),
            ],
        )
    }

    fn put_token_account(&mut self, address: &Address, transferring: bool, mint: &Address) {
        let mut data = vec![0u8; TOKEN_ACCOUNT_LEN];
        data[..32].copy_from_slice(mint.as_ref());
        data[165] = 2;
        data[166..168].copy_from_slice(&15u16.to_le_bytes());
        data[168..170].copy_from_slice(&1u16.to_le_bytes());
        data[170] = transferring as u8;
        self.svm
            .set_account(
                *address,
                Account {
                    lamports: 10_000_000,
                    data,
                    owner: TOKEN_2022,
                    executable: false,
                    rent_epoch: 0,
                },
            )
            .unwrap();
    }
}

fn parse_stats(data: &[u8]) -> Stats {
    assert_eq!(data.len(), 48);
    Stats {
        count: u64::from_le_bytes(data[16..24].try_into().unwrap()),
        total: u128::from_le_bytes(data[24..40].try_into().unwrap()),
        largest: u64::from_le_bytes(data[40..48].try_into().unwrap()),
    }
}

/// Env with the mint created and the hook initialized, plus two funded wallets.
struct Ready {
    env: Env,
    alice: Keypair,
    bob: Keypair,
    alice_ata: Address,
    bob_ata: Address,
}

fn ready() -> Ready {
    let mut env = Env::new();
    env.create_mint();
    let alice = Keypair::new();
    let bob = Keypair::new();
    let alice_ata = env.create_token_account(&alice.pubkey());
    let bob_ata = env.create_token_account(&bob.pubkey());
    env.mint_to(&alice_ata, 1_000_000);
    env.mint_to(&bob_ata, 1_000_000);
    env.initialize().expect("initialize");
    Ready {
        env,
        alice,
        bob,
        alice_ata,
        bob_ata,
    }
}

#[test]
fn hook_init_json_matches_account_layout() {
    let json = std::fs::read_to_string(manifest_dir().join("hook-init.json")).unwrap();
    let compact: String = json.chars().filter(|c| !c.is_whitespace()).collect();
    assert_eq!(compact, r#"{"initAccounts":["stats"]}"#);
}

#[test]
fn initialize_creates_metas_and_stats() {
    let r = ready();
    let env = &r.env;

    let metas = env.svm.get_account(&env.metas_pda()).unwrap();
    assert_eq!(metas.owner, env.program_id);
    assert_eq!(metas.data.len(), 51);
    assert_eq!(metas.data[0..8], EXECUTE);
    assert_eq!(u32::from_le_bytes(metas.data[8..12].try_into().unwrap()), 39);
    assert_eq!(u32::from_le_bytes(metas.data[12..16].try_into().unwrap()), 1);
    // discriminator 1 = PDA of the hook program; seeds: literal "stats", account key #1 (mint)
    assert_eq!(metas.data[16], 1);
    assert_eq!(
        metas.data[17..28],
        [1, 5, b's', b't', b'a', b't', b's', 3, 1, 0, 0]
    );
    assert_eq!(metas.data[49], 0, "not a signer");
    assert_eq!(metas.data[50], 1, "writable");

    let stats = env.svm.get_account(&env.stats_pda()).unwrap();
    assert_eq!(stats.owner, env.program_id);
    assert_eq!(
        env.stats(),
        Stats {
            count: 0,
            total: 0,
            largest: 0
        }
    );
}

#[test]
fn transfers_update_stats_in_both_directions() {
    let mut r = ready();
    let (alice, bob) = (r.alice.insecure_clone(), r.bob.insecure_clone());
    let (a, b) = (r.alice_ata, r.bob_ata);

    r.env.transfer(&a, &b, &alice, 100).expect("buy 100");
    r.env.transfer(&b, &a, &bob, 500).expect("sell 500");
    let meta = r.env.transfer(&a, &b, &alice, 250).expect("buy 250");
    println!("transfer CU: {}", meta.compute_units_consumed);

    assert_eq!(
        r.env.stats(),
        Stats {
            count: 3,
            total: 850,
            largest: 500
        }
    );
    assert_eq!(r.env.token_balance(&a), 1_000_000 - 100 + 500 - 250);
    assert_eq!(r.env.token_balance(&b), 1_000_000 + 100 - 500 + 250);
}

#[test]
fn zero_amount_transfer_is_counted_and_not_blocked() {
    let mut r = ready();
    let alice = r.alice.insecure_clone();
    let (a, b) = (r.alice_ata, r.bob_ata);
    r.env.transfer(&a, &b, &alice, 0).expect("zero transfer");
    assert_eq!(
        r.env.stats(),
        Stats {
            count: 1,
            total: 0,
            largest: 0
        }
    );
}

#[test]
fn full_balance_and_max_values_are_never_blocked() {
    let mut r = ready();
    let alice = r.alice.insecure_clone();
    let (a, b) = (r.alice_ata, r.bob_ata);

    // Put the stats account at its numeric limits: the counters must saturate.
    let stats_addr = r.env.stats_pda();
    let mut acc = r.env.svm.get_account(&stats_addr).unwrap();
    acc.data[16..24].copy_from_slice(&u64::MAX.to_le_bytes());
    acc.data[24..40].copy_from_slice(&(u128::MAX - 5).to_le_bytes());
    acc.data[40..48].copy_from_slice(&(u64::MAX - 1).to_le_bytes());
    r.env.svm.set_account(stats_addr, acc).unwrap();

    r.env
        .transfer(&a, &b, &alice, 1_000_000)
        .expect("transfer must not fail on counter overflow");
    assert_eq!(
        r.env.stats(),
        Stats {
            count: u64::MAX,
            total: u128::MAX,
            largest: u64::MAX - 1
        }
    );
}

#[test]
fn transfer_fails_before_initialization() {
    let mut env = Env::new();
    env.create_mint();
    let alice = Keypair::new();
    let bob = Keypair::new();
    let a = env.create_token_account(&alice.pubkey());
    let b = env.create_token_account(&bob.pubkey());
    env.mint_to(&a, 1_000);

    assert!(env.transfer(&a, &b, &alice, 10).is_err());
    assert_eq!(env.token_balance(&a), 1_000);
    assert_eq!(env.token_balance(&b), 0);
}

#[test]
fn transfer_fails_if_stats_account_is_replaced() {
    let mut r = ready();
    let alice = r.alice.insecure_clone();
    let (a, b) = (r.alice_ata, r.bob_ata);

    // Another program-owned, correctly sized account is not the stats PDA.
    let stats_addr = r.env.stats_pda();
    let mut acc = r.env.svm.get_account(&stats_addr).unwrap();
    acc.owner = SYSTEM;
    acc.data = vec![];
    r.env.svm.set_account(stats_addr, acc).unwrap();

    let err = err_string(r.env.transfer(&a, &b, &alice, 10));
    assert!(err.contains(ERR_STATS_NOT_INITIALIZED), "{err}");
    assert_eq!(r.env.token_balance(&a), 1_000_000);
}

#[test]
fn execute_outside_a_transfer_is_rejected() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, false, &mint);
    r.env.put_token_account(&dst, false, &mint);

    let ix = r.env.execute_ix(&src, &dst, &r.alice.pubkey(), 1_000);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_NOT_TRANSFERRING), "{err}");
    assert_eq!(r.env.stats().count, 0);
}

#[test]
fn execute_with_transferring_flag_updates_stats() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);

    let ix = r.env.execute_ix(&src, &dst, &r.alice.pubkey(), 777);
    r.env.send(&[ix], &[]).expect("execute");
    assert_eq!(
        r.env.stats(),
        Stats {
            count: 1,
            total: 777,
            largest: 777
        }
    );
}

#[test]
fn execute_rejects_account_not_owned_by_token_2022() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);
    let mut acc = r.env.svm.get_account(&src).unwrap();
    acc.owner = SYSTEM;
    r.env.svm.set_account(src, acc).unwrap();

    let ix = r.env.execute_ix(&src, &dst, &r.alice.pubkey(), 1);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_INVALID_TOKEN_ACCOUNT), "{err}");
}

#[test]
fn execute_rejects_foreign_mint_accounts() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let other = Address::new_unique();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &other);

    let ix = r.env.execute_ix(&src, &dst, &r.alice.pubkey(), 1);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_MINT_MISMATCH), "{err}");

    r.env.put_token_account(&src, true, &other);
    r.env.put_token_account(&dst, true, &mint);
    let ix = r.env.execute_ix(&src, &dst, &r.alice.pubkey(), 1);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_MINT_MISMATCH), "{err}");
}

#[test]
fn execute_fails_when_state_is_missing() {
    // Nothing initialized: no extra metas account.
    let mut env = Env::new();
    env.create_mint();
    let mint = env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    env.put_token_account(&src, true, &mint);
    env.put_token_account(&dst, true, &mint);
    let ix = env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    let err = err_string(env.send(&[ix], &[]));
    assert!(err.contains(ERR_METAS_NOT_INITIALIZED), "{err}");

    // Metas exist but the stats account was never created.
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);
    let stats_addr = r.env.stats_pda();
    let mut acc = r.env.svm.get_account(&stats_addr).unwrap();
    acc.owner = SYSTEM;
    acc.data = vec![];
    r.env.svm.set_account(stats_addr, acc).unwrap();
    let ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_STATS_NOT_INITIALIZED), "{err}");
}

#[test]
fn execute_rejects_wrong_or_readonly_stats() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);

    // A copy of the stats data at a different, program-owned address.
    let fake = Address::new_unique();
    let real = r.env.svm.get_account(&r.env.stats_pda()).unwrap();
    r.env.svm.set_account(fake, real).unwrap();
    let mut ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    ix.accounts[5] = AccountMeta::new(fake, false);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_INVALID_STATS), "{err}");

    // Read-only stats.
    let mut ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    ix.accounts[5] = AccountMeta::new_readonly(r.env.stats_pda(), false);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_INVALID_STATS), "{err}");
}

#[test]
fn execute_requires_all_accounts() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);
    let mut ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    ix.accounts.pop();
    assert!(r.env.send(&[ix], &[]).is_err());
}

#[test]
fn initialize_twice_fails() {
    let mut r = ready();
    let err = err_string(r.env.initialize());
    assert!(err.contains(ERR_ALREADY_INITIALIZED), "{err}");
}

#[test]
fn initialize_requires_mint_authority_signature() {
    let mut env = Env::new();
    env.create_mint();

    // The right authority, but not signing.
    let mut ix = env.initialize_ix(&env.authority.pubkey());
    ix.accounts[2].is_signer = false;
    assert!(env.send(&[ix], &[]).is_err());

    // A different signer that is not the mint authority.
    let imposter = Keypair::new();
    env.svm.airdrop(&imposter.pubkey(), 10_000_000_000).unwrap();
    let ix = env.initialize_ix(&imposter.pubkey());
    let err = err_string(env.send(&[ix], &[&imposter]));
    assert!(err.contains(ERR_INVALID_MINT), "{err}");
    assert!(env.svm.get_account(&env.stats_pda()).is_none());
}

#[test]
fn initialize_rejects_non_token_2022_mint() {
    let mut env = Env::new();
    let fake_mint = Keypair::new();
    let create = env.create_account_ix(&fake_mint.pubkey(), 82, &SYSTEM);
    env.send(&[create], &[&fake_mint]).unwrap();
    env.mint = fake_mint;
    let err = err_string(env.initialize());
    assert!(err.contains(ERR_INVALID_MINT), "{err}");
}

#[test]
fn initialize_rejects_wrong_pdas_and_system_program() {
    let mut env = Env::new();
    env.create_mint();
    let authority = Keypair::try_from(&env.authority.to_bytes()[..]).unwrap();

    let mut ix = env.initialize_ix(&authority.pubkey());
    ix.accounts[4] = AccountMeta::new(Address::new_unique(), false);
    let err = err_string(env.send(&[ix], &[&authority]));
    assert!(err.contains(ERR_INVALID_PDA), "{err}");

    let mut ix = env.initialize_ix(&authority.pubkey());
    ix.accounts[0] = AccountMeta::new(Address::new_unique(), false);
    let err = err_string(env.send(&[ix], &[&authority]));
    assert!(err.contains(ERR_INVALID_PDA), "{err}");

    let mut ix = env.initialize_ix(&authority.pubkey());
    ix.accounts[3] = AccountMeta::new_readonly(Address::new_unique(), false);
    assert!(env.send(&[ix], &[&authority]).is_err());

    // Missing the stats account.
    let mut ix = env.initialize_ix(&authority.pubkey());
    ix.accounts.pop();
    assert!(env.send(&[ix], &[&authority]).is_err());

    assert!(env.svm.get_account(&env.metas_pda()).is_none());
    assert!(env.svm.get_account(&env.stats_pda()).is_none());
}

#[test]
fn initialize_works_when_pdas_were_prefunded() {
    let mut env = Env::new();
    env.create_mint();
    for pda in [env.metas_pda(), env.stats_pda()] {
        let rent_for_empty = env.svm.minimum_balance_for_rent_exemption(0);
        env.svm.airdrop(&pda, rent_for_empty).unwrap();
    }
    env.initialize().expect("initialize with prefunded PDAs");
    assert_eq!(env.stats().count, 0);
}

#[test]
fn unknown_instruction_is_rejected() {
    let mut env = Env::new();
    let ix = Instruction::new_with_bytes(env.program_id, &[1, 2, 3, 4, 5, 6, 7, 8], vec![]);
    assert!(env.send(&[ix], &[]).is_err());
    let ix = Instruction::new_with_bytes(env.program_id, &[1, 2], vec![]);
    assert!(env.send(&[ix], &[]).is_err());
}

#[test]
fn execute_rejects_corrupted_stats_data() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);

    let stats_addr = r.env.stats_pda();
    let original = r.env.svm.get_account(&stats_addr).unwrap();

    let mut bad_magic = original.clone();
    bad_magic.data[0] ^= 0xff;
    r.env.svm.set_account(stats_addr, bad_magic).unwrap();
    let ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_STATS_NOT_INITIALIZED), "{err}");

    let mut bad_len = original.clone();
    bad_len.data.truncate(40);
    r.env.svm.set_account(stats_addr, bad_len).unwrap();
    let ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_STATS_NOT_INITIALIZED), "{err}");

    let mut bad_bump = original;
    bad_bump.data[8] = bad_bump.data[8].wrapping_sub(1);
    r.env.svm.set_account(stats_addr, bad_bump).unwrap();
    let ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    assert!(r.env.send(&[ix], &[]).is_err());
}

#[test]
fn execute_rejects_metas_not_owned_by_program() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);

    let metas_addr = r.env.metas_pda();
    let mut acc = r.env.svm.get_account(&metas_addr).unwrap();
    acc.owner = SYSTEM;
    acc.data = vec![];
    r.env.svm.set_account(metas_addr, acc).unwrap();

    let ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 1);
    let err = err_string(r.env.send(&[ix], &[]));
    assert!(err.contains(ERR_METAS_NOT_INITIALIZED), "{err}");
}

#[test]
fn execute_rejects_bad_instruction_data_length() {
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);

    let mut ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 5);
    ix.data.pop();
    assert!(r.env.send(&[ix], &[]).is_err());
    let mut ix = r.env.execute_ix(&src, &dst, &Address::new_unique(), 5);
    ix.data.push(0);
    assert!(r.env.send(&[ix], &[]).is_err());
    assert_eq!(r.env.stats().count, 0);
}

#[test]
fn execute_accepts_pool_vault_owner_off_curve() {
    // Pool vaults are owned by a PDA of the pool program; the hook must not
    // require the owner to be a signer or an on-curve key.
    let mut r = ready();
    let mint = r.env.mint.pubkey();
    let (src, dst) = (Address::new_unique(), Address::new_unique());
    r.env.put_token_account(&src, true, &mint);
    r.env.put_token_account(&dst, true, &mint);
    let (pool_authority, _) =
        Address::find_program_address(&[b"pool_authority"], &Address::new_unique());

    for amount in [10u64, 20] {
        let ix = r.env.execute_ix(&src, &dst, &pool_authority, amount);
        r.env.send(&[ix], &[]).expect("execute with PDA owner");
    }
    assert_eq!(
        r.env.stats(),
        Stats {
            count: 2,
            total: 30,
            largest: 20
        }
    );
}

#[test]
fn largest_tracks_maximum_not_latest() {
    let mut r = ready();
    let alice = r.alice.insecure_clone();
    let (a, b) = (r.alice_ata, r.bob_ata);
    for amount in [300u64, 7, 300, 299] {
        r.env.transfer(&a, &b, &alice, amount).expect("transfer");
    }
    assert_eq!(
        r.env.stats(),
        Stats {
            count: 4,
            total: 906,
            largest: 300
        }
    );
}

#[test]
fn failed_transfer_does_not_change_stats() {
    let mut r = ready();
    let alice = r.alice.insecure_clone();
    let (a, b) = (r.alice_ata, r.bob_ata);
    // More than the balance: the token program fails and the tx is rolled back.
    assert!(r.env.transfer(&a, &b, &alice, 2_000_000).is_err());
    assert_eq!(r.env.stats().count, 0);
}

#[test]
fn initialize_rejects_mint_without_authority() {
    let mut env = Env::new();
    env.create_mint();
    let authority = Keypair::try_from(&env.authority.to_bytes()[..]).unwrap();

    // SetAuthority(MintTokens -> None)
    let data = vec![6, 0, 0];
    let ix = Instruction::new_with_bytes(
        TOKEN_2022,
        &data,
        vec![
            AccountMeta::new(env.mint.pubkey(), false),
            AccountMeta::new_readonly(authority.pubkey(), true),
        ],
    );
    env.send(&[ix], &[&authority])
        .expect("revoke mint authority");

    let ix = env.initialize_ix(&authority.pubkey());
    let err = err_string(env.send(&[ix], &[&authority]));
    assert!(err.contains(ERR_INVALID_MINT), "{err}");
    assert!(env.svm.get_account(&env.stats_pda()).is_none());
}

#[test]
fn initialize_requires_writable_accounts() {
    let mut env = Env::new();
    env.create_mint();
    let authority = Keypair::try_from(&env.authority.to_bytes()[..]).unwrap();

    for idx in [0usize, 2, 4] {
        let mut ix = env.initialize_ix(&authority.pubkey());
        ix.accounts[idx].is_writable = false;
        assert!(env.send(&[ix], &[&authority]).is_err(), "account {idx}");
    }
    assert!(env.svm.get_account(&env.metas_pda()).is_none());
    assert!(env.svm.get_account(&env.stats_pda()).is_none());
}

#[test]
fn initialize_ignores_passed_extra_account_list() {
    let mut env = Env::new();
    env.create_mint();
    let authority = Keypair::try_from(&env.authority.to_bytes()[..]).unwrap();

    let mut ix = env.initialize_ix(&authority.pubkey());
    ix.data.truncate(8);
    ix.data.extend_from_slice(&[0xAB; 40]);
    env.send(&[ix], &[&authority])
        .expect("initialize with junk list");

    let metas = env.svm.get_account(&env.metas_pda()).unwrap();
    assert_eq!(metas.data.len(), 51);
    assert_eq!(
        metas.data[17..28],
        [1, 5, b's', b't', b'a', b't', b's', 3, 1, 0, 0]
    );
}

#[test]
fn initialize_rent_is_paid_by_authority() {
    let mut env = Env::new();
    env.create_mint();
    let before = env.svm.get_balance(&env.authority.pubkey()).unwrap();
    env.initialize().expect("initialize");
    let after = env.svm.get_balance(&env.authority.pubkey()).unwrap();
    let rent = env.svm.minimum_balance_for_rent_exemption(51)
        + env.svm.minimum_balance_for_rent_exemption(48);
    assert_eq!(before - after, rent);
}
