use sc_client_api::execution_extensions::ExecutionStrategies;

use sp_application_crypto::{Ss58Codec, key_types};
use sc_keystore::LocalKeystore;
use sc_service::{config::{
    Configuration,
    KeystoreConfig,
    TaskType,
    WasmExecutionMethod
}, KeepBlocks, TransactionStorageMode, TaskExecutor, BasePath, ChainSpec, RuntimeGenesis, ChainSpecExtension, GenericChainSpec, TaskManager};
use sc_network::{config::{Role, NetworkConfiguration}};
use sp_keystore::SyncCryptoStore;
use substrate_test_runtime::Block;
use crate::party::{run_consensus_party, AlephParams};
use crate::{AlephConfig, SessionPeriod};
use super::network::TestNetworkHub;

use std::iter;
use std::sync::Arc;
use sc_executor::native_executor_instance;
use std::path::PathBuf;
use std::time::Duration;
use sc_network::config::{TransportConfig};
use std::net::Ipv4Addr;

const ACCOUNT_IDS: [&str; 4] = [
    "5D34dL5prEUaGNQtPPZ3yN5Y6BnkfXunKXXz6fo7ZJbLwRRH",
    "5GBNeWRhZc2jXu7D55rBimKYDk8PGk8itRYFTPfC8RJLKG5o",
    "5Dfis6XL8J2P6JHUnUtArnFWndn62SydeP8ee8sG2ky9nfm9",
    "5F4H97f7nQovyrbiq4ZetaaviNwThSVcFobcA5aGab6167dK",
];

const CHAIN_ID: &str = "test";

fn base_path(i: usize) -> BasePath {
    BasePath::Permanenent(PathBuf::from(format!("/tmp/alephtest{}/", i)))
}

/*
fn configuration(i: usize, chain_spec: impl ChainSpec + Clone + Sized + 'static) -> Configuration {
    let key_seed = format!("test key seed {}", i);
    let db_path = base_path(i).path().join("db");
    let task_executor = TaskExecutor::from(|fut, _tt| async move {
        let tt = match _tt {
            TaskType::Async => {tokio::spawn(fut); },
            TaskType::Blocking => {fut.await; },
        };
    });
    Configuration {
        impl_name: "aleph-test-node".to_string(),
        impl_version: "0.1".to_string(),
        role: Role::Authority,
        transaction_pool: Default::default(),
        network: NetworkConfiguration::new(
            key_seed.clone(),
            "network/test/0.1",
            Default::default(),
            None,
        ),
        keystore: KeystoreConfig::Path { path: base_path(i).path().to_path_buf(), password: None },
        keystore_remote: Default::default(),
        database: DatabaseConfig::RocksDb { path: db_path, cache_size: 128 },
        state_cache_size: 16777216,
        state_cache_child_ratio: None,
        state_pruning: Default::default(),
        keep_blocks: KeepBlocks::All,
        transaction_storage: TransactionStorageMode::BlockBody,
        chain_spec: Box::new(chain_spec.clone()),
        wasm_method: WasmExecutionMethod::Interpreted,
        wasm_runtime_overrides: Default::default(),
        // NOTE: we enforce the use of the native runtime to make the errors more debuggable
        execution_strategies: ExecutionStrategies {
            syncing: sc_client_api::ExecutionStrategy::NativeWhenPossible,
            importing: sc_client_api::ExecutionStrategy::NativeWhenPossible,
            block_construction: sc_client_api::ExecutionStrategy::NativeWhenPossible,
            offchain_worker: sc_client_api::ExecutionStrategy::NativeWhenPossible,
            other: sc_client_api::ExecutionStrategy::NativeWhenPossible,
        },
        rpc_http: None,
        rpc_ws: None,
        rpc_ipc: None,
        rpc_max_payload: None,
        rpc_ws_max_connections: None,
        rpc_http_threads: None,
        rpc_cors: None,
        rpc_methods: Default::default(),
        prometheus_config: None,
        telemetry_endpoints: None,
        telemetry_external_transport: None,
        default_heap_pages: None,
        offchain_worker: Default::default(),
        force_authoring: false,
        disable_grandpa: false,
        dev_key_seed: Some(key_seed.to_string()),
        tracing_targets: None,
        tracing_receiver: Default::default(),
        max_runtime_instances: 8,
        announce_block: true,
        base_path: Some(base_path(i)),
        informant_output_format: Default::default(),
        disable_log_reloading: false,
        task_executor
    }
}
*/

