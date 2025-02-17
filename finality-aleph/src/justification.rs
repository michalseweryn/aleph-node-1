use crate::{
    finalization::finalize_block, last_block_of_session, network, session_id_from_block_num,
    KeyBox, SessionId, SessionMap, SessionPeriod, Signature,
};
use aleph_bft::{MultiKeychain, NodeIndex, SignatureSet};
use aleph_primitives::ALEPH_ENGINE_ID;
use codec::{Decode, Encode};
use futures::{channel::mpsc, StreamExt};
use futures_timer::Delay;
use log::{debug, error, warn};
use parking_lot::Mutex;
use sc_client_api::backend::Backend;
use sp_api::{BlockId, BlockT, NumberFor};
use sp_keystore::SyncCryptoStorePtr;
use sp_runtime::traits::Header;
use std::{
    marker::PhantomData,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::time::timeout;

#[derive(Clone, Encode, Decode, Debug)]
pub struct AlephJustification {
    pub(crate) signature: SignatureSet<Signature>,
}

impl AlephJustification {
    pub(crate) fn new<Block: BlockT>(signature: SignatureSet<Signature>) -> Self {
        Self { signature }
    }

    pub(crate) fn verify<
        Block: BlockT,
        MK: MultiKeychain<Signature = Signature, PartialMultisignature = SignatureSet<Signature>>,
    >(
        aleph_justification: &AlephJustification,
        block_hash: Block::Hash,
        multi_keychain: &MK,
    ) -> bool {
        if !multi_keychain.is_complete(&block_hash.encode()[..], &aleph_justification.signature) {
            debug!(target: "afa", "Bad justification for block hash #{:?} {:?}", block_hash, aleph_justification);
            return false;
        }
        true
    }
}

pub struct ChainCadence {
    pub session_period: SessionPeriod,
    pub justifications_cadence: Duration,
}

pub struct JustificationNotification<Block>
where
    Block: BlockT,
{
    pub justification: AlephJustification,
    pub hash: Block::Hash,
    pub number: NumberFor<Block>,
}

pub struct JustificationHandler<B, N, C, BE>
where
    B: BlockT,
    N: network::Network<B> + 'static,
    C: crate::ClientForAleph<B, BE> + Send + Sync + 'static,
    BE: Backend<B> + 'static,
{
    session_authorities: Arc<Mutex<SessionMap>>,
    keystore: SyncCryptoStorePtr,
    chain_cadence: ChainCadence,
    network: N,
    client: Arc<C>,
    last_request_time: Instant,
    last_finalization_time: Instant,
    phantom: PhantomData<(B, BE)>,
}

impl<B, N, C, BE> JustificationHandler<B, N, C, BE>
where
    B: BlockT,
    N: network::Network<B> + 'static,
    C: crate::ClientForAleph<B, BE> + Send + Sync + 'static,
    BE: Backend<B> + 'static,
{
    pub(crate) fn new(
        session_authorities: Arc<Mutex<SessionMap>>,
        keystore: SyncCryptoStorePtr,
        chain_cadence: ChainCadence,
        network: N,
        client: Arc<C>,
    ) -> Self {
        Self {
            session_authorities,
            keystore,
            chain_cadence,
            network,
            client,
            last_request_time: Instant::now(),
            last_finalization_time: Instant::now(),
            phantom: PhantomData,
        }
    }

    pub(crate) fn handle_justification_notification(
        &mut self,
        notification: JustificationNotification<B>,
        keybox: KeyBox,
        last_finalized: NumberFor<B>,
        stop_h: NumberFor<B>,
    ) {
        let num = notification.number;
        let block_hash = notification.hash;
        if AlephJustification::verify::<B, _>(
            &notification.justification,
            notification.hash,
            &crate::MultiKeychain::new(keybox),
        ) {
            if num > last_finalized && num <= stop_h {
                debug!(target: "afa", "Finalizing block {:?} {:?}", num, block_hash);
                let finalization_res = finalize_block(
                    self.client.clone(),
                    block_hash,
                    num,
                    Some((ALEPH_ENGINE_ID, notification.justification.encode())),
                );
                match finalization_res {
                    Ok(()) => {
                        self.last_finalization_time = Instant::now();
                        debug!(target: "afa", "Successfully finalized {:?}", num);
                    }
                    Err(e) => {
                        warn!(target: "afa", "Fail in finalization of {:?} {:?} -- {:?}", num, block_hash, e);
                    }
                }
            } else {
                debug!(target: "afa", "Not finalizing block {:?}. Last finalized {:?}, stop_h {:?}", num, last_finalized, stop_h);
            }
        } else {
            error!(target: "afa", "Error when verifying justification for block {:?} {:?}", num, block_hash);
        }
    }

    pub(crate) fn request_justification(&mut self, num: NumberFor<B>) {
        let current_time = Instant::now();

        let ChainCadence {
            justifications_cadence,
            ..
        } = self.chain_cadence;

        if current_time - self.last_finalization_time > justifications_cadence
            && current_time - self.last_request_time > 2 * justifications_cadence
        {
            debug!(target: "afa", "Trying to request block {:?}", num);

            if let Ok(Some(header)) = self.client.header(BlockId::Number(num)) {
                debug!(target: "afa", "We have block {:?} with hash {:?}. Requesting justification.", num, header.hash());
                self.last_request_time = current_time;
                self.network
                    .request_justification(&header.hash(), *header.number());
            } else {
                debug!(target: "afa", "Cancelling request, because we don't have block {:?}.", num);
            }
        }
    }

    pub(crate) async fn run(
        mut self,
        authority_justification_rx: mpsc::UnboundedReceiver<JustificationNotification<B>>,
        import_justification_rx: mpsc::UnboundedReceiver<JustificationNotification<B>>,
    ) {
        let import_stream = import_justification_rx
            .inspect(|_| {
                debug!(target: "afa", "Got justification (import)");
            })
            .chain(futures::stream::iter(std::iter::from_fn(|| {
                error!(target: "afa", "Justification (import) stream ended.");
                None
            })));

        let authority_stream = authority_justification_rx
            .inspect(|_| {
                debug!(target: "afa", "Got justification (aggregator)");
            })
            .chain(futures::stream::iter(std::iter::from_fn(|| {
                error!(target: "afa", "Justification (aggregator) stream ended.");
                None
            })));

        let mut notification_stream = futures::stream::select(import_stream, authority_stream);

        let ChainCadence { session_period, .. } = self.chain_cadence;

        loop {
            let last_finalized_number = self.client.info().finalized_number;
            let current_session =
                session_id_from_block_num::<B>(last_finalized_number + 1u32.into(), session_period);
            let stop_h: NumberFor<B> = last_block_of_session::<B>(current_session, session_period);
            let keybox = self.session_keybox(current_session);
            if keybox.is_none() {
                debug!(target: "afa", "Keybox for session {:?} not yet available. Waiting 500ms and will try again ...", current_session);
                Delay::new(Duration::from_millis(500)).await;
                continue;
            }
            let keybox = keybox.expect("We loop until this is some.");

            match timeout(Duration::from_millis(1000), notification_stream.next()).await {
                Ok(Some(notification)) => {
                    self.handle_justification_notification(
                        notification,
                        keybox,
                        last_finalized_number,
                        stop_h,
                    );
                }
                Ok(None) => {
                    error!(target: "afa", "Justification stream ended.");
                    return;
                }
                Err(_) => {
                    //Timeout passed
                }
            }

            self.request_justification(stop_h);
        }
    }

    fn session_keybox(&self, session_id: SessionId) -> Option<KeyBox> {
        let authorities = match self.session_authorities.lock().get(&session_id) {
            Some(authorities) => authorities.to_vec(),
            None => return None,
        };
        // Below we use a fake index 0 of a node -- we never use this keybox for signing so that's OK.
        // TODO: consider refactoring this in the future so that verification and signing are separate (not combined within a KeyBox).
        Some(KeyBox::new(
            NodeIndex(0),
            authorities,
            self.keystore.clone(),
        ))
    }
}
