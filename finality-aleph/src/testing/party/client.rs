use sp_api::{Core, ProvideRuntimeApi, ApiRef, NumberFor, ExecutionContext, NativeOrEncoded};
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
use sc_client_api::{LockImportRun, ClientImportOperation, Finalizer, TransactionFor, HeaderBackend, BlockchainEvents, ImportNotifications, FinalityNotifications, StorageKey, StorageEventStream, Backend, StateBackend};
use sc_client_api::blockchain::{Error, Info, BlockStatus, CachedHeaderMetadata};
use crate::finalization::finalize_block;
use aleph_primitives::{AlephSessionApi, AuthorityId, ApiError};
use sc_consensus::{BlockImport, BlockCheckParams, ImportResult, BlockImportParams};
use std::collections::HashMap;
use sp_runtime::traits::Block as BlockT;
use sp_blockchain::HeaderMetadata;

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
use crate::{AlephConfig, SessionPeriod, ClientForAleph, ProvideAlephApi};

use std::iter;
use std::sync::Arc;
use sc_executor::{native_executor_instance, RuntimeVersion};
use std::path::PathBuf;
use std::time::Duration;
use sc_network::config::{TransportConfig};
use std::net::Ipv4Addr;
use async_trait::async_trait;
use std::marker::PhantomData;

pub struct TestClient<C, BE>(pub C, pub PhantomData<BE>);


unsafe impl<C, BE> Send for TestClient<C, BE> {}
unsafe impl<C, BE> Sync for TestClient<C, BE> {}