use sp_api::{Core, ProvideRuntimeApi, ApiRef, NumberFor};
use sp_runtime::{generic::BlockId, traits::{HashFor, Header as HeaderT}, Justification};
use sp_state_machine::{
    create_proof_check_backend, execution_proof_check_on_trie_backend, ExecutionStrategy,
};
use substrate_test_runtime_client::{
    prelude::*,
    runtime::{DecodeFails, Header, Transfer, RuntimeApi},
    DefaultTestClientBuilderExt, TestClientBuilder,
};
use substrate_test_runtime::TestAPI;

use codec::Encode;
use sc_block_builder::BlockBuilderProvider;
use sp_consensus::{SelectChain, CacheKeyId};
use futures::FutureExt;
use sc_client_api::{LockImportRun, ClientImportOperation, Finalizer, TransactionFor, HeaderBackend};
use sc_client_api::blockchain::{Error, Info, BlockStatus, CachedHeaderMetadata};
use crate::finalization::finalize_block;
use sc_consensus::{BlockImport, BlockCheckParams, ImportResult, BlockImportParams};
use std::collections::HashMap;
use sp_runtime::traits::Block as BlockT;
use sp_blockchain::HeaderMetadata;


pub(crate) mod client;

#[tokio::test]
async fn run() {
    let size = ACCOUNT_IDS.len();

    /*let keystores: Vec<Arc<LocalKeystore>> = (0..size).map(|i| Arc::new(LocalKeystore::open(base_path(i).path(), None).unwrap())).collect();
    let authorities: Vec<_> = keystores.iter().zip(&ACCOUNT_IDS).map(|(keystore, &account_id)| {
        let aura_key = SyncCryptoStore::sr25519_public_keys(&**keystore, key_types::AURA)
            .pop()
            .unwrap_or_else(|| {
                SyncCryptoStore::sr25519_generate_new(&**keystore, key_types::AURA, None)
                    .unwrap()
            })
            .into();
        let aleph_key =
            SyncCryptoStore::ed25519_public_keys(&**keystore, aleph_primitives::KEY_TYPE)
                .pop()
                .unwrap_or_else(|| {
                    SyncCryptoStore::ed25519_generate_new(&**keystore, aleph_primitives::KEY_TYPE, None)
                        .unwrap()
                })
                .into();
        AuthorityKeys {
            account_id: AccountId::from_string(account_id).unwrap(),
            aura_key,
            aleph_key,
        }
    }).collect();
    let chain_params = ChainParams {
        account_ids: Some(ACCOUNT_IDS.iter().map(|id| id.to_string()).collect()),
        base_path: PathBuf::new(), // the parameter is ignored anyway
        chain_id: CHAIN_ID.to_string(),
        chain_name: Some("Test".to_string()),
        millisecs_per_block: None,
        n_members: Some(4),
        session_period: None,
        token_symbol: None,
    };
    let chain_spec = development_config(chain_params, authorities).unwrap();*/
    let (network_hub, networks) = TestNetworkHub::new(size);

    let mut justification_txs = Vec::with_capacity(size);

    for (i, network) in networks.into_iter().enumerate() {
        // let configuration = configuration(i, chain_spec.clone());

        let builder = TestClientBuilder::with_default_backend();
        let backend = builder.backend();
        let select_chain = sc_consensus::LongestChain::new(backend.clone());
        let mut client = Arc::new(client::TestClient::new(builder.build()));
        ProvideRuntimeApi::runtime_api(&*client);

        let (justification_tx, justification_rx) = futures::channel::mpsc::unbounded();
        let keystore = Arc::new(LocalKeystore::open(base_path(i).path(), None).unwrap());
        justification_txs.push(justification_tx);

        let tokio_runtime = tokio::runtime::Builder::new()
            .threaded_scheduler()
            .enable_all()
            .build()
            .unwrap();
        let runtime_handle = tokio_runtime.handle().clone();

        let task_executor = move |fut, task_type| match task_type {
            TaskType::Async => runtime_handle.spawn(fut).map(drop),
            TaskType::Blocking => runtime_handle
                .spawn_blocking(move || futures::executor::block_on(fut))
                .map(drop),
        };

        let manager = TaskManager::new(TaskExecutor::from(task_executor), None).unwrap();


        tokio::spawn(run_consensus_party(AlephParams {
            config: AlephConfig {
                network,
                client,
                select_chain,
                spawn_handle: manager.spawn_handle(),
                keystore,
                justification_rx,
                metrics: None,
                session_period: SessionPeriod(400),
                millisecs_per_block: Default::default()
            }
        }));
    }
    tokio::spawn(network_hub.run());
    futures_timer::Delay::new(Duration::from_secs(10)).await;

}