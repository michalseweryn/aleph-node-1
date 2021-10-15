use crate::chain_spec::{
    self, get_account_id_from_seed, AuthorityKeys, ChainParams, SerializablePeerId,
};
use aleph_primitives::AuthorityId as AlephId;
use aleph_runtime::AccountId;
use libp2p::identity::{ed25519 as libp2p_ed25519, PublicKey};
use sc_cli::{Error, KeystoreParams};
use sc_keystore::LocalKeystore;
use sc_service::config::{BasePath, KeystoreConfig};
use sp_application_crypto::key_types;
use sp_application_crypto::Ss58Codec;
use sp_consensus_aura::sr25519::AuthorityId as AuraId;
use sp_core::sr25519;
use sp_keystore::SyncCryptoStore;
use std::fs;
use std::io::Write;
use structopt::StructOpt;

/// returns Aura key, if absent a new key is generated
fn aura_key(keystore: &impl SyncCryptoStore) -> AuraId {
    SyncCryptoStore::sr25519_public_keys(&*keystore, key_types::AURA)
        .pop()
        .unwrap_or_else(|| {
            SyncCryptoStore::sr25519_generate_new(&*keystore, key_types::AURA, None)
                .expect("Could not create Aura key")
        })
        .into()
}

/// returns Aleph key, if absent a new key is generated
fn aleph_key(keystore: &impl SyncCryptoStore) -> AlephId {
    SyncCryptoStore::ed25519_public_keys(&*keystore, aleph_primitives::KEY_TYPE)
        .pop()
        .unwrap_or_else(|| {
            SyncCryptoStore::ed25519_generate_new(&*keystore, aleph_primitives::KEY_TYPE, None)
                .expect("Could not create Aleph key")
        })
        .into()
}

/// Returns peer id, if not p2p key found under base_path/account-id/node-key-file a new provate key gets generated
fn p2p_key(chain_params: &ChainParams, account_id: &AccountId) -> SerializablePeerId {
    let authority = account_id.to_string();
    let file = chain_params
        .base_path()
        .path()
        .join(authority)
        .join(&chain_params.node_key_file);

    if file.exists() {
        let mut file_content =
            hex::decode(fs::read(&file).unwrap()).expect("failed to decode secret as hex");
        let secret =
            libp2p_ed25519::SecretKey::from_bytes(&mut file_content).expect("Bad node key file");
        let keypair = libp2p_ed25519::Keypair::from(secret);
        SerializablePeerId::new(PublicKey::Ed25519(keypair.public()).into_peer_id())
    } else {
        let keypair = libp2p_ed25519::Keypair::generate();
        let secret = keypair.secret();
        let secret_hex = hex::encode(secret.as_ref());
        fs::write(file, secret_hex).expect("Could not write p2p secret");
        SerializablePeerId::new(PublicKey::Ed25519(keypair.public()).into_peer_id())
    }
}

fn open_keystore(
    keystore_params: &KeystoreParams,
    chain_params: &ChainParams,
    account_id: &AccountId,
) -> impl SyncCryptoStore {
    let chain_id = chain_params.chain_id();
    let base_path: BasePath = chain_params
        .base_path()
        .path()
        .join(account_id.to_string())
        .into();

    let config_dir = base_path.config_dir(chain_id);
    match keystore_params
        .keystore_config(&config_dir)
        .expect("keystore configuration should be available")
    {
        (_, KeystoreConfig::Path { path, password }) => {
            LocalKeystore::open(path, password).expect("Keystore open should succeed")
        }
        _ => unreachable!("keystore_config always returns path and password; qed"),
    }
}

fn authority_keys(
    keystore: &impl SyncCryptoStore,
    chain_params: &ChainParams,
    account_id: &AccountId,
) -> AuthorityKeys {
    let aura_key = aura_key(keystore);
    let aleph_key = aleph_key(keystore);
    let peer_id = p2p_key(chain_params, account_id);

    let account_id = account_id.clone();
    AuthorityKeys {
        account_id,
        aura_key,
        aleph_key,
        peer_id,
    }
}

/// The `bootstrap-chain` command is used to generate private keys for the genesis authorities
/// keys are written to the keystore of the authorities
/// and the chain specification is printed to stdout in the JSON format
#[derive(Debug, StructOpt)]
pub struct BootstrapChainCmd {
    /// Force raw genesis storage output.
    #[structopt(long = "raw")]
    pub raw: bool,

    #[structopt(flatten)]
    pub keystore_params: KeystoreParams,

    #[structopt(flatten)]
    pub chain_params: ChainParams,
}

impl BootstrapChainCmd {
    pub fn run(&self, mut out: impl Write) -> Result<(), Error> {
        let genesis_authorities = self
            .chain_params
            .account_ids()
            .iter()
            .map(|account_id| {
                let keystore = open_keystore(&self.keystore_params, &self.chain_params, account_id);
                authority_keys(&keystore, &self.chain_params, account_id)
            })
            .collect();

        let chain_spec = match self.chain_params.chain_id() {
            chain_spec::DEVNET_ID => {
                chain_spec::development_config(self.chain_params.clone(), genesis_authorities)
            }
            chain_id => {
                chain_spec::config(self.chain_params.clone(), genesis_authorities, chain_id)
            }
        };

        let spec = chain_spec?;
        let json = sc_service::chain_ops::build_spec(&spec, self.raw)?;
        if out.write_all(json.as_bytes()).is_err() {
            let _ = std::io::stderr().write_all(b"Error writing to stdout\n");
        }

        Ok(())
    }
}

/// The `bootstrap-node` command is used to generate key pairs for a single authority
/// private keys are stored in a specified keystore, and the public keys are written to stdout.
#[derive(Debug, StructOpt)]
pub struct BootstrapNodeCmd {
    /// Pass the AccountId of a new node
    ///
    /// Expects a string with an AccountId
    /// If this argument is not passed a random Id will be generated using account-seed argument as seed
    #[structopt(long)]
    account_id: Option<String>,

    /// human-readable authority name used as a seed to generate the AccountId
    #[structopt(long, required_unless = "account-id")]
    pub account_seed: Option<String>,

    #[structopt(flatten)]
    pub keystore_params: KeystoreParams,

    #[structopt(flatten)]
    pub chain_params: ChainParams,
}

impl BootstrapNodeCmd {
    pub fn run(&self) -> Result<(), Error> {
        let account_id = &self.account_id();
        let keystore = open_keystore(&self.keystore_params, &self.chain_params, account_id);

        let authority_keys = authority_keys(&keystore, &self.chain_params, account_id);
        let keys_json = serde_json::to_string_pretty(&authority_keys)
            .expect("serialization of authority keys should have succeed");
        println!("{}", keys_json);
        Ok(())
    }

    pub fn account_id(&self) -> AccountId {
        match &self.account_id {
            Some(id) => AccountId::from_string(id.as_str())
                .expect("Passed string is not a hex encoding of a public key"),
            None => get_account_id_from_seed::<sr25519::Public>(
                &self
                    .account_seed
                    .clone()
                    .expect("Pass account-id or node-name argument")
                    .as_str(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::{Cli, Subcommand};
    use futures::FutureExt;
    use sc_cli::CliConfiguration;
    use sc_cli::SubstrateCli;
    use sc_service::DatabaseConfig;
    use sc_service::TaskType;
    use std::ffi::OsString;
    use std::fs::File;
    use std::iter::Iterator;
    use std::path::PathBuf;

    fn to_os_string(s: &str) -> OsString {
        let mut os = OsString::new();
        os.push(s);
        os
    }

    #[test]
    fn run_nodes() {
        const ACCOUNT_IDS: [&str; 4] = [
            "5D34dL5prEUaGNQtPPZ3yN5Y6BnkfXunKXXz6fo7ZJbLwRRH",
            "5GBNeWRhZc2jXu7D55rBimKYDk8PGk8itRYFTPfC8RJLKG5o",
            "5Dfis6XL8J2P6JHUnUtArnFWndn62SydeP8ee8sG2ky9nfm9",
            "5F4H97f7nQovyrbiq4ZetaaviNwThSVcFobcA5aGab6167dK",
        ];
        const BASE_PATH: &str = "/tmp";
        let cli = Cli::from_iter(
            [
                "aleph-node",
                "bootstrap-chain",
                "--millisecs-per-block",
                "2000",
                "--session-period",
                "40",
                "--base-path",
                BASE_PATH,
                "--chain-id",
                "dev",
                "--account-ids",
                &ACCOUNT_IDS.join(","),
            ]
            .iter()
            .cloned()
            .map(to_os_string),
        );
        match cli.subcommand.unwrap() {
            Subcommand::BootstrapChain(cmd) => {
                cmd.run(File::create(&format!("{}/chain_spec.json", BASE_PATH)).unwrap())
                    .unwrap();
            }
            _ => unreachable!("the command is bootstrap chain"),
        };

        for (i, &account_id) in ACCOUNT_IDS.iter().enumerate() {
            let cli = Cli::from_iter(
                [
                    "aleph-node",
                    "purge-chain",
                    "--base-path",
                    &format!("{}/{}", BASE_PATH, account_id),
                    "--chain",
                    &format!("{}/chain_spec.json", BASE_PATH),
                    "-y",
                ]
                .iter()
                .cloned()
                .map(to_os_string),
            );
            let cmd = match cli.subcommand.unwrap() {
                Subcommand::PurgeChain(cmd) => cmd,
                _ => unreachable!("the command is purge chain"),
            };
            cmd.run(DatabaseConfig::RocksDb {
                path: PathBuf::from(format!("/tmp/{}/db", account_id)),
                cache_size: 128,
            })
            .unwrap();

            let cli = Cli::from_iter(
                [
                    "aleph-node", // the 0th arg should hopefully be ignored
                    "--validator",
                    "--chain",
                    &format!("{}/chain_spec.json", BASE_PATH),
                    "--base-path",
                    &format!("{}/{}", BASE_PATH, account_id),
                    "--name",
                    account_id,
                    "--rpc-port",
                    &(9933 + i).to_string(),
                    "--ws-port",
                    &(9944 + i).to_string(),
                    "--port",
                    &(30334 + i).to_string(),
                    "--execution",
                    "Native",
                    "-lafa=debug",
                ]
                .iter()
                .cloned()
                .map(to_os_string),
            );

            let tokio_runtime = tokio::runtime::Runtime::new().unwrap();
            let runtime_handle = tokio_runtime.handle().clone();

            let task_executor = move |fut, task_type| match task_type {
                TaskType::Async => runtime_handle.spawn(fut).map(drop),
                TaskType::Blocking => runtime_handle
                    .spawn_blocking(move || futures::executor::block_on(fut))
                    .map(drop),
            };

            let config = cli
                .run
                .create_configuration(&cli, task_executor.into())
                .unwrap();
            let mut task_manager = crate::service::new_full(config).unwrap();
            tokio_runtime.spawn(async move {
                task_manager.future().await.unwrap();
            });
        }
    }
}
