use sp_api::{ApiRef, NumberFor, ProvideRuntimeApi};
use sp_runtime::{generic::BlockId, traits::Header as HeaderT, Justification};
use std::marker::PhantomData;
use substrate_test_runtime_client::prelude::*;

use aleph_primitives::MillisecsPerBlock;
use aleph_primitives::{AlephSessionApi, ApiError, AuthorityId};
use sc_client_api::blockchain::{BlockStatus, CachedHeaderMetadata, Error, Info};
use sc_client_api::{
    Backend, BlockchainEvents, ClientImportOperation, FinalityNotifications, Finalizer,
    HeaderBackend, ImportNotifications, LockImportRun, StorageEventStream, StorageKey,
    TransactionFor,
};
use sc_consensus::{BlockCheckParams, BlockImport, BlockImportParams, ImportResult};
use sp_blockchain::HeaderMetadata;
use sp_consensus::CacheKeyId;
use sp_runtime::traits::Block as BlockT;
use std::collections::HashMap;

use crate::{ClientForAleph, SessionPeriod};

use async_trait::async_trait;

pub struct TestClient<C, BE>(pub C, pub PhantomData<BE>);

unsafe impl<C, BE> Send for TestClient<C, BE> {}
unsafe impl<C, BE> Sync for TestClient<C, BE> {}

impl<C, BE> TestClient<C, BE> {
    pub fn new(client: C) -> Self {
        TestClient(client, PhantomData)
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: LockImportRun<B, BE>> LockImportRun<B, BE>
    for TestClient<C, BE>
{
    fn lock_import_and_run<R, Err, F>(&self, f: F) -> Result<R, Err>
    where
        F: FnOnce(&mut ClientImportOperation<B, BE>) -> Result<R, Err>,
        Err: From<Error>,
    {
        self.0.lock_import_and_run(f)
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: Finalizer<B, BE>> Finalizer<B, BE>
    for TestClient<C, BE>
{
    fn apply_finality(
        &self,
        operation: &mut ClientImportOperation<B, BE>,
        id: BlockId<B>,
        justification: Option<Justification>,
        notify: bool,
    ) -> sp_blockchain::Result<()> {
        self.0.apply_finality(operation, id, justification, notify)
    }

    fn finalize_block(
        &self,
        id: BlockId<B>,
        justification: Option<Justification>,
        notify: bool,
    ) -> sp_blockchain::Result<()> {
        self.0.finalize_block(id, justification, notify)
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: ProvideRuntimeApi<B>> ProvideRuntimeApi<B>
    for TestClient<C, BE>
{
    type Api = C::Api;

    fn runtime_api(&self) -> ApiRef<Self::Api> {
        self.0.runtime_api()
    }
}

#[async_trait]
impl<
        B: BlockT,
        BE: sc_client_api::Backend<B>,
        C: BlockImport<B, Transaction = TransactionFor<BE, B>, Error = sp_consensus::Error> + Send,
    > BlockImport<B> for TestClient<C, BE>
where
    C::Transaction: 'static,
{
    type Error = C::Error;
    type Transaction = C::Transaction;

    async fn check_block(
        &mut self,
        block: BlockCheckParams<B>,
    ) -> Result<ImportResult, Self::Error> {
        self.0.check_block(block).await
    }

    async fn import_block(
        &mut self,
        block: BlockImportParams<B, Self::Transaction>,
        cache: HashMap<CacheKeyId, Vec<u8>>,
    ) -> Result<ImportResult, Self::Error> {
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

    fn number(
        &self,
        hash: <B as BlockT>::Hash,
    ) -> sp_blockchain::Result<Option<<<B as BlockT>::Header as HeaderT>::Number>> {
        self.0.number(hash)
    }

    fn hash(&self, number: NumberFor<B>) -> sp_blockchain::Result<Option<<B as BlockT>::Hash>> {
        self.0.hash(number)
    }
}

impl<B: BlockT, C: HeaderMetadata<B, Error = sp_blockchain::Error>, BE> HeaderMetadata<B>
    for TestClient<C, BE>
{
    type Error = C::Error;

    fn header_metadata(
        &self,
        hash: <B as BlockT>::Hash,
    ) -> Result<CachedHeaderMetadata<B>, Self::Error> {
        self.0.header_metadata(hash)
    }

    fn insert_header_metadata(
        &self,
        hash: <B as BlockT>::Hash,
        header_metadata: CachedHeaderMetadata<B>,
    ) {
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

    fn storage_changes_notification_stream(
        &self,
        filter_keys: Option<&[StorageKey]>,
        child_filter_keys: Option<&[(StorageKey, Option<Vec<StorageKey>>)]>,
    ) -> sp_blockchain::Result<StorageEventStream<<B as BlockT>::Hash>> {
        self.0
            .storage_changes_notification_stream(filter_keys, child_filter_keys)
    }
}

impl<B: BlockT, C, BE> ClientForAleph<B, BE> for TestClient<C, BE>
where
    BE: Backend<B>,
    B: BlockT,
    C: LockImportRun<B, BE>
        + Finalizer<B, BE>
        + ProvideRuntimeApi<B>
        + HeaderBackend<B>
        + HeaderMetadata<B, Error = sp_blockchain::Error>
        + BlockchainEvents<B>
        + BlockImport<B, Transaction = TransactionFor<BE, B>, Error = sp_consensus::Error>,
{
    fn next_session_authorities(
        &self,
        block_id: &BlockId<B>,
    ) -> Result<Result<Vec<AuthorityId>, ApiError>, sp_api::ApiError> {
        self.runtime_api().next_session_authorities(block_id)
    }

    fn authorities(&self, block_id: &BlockId<B>) -> Result<Vec<AuthorityId>, sp_api::ApiError> {
        self.runtime_api().authorities(block_id)
    }

    fn session_period(&self, block_id: &BlockId<B>) -> Result<SessionPeriod, sp_api::ApiError> {
        self.runtime_api().session_period(block_id)
    }

    fn millisecs_per_block(
        &self,
        block_id: &BlockId<B>,
    ) -> Result<MillisecsPerBlock, sp_api::ApiError> {
        self.runtime_api().millisecs_per_block(block_id)
    }
}