impl<C, BE> TestClient<C, BE> {
    pub fn new(client: C) -> Self {
        TestClient(client, PhantomData)
    }
}
/*
    LockImportRun<B, BE>
    + Finalizer<B, BE>
    + ProvideRuntimeApi<B>
    + BlockImport<B, Transaction = TransactionFor<BE, B>, Error = sp_consensus::Error>
    + HeaderBackend<B>
    + HeaderMetadata<B, Error = sp_blockchain::Error>
    + BlockchainEvents<B>
*/

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: LockImportRun<B, BE>> LockImportRun<B, BE> for TestClient<C, BE> {
    fn lock_import_and_run<R, Err, F>(&self, f: F) -> Result<R, Err> where F: FnOnce(&mut ClientImportOperation<B, BE>) -> Result<R, Err>, Err: From<Error> {
        self.0.lock_import_and_run(f)
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: Finalizer<B, BE>> Finalizer<B, BE> for TestClient<C, BE> {
    fn apply_finality(&self, operation: &mut ClientImportOperation<B, BE>, id: BlockId<B>, justification: Option<Justification>, notify: bool) -> sp_blockchain::Result<()> {
        self.0.apply_finality(operation, id, justification, notify)
    }

    fn finalize_block(&self, id: BlockId<B>, justification: Option<Justification>, notify: bool) -> sp_blockchain::Result<()> {
        self.0.finalize_block(id, justification, notify)
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: ProvideRuntimeApi<B>> ProvideRuntimeApi<B> for TestClient<C, BE> {
    type Api = C::Api;

    fn runtime_api(&self) -> ApiRef<Self::Api> {
        self.0.runtime_api()
    }
}

#[async_trait]
impl<B: BlockT, BE: sc_client_api::Backend<B>, C: BlockImport<B, Transaction = TransactionFor<BE, B>, Error = sp_consensus::Error> + Send>
BlockImport<B> for TestClient<C, BE> where
    C::Transaction: 'static
{
    type Error = C::Error;
    type Transaction = C::Transaction;

    async fn check_block(&mut self, block: BlockCheckParams<B>) -> Result<ImportResult, Self::Error> {
        self.0.check_block(block).await
    }

    async fn import_block(&mut self, block: BlockImportParams<B, Self::Transaction>, cache: HashMap<CacheKeyId, Vec<u8>>) -> Result<ImportResult, Self::Error> {
        self.0.import_block(block, cache).await
    }
}

impl<B: BlockT, BE: Sync + Send, C: HeaderBackend<B>> HeaderBackend<B> for TestClient<C, BE> {
    fn header(&self, id: BlockId<B>) -> sp_blockchain::Result<Option<<B as BlockT>::Header>> {
        self.0.header(id)
    }

    fn info(&self) -> Info<B> {
        self.0.info()
    }

    fn status(&self, id: BlockId<B>) -> sp_blockchain::Result<BlockStatus> {
        self.0.status(id)
    }

    fn number(&self, hash: <B as BlockT>::Hash) -> sp_blockchain::Result<Option<<<B as BlockT>::Header as HeaderT>::Number>> {
        self.0.number(hash)
    }

    fn hash(&self, number: NumberFor<B>) -> sp_blockchain::Result<Option<<B as BlockT>::Hash>> {
        self.0.hash(number)
    }
}

impl<B: BlockT, C: HeaderMetadata<B, Error = sp_blockchain::Error>, BE> HeaderMetadata<B> for TestClient<C, BE> {
    type Error = C::Error;

    fn header_metadata(&self, hash: <B as BlockT>::Hash) -> Result<CachedHeaderMetadata<B>, Self::Error> {
        self.0.header_metadata(hash)
    }

    fn insert_header_metadata(&self, hash: <B as BlockT>::Hash, header_metadata: CachedHeaderMetadata<B>) {
        self.0.insert_header_metadata(hash, header_metadata)
    }

    fn remove_header_metadata(&self, hash: <B as BlockT>::Hash) {
        self.0.remove_header_metadata(hash)
    }
}

impl<B: BlockT, C: BlockchainEvents<B>, BE> BlockchainEvents<B> for TestClient<C, BE> {
    fn import_notification_stream(&self) -> ImportNotifications<B> {
        self.0.import_notification_stream()
    }

    fn finality_notification_stream(&self) -> FinalityNotifications<B> {
        self.0.finality_notification_stream()
    }

    fn storage_changes_notification_stream(&self, filter_keys: Option<&[StorageKey]>, child_filter_keys: Option<&[(StorageKey, Option<Vec<StorageKey>>)]>) -> sp_blockchain::Result<StorageEventStream<<B as BlockT>::Hash>> {
        self.0.storage_changes_notification_stream(filter_keys, child_filter_keys)
    }
}
/*
impl<B: BlockT + 'static, C: 'static, BE: 'static> sp_api::Core<B> for AlephClient<C, BE> {
    fn Core_version_runtime_api_impl(&self, _: &sp_api::BlockId<B>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> Result<NativeOrEncoded<RuntimeVersion>, sp_api::ApiError> { todo!() }
    fn Core_execute_block_runtime_api_impl(&self, _: &sp_api::BlockId<B>, _: ExecutionContext, _: std::option::Option<B>, _: Vec<u8>) -> Result<NativeOrEncoded<()>, sp_api::ApiError> { todo!() }
    fn Core_initialize_block_runtime_api_impl(&self, _: &sp_api::BlockId<B>, _: ExecutionContext, _: std::option::Option<&<B as BlockT>::Header>, _: Vec<u8>) -> Result<NativeOrEncoded<()>, sp_api::ApiError> { todo!() }
}

impl<B: BlockT + 'static, C: 'static, BE: 'static> AlephSessionApi<B> for AlephClient<C, BE> {
    fn AlephSessionApi_next_session_authorities_runtime_api_impl(&self, _: &sp_api::BlockId<B>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> Result<NativeOrEncoded<Result<Vec<aleph_primitives::app::Public>, aleph_primitives::ApiError>>, sp_api::ApiError> { todo!() }
    fn AlephSessionApi_authorities_runtime_api_impl(&self, _: &sp_api::BlockId<B>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> Result<NativeOrEncoded<Vec<aleph_primitives::app::Public>>, sp_api::ApiError> { todo!() }
    fn AlephSessionApi_session_period_runtime_api_impl(&self, _: &sp_api::BlockId<B>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> Result<NativeOrEncoded<u32>, sp_api::ApiError> { todo!() }
    fn AlephSessionApi_millisecs_per_block_runtime_api_impl(&self, _: &sp_api::BlockId<B>, _: ExecutionContext, _: std::option::Option<()>, _: Vec<u8>) -> Result<NativeOrEncoded<u64>, sp_api::ApiError> { todo!() }
}*/

/*
impl<C, B, BE> Finalizer<B, BE> for TestClient<C, BE> where
    C: Finalizer<B, BE> {
    fn apply_finality(&self, operation: &mut ClientImportOperation<B, BE>, id: BlockId<B>, justification: Option<Justification>, notify: bool) -> sp_blockchain::Result<()> {
        self.0.apply_finality(operation, id, justification, notify)
    }

    fn finalize_block(&self, id: BlockId<B>, justification: Option<Justification>, notify: bool) -> sp_blockchain::Result<()> {
        self.0.finalize_block(id, justification, notify)
    }
}
*/

impl<C, B: BlockT, BE> ProvideAlephApi<B> for TestClient<C, BE> {
    fn next_session_authorities(&self, block_id: &BlockId<B>) -> Result<Vec<AuthorityId>, ApiError> {
        todo!()
    }

    fn authorities(&self, block_id: &BlockId<B>) -> Vec<AuthorityId> {
        todo!()
    }

    fn session_period(&self, block_id: &BlockId<B>) -> u32 {
        todo!()
    }

    fn millisecs_per_block(&self, block_id: &BlockId<B>) -> u64 {
        todo!()
    }
}

impl<C, B, BE> ClientForAleph<B, BE> for TestClient<C, BE> where
    B: BlockT,
    BE: Backend<B>,
    C: LockImportRun<B, BE>
    + Finalizer<B, BE>
    + ProvideRuntimeApi<B>
    + ProvideAlephApi<B>
    + HeaderBackend<B>
    + HeaderMetadata<B, Error = sp_blockchain::Error>
    + BlockchainEvents<B>
    + BlockImport<B, Transaction = TransactionFor<BE, B>, Error = sp_consensus::Error>,
    <<BE as sc_client_api::Backend<B>>::State as StateBackend<<<B as BlockT>::Header as HeaderT>::Hashing>>::Transaction: 'static
{

}