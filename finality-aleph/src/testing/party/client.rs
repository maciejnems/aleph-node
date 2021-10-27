use sp_api::{ApiRef, NumberFor, ProvideRuntimeApi};
use sp_runtime::{generic::BlockId, traits::Header as HeaderT, Justification};
use std::marker::PhantomData;

use aleph_primitives::MillisecsPerBlock;
use aleph_primitives::{ApiError, AuthorityId};
// use sc_block_builder::{BlockBuilder, BlockBuilderProvider, RecordProof};
use sc_client_api::blockchain::{BlockStatus, CachedHeaderMetadata, Error, Info};
use sc_client_api::{
    Backend, BlockchainEvents, ClientImportOperation, FinalityNotifications, Finalizer,
    HeaderBackend, ImportNotifications, LockImportRun, StorageEventStream, StorageKey,
    TransactionFor,
};
use sc_consensus::{BlockCheckParams, BlockImport, BlockImportParams, ImportResult};
use sp_blockchain::HeaderMetadata;
use sp_consensus::{BlockOrigin, CacheKeyId};
use sp_runtime::traits::{Block as BlockT, DigestFor};
use std::collections::{BTreeMap, HashMap};
use std::ops::Bound::{Excluded, Included, Unbounded};

use crate::{ProvideAlephSessionApi, SessionPeriod};
// use sp_consensus::Error as ConsensusError;

use async_trait::async_trait;

pub struct TestClient<C, BE, B: BlockT> {
    client: C,
    authority_schedule: BTreeMap<NumberFor<B>, Vec<AuthorityId>>,
    session_period: SessionPeriod,
    millisecs_per_block: MillisecsPerBlock,
    _phantom: PhantomData<BE>,
}

unsafe impl<C, BE, B: BlockT> Send for TestClient<C, BE, B> {}
unsafe impl<C, BE, B: BlockT> Sync for TestClient<C, BE, B> {}

impl<C, BE, B: BlockT> TestClient<C, BE, B> {
    pub fn new(
        client: C,
        authority_schedule: BTreeMap<NumberFor<B>, Vec<AuthorityId>>,
        session_period: SessionPeriod,
        millisecs_per_block: MillisecsPerBlock,
    ) -> Self {
        Self {
            client,
            authority_schedule,
            session_period,
            millisecs_per_block,
            _phantom: PhantomData,
        }
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: LockImportRun<B, BE>> LockImportRun<B, BE>
    for TestClient<C, BE, B>
{
    fn lock_import_and_run<R, Err, F>(&self, f: F) -> Result<R, Err>
    where
        F: FnOnce(&mut ClientImportOperation<B, BE>) -> Result<R, Err>,
        Err: From<Error>,
    {
        self.client.lock_import_and_run(f)
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: Finalizer<B, BE>> Finalizer<B, BE>
    for TestClient<C, BE, B>
{
    fn apply_finality(
        &self,
        operation: &mut ClientImportOperation<B, BE>,
        id: BlockId<B>,
        justification: Option<Justification>,
        notify: bool,
    ) -> sp_blockchain::Result<()> {
        self.client
            .apply_finality(operation, id, justification, notify)
    }

    fn finalize_block(
        &self,
        id: BlockId<B>,
        justification: Option<Justification>,
        notify: bool,
    ) -> sp_blockchain::Result<()> {
        self.client.finalize_block(id, justification, notify)
    }
}

impl<B: BlockT, BE: sc_client_api::Backend<B>, C: ProvideRuntimeApi<B>> ProvideRuntimeApi<B>
    for TestClient<C, BE, B>
{
    type Api = C::Api;

    fn runtime_api(&self) -> ApiRef<Self::Api> {
        self.client.runtime_api()
    }
}

#[async_trait]
impl<
        B: BlockT,
        BE: sc_client_api::Backend<B>,
        C: BlockImport<B, Transaction = TransactionFor<BE, B>, Error = sp_consensus::Error> + Send,
    > BlockImport<B> for TestClient<C, BE, B>
where
    C::Transaction: 'static,
{
    type Error = C::Error;
    type Transaction = C::Transaction;

    async fn check_block(
        &mut self,
        block: BlockCheckParams<B>,
    ) -> Result<ImportResult, Self::Error> {
        self.client.check_block(block).await
    }

    async fn import_block(
        &mut self,
        block: BlockImportParams<B, Self::Transaction>,
        cache: HashMap<CacheKeyId, Vec<u8>>,
    ) -> Result<ImportResult, Self::Error> {
        self.client.import_block(block, cache).await
    }
}

impl<B: BlockT, BE: Sync + Send, C: HeaderBackend<B>> HeaderBackend<B> for TestClient<C, BE, B> {
    fn header(&self, id: BlockId<B>) -> sp_blockchain::Result<Option<<B as BlockT>::Header>> {
        self.client.header(id)
    }

    fn info(&self) -> Info<B> {
        self.client.info()
    }

    fn status(&self, id: BlockId<B>) -> sp_blockchain::Result<BlockStatus> {
        self.client.status(id)
    }

    fn number(
        &self,
        hash: <B as BlockT>::Hash,
    ) -> sp_blockchain::Result<Option<<<B as BlockT>::Header as HeaderT>::Number>> {
        self.client.number(hash)
    }

    fn hash(&self, number: NumberFor<B>) -> sp_blockchain::Result<Option<<B as BlockT>::Hash>> {
        self.client.hash(number)
    }
}

impl<B: BlockT, C: HeaderMetadata<B, Error = sp_blockchain::Error>, BE> HeaderMetadata<B>
    for TestClient<C, BE, B>
{
    type Error = C::Error;

    fn header_metadata(
        &self,
        hash: <B as BlockT>::Hash,
    ) -> Result<CachedHeaderMetadata<B>, Self::Error> {
        self.client.header_metadata(hash)
    }

    fn insert_header_metadata(
        &self,
        hash: <B as BlockT>::Hash,
        header_metadata: CachedHeaderMetadata<B>,
    ) {
        self.client.insert_header_metadata(hash, header_metadata)
    }

    fn remove_header_metadata(&self, hash: <B as BlockT>::Hash) {
        self.client.remove_header_metadata(hash)
    }
}

impl<B: BlockT, C: BlockchainEvents<B>, BE> BlockchainEvents<B> for TestClient<C, BE, B> {
    fn import_notification_stream(&self) -> ImportNotifications<B> {
        self.client.import_notification_stream()
    }

    fn finality_notification_stream(&self) -> FinalityNotifications<B> {
        self.client.finality_notification_stream()
    }

    #[allow(clippy::type_complexity)]
    fn storage_changes_notification_stream(
        &self,
        filter_keys: Option<&[StorageKey]>,
        child_filter_keys: Option<&[(StorageKey, Option<Vec<StorageKey>>)]>,
    ) -> sp_blockchain::Result<StorageEventStream<<B as BlockT>::Hash>> {
        self.client
            .storage_changes_notification_stream(filter_keys, child_filter_keys)
    }
}

impl<B: BlockT, C: HeaderBackend<B>, BE> ProvideAlephSessionApi<B> for TestClient<C, BE, B>
where
    BE: Backend<B>,
    B: BlockT,
{
    fn next_session_authorities(
        &self,
        block_id: &BlockId<B>,
    ) -> Result<Result<Vec<AuthorityId>, ApiError>, sp_api::ApiError> {
        let number = match block_id {
            BlockId::Hash(hash) => self.client.number(*hash).unwrap().unwrap(),
            BlockId::Number(number) => *number,
        };
        Ok(Ok(self
            .authority_schedule
            .range((Unbounded, Included(number)))
            .last()
            .map(|(_, authorities)| authorities.clone())
            .unwrap_or(Vec::<AuthorityId>::new())))
    }

    fn authorities(&self, block_id: &BlockId<B>) -> Result<Vec<AuthorityId>, sp_api::ApiError> {
        let number = match block_id {
            BlockId::Hash(hash) => self.client.number(*hash).unwrap().unwrap(),
            BlockId::Number(number) => *number,
        };
        Ok(self
            .authority_schedule
            .range((Unbounded, Included(number)))
            .last()
            .map(|(_, authorities)| authorities.clone())
            .unwrap_or(Vec::<AuthorityId>::new()))
    }

    fn session_period(&self, _block_id: &BlockId<B>) -> Result<SessionPeriod, sp_api::ApiError> {
        Ok(self.session_period)
    }

    fn millisecs_per_block(
        &self,
        _block_id: &BlockId<B>,
    ) -> Result<MillisecsPerBlock, sp_api::ApiError> {
        Ok(self.millisecs_per_block)
    }
}
