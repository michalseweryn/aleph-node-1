use crate::{
    network::{
        AuthData, ConsensusNetwork, DataNetwork, InternalMessage, MetaMessage, Network, PeerId,
        Recipient,
    },
    AuthorityId, AuthorityKeystore, KeyBox, MultiKeychain, SessionId, KEY_TYPE,
};
use aleph_bft::{Index, KeyBox as _, NodeIndex};
use codec::DecodeAll;
use futures::{
    channel::{mpsc, oneshot},
    future::{try_join_all, TryJoinAll},
    stream::Stream,
    StreamExt,
};
use parking_lot::Mutex;
use sc_network::{Event, ObservedRole, PeerId as ScPeerId, ReputationChange};
use sp_api::NumberFor;
use sp_core::Encode;
use sp_keystore::{testing::KeyStore, CryptoStore};
use sp_runtime::traits::Block as BlockT;
use std::{borrow::Cow, pin::Pin, sync::Arc};
use substrate_test_runtime::Block;

#[derive(Clone)]
pub(crate) struct TestNetwork<B: BlockT> {
    event_sinks: Arc<Mutex<Vec<mpsc::UnboundedSender<Event>>>>,
    oneshot_sender: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    report_peer: mpsc::UnboundedSender<(PeerId, ReputationChange)>,
    disconnect_peer: mpsc::UnboundedSender<(PeerId, Cow<'static, str>)>,
    send_message: mpsc::UnboundedSender<(PeerId, Cow<'static, str>, Vec<u8>)>,
    announce: mpsc::UnboundedSender<(B::Hash, Option<Vec<u8>>)>,
    add_set_reserved: mpsc::UnboundedSender<(PeerId, Cow<'static, str>)>,
    remove_set_reserved: mpsc::UnboundedSender<(PeerId, Cow<'static, str>)>,
    request_justification: mpsc::UnboundedSender<(B::Hash, NumberFor<B>)>,
    peer_id: PeerId,
}

pub(crate) struct TestNetworkReceivers<B: BlockT> {
    report_peer: mpsc::UnboundedReceiver<(PeerId, ReputationChange)>,
    disconnect_peer: mpsc::UnboundedReceiver<(PeerId, Cow<'static, str>)>,
    send_message: mpsc::UnboundedReceiver<(PeerId, Cow<'static, str>, Vec<u8>)>,
    announce: mpsc::UnboundedReceiver<(B::Hash, Option<Vec<u8>>)>,
    add_set_reserved: mpsc::UnboundedReceiver<(PeerId, Cow<'static, str>)>,
    remove_set_reserved: mpsc::UnboundedReceiver<(PeerId, Cow<'static, str>)>,
    request_justification: mpsc::UnboundedReceiver<(B::Hash, NumberFor<B>)>,
}

impl<B: BlockT> TestNetwork<B> {
    pub(crate) fn new(peer_id: PeerId, tx: oneshot::Sender<()>) -> (Self, TestNetworkReceivers<B>) {
        let report_peer = mpsc::unbounded();
        let disconnect_peer =  mpsc::unbounded();
        let send_message = mpsc::unbounded();
        let announce = mpsc::unbounded();
        let add_set_reserved = mpsc::unbounded();
        let remove_set_reserved = mpsc::unbounded();
        let request_justification = mpsc::unbounded();
        let network = TestNetwork {
            event_sinks: Arc::new(Mutex::new(vec![])),
            oneshot_sender: Arc::new(Mutex::new(Some(tx))),
            report_peer: report_peer.0,
            disconnect_peer: disconnect_peer.0,
            send_message: send_message.0,
            announce: announce.0,
            add_set_reserved: add_set_reserved.0,
            remove_set_reserved: remove_set_reserved.0,
            request_justification: request_justification.0,
            peer_id,
        };
        let receivers = TestNetworkReceivers {
            report_peer: report_peer.1,
            disconnect_peer: disconnect_peer.1,
            send_message: send_message.1,
            announce: announce.1,
            add_set_reserved: add_set_reserved.1,
            remove_set_reserved: remove_set_reserved.1,
            request_justification: request_justification.1,
        };
        (network, receivers)
    }
}

impl<B: BlockT> TestNetworkReceivers<B> {
    fn close(mut self) {
        assert!(self.report_peer.try_next().unwrap().is_none());
        assert!(self.disconnect_peer.try_next().unwrap().is_none());
        assert!(self.send_message.try_next().unwrap().is_none());
        assert!(self.announce.try_next().unwrap().is_none());
        assert!(self.add_set_reserved.try_next().unwrap().is_none());
        assert!(self.remove_set_reserved.try_next().unwrap().is_none());
        assert!(self.request_justification.try_next().unwrap().is_none());
    }
}


impl<B: BlockT> Network<B> for TestNetwork<B> {
    fn event_stream(&self) -> Pin<Box<dyn Stream<Item = Event> + Send>> {
        let (tx, rx) = mpsc::unbounded();
        self.event_sinks.lock().push(tx);
        if let Some(tx) = self.oneshot_sender.lock().take() {
            tx.send(()).unwrap();
        }
        Box::pin(rx)
    }

    fn _report_peer(&self, peer_id: PeerId, reputation: ReputationChange) {
        self.report_peer
            .unbounded_send((peer_id, reputation))
            .unwrap();
    }

    fn _disconnect_peer(&self, peer_id: PeerId, protocol: Cow<'static, str>) {
        self.disconnect_peer
            .unbounded_send((peer_id, protocol))
            .unwrap();
    }

    fn send_message(&self, peer_id: PeerId, protocol: Cow<'static, str>, message: Vec<u8>) {
        self.send_message
            .unbounded_send((peer_id, protocol, message))
            .unwrap();
    }

    fn _announce(&self, block: <B as BlockT>::Hash, associated_data: Option<Vec<u8>>) {
        self.announce
            .unbounded_send((block, associated_data))
            .unwrap();
    }

    fn add_set_reserved(&self, who: PeerId, protocol: Cow<'static, str>) {
        self.add_set_reserved
            .unbounded_send((who, protocol))
            .unwrap();
    }

    fn remove_set_reserved(&self, who: PeerId, protocol: Cow<'static, str>) {
        self.remove_set_reserved
            .unbounded_send((who, protocol))
            .unwrap();
    }

    fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    fn request_justification(&self, hash: &B::Hash, number: NumberFor<B>) {
        self.request_justification
            .unbounded_send((*hash, number))
            .unwrap();
    }
}

impl<B: BlockT> TestNetwork<B> {
    fn emit_event(&self, event: Event) {
        for sink in &*self.event_sinks.lock() {
            sink.unbounded_send(event.clone()).unwrap();
        }
    }

    // Consumes the network asserting there are no unreceived messages in the channels.
    fn close_channels(self) {
        self.event_sinks.lock().clear();
        self.report_peer.close_channel();
        self.disconnect_peer.close_channel();
        self.send_message.close_channel();
        self.announce.close_channel();
        self.add_set_reserved.close_channel();
        self.remove_set_reserved.close_channel();
    }
}

// A hub which mediates between instances of TestNetwork
pub(crate) struct TestNetworkHub<B: BlockT> {
    protocol_name: Cow<'static, str>,
    networks: Vec<TestNetwork<B>>,
    receivers: Vec<TestNetworkReceivers<B>>,
    all_event_streams_created: TryJoinAll<oneshot::Receiver<()>>,
    peer_ids: Vec<PeerId>,
}

impl<B: BlockT> TestNetworkHub<B> {
    pub(crate) fn new(protocol_name: Cow<'static, str>, size: usize) -> Self {
        let mut networks = Vec::with_capacity(size);
        let mut all_receivers = Vec::with_capacity(size);
        let mut rxs = Vec::with_capacity(size);
        let mut peer_ids = Vec::with_capacity(size);
        for _ in 0..size {
            let peer_id = ScPeerId::random().into();
            let (tx, rx) = oneshot::channel();
            let (network, receivers) = TestNetwork::new(peer_id, tx);
            networks.push(network);
            all_receivers.push(receivers);
            rxs.push(rx);
            peer_ids.push(peer_id);
        }
        Self {
            protocol_name,
            networks,
            receivers: all_receivers,
            all_event_streams_created: try_join_all(rxs),
            peer_ids }
    }

    pub(crate) fn network(&self, i: usize) -> TestNetwork<B> {
        self.networks[i].clone()
    }

    pub(crate) async fn run(mut self) {
        // wait until an event stream is created for every network
        self.all_event_streams_created.await.expect("all networks should send confirmations");
        // initialize connections for the networks
        for (i, network) in self.networks.iter().enumerate() {
            for (j, peer_id) in self.peer_ids.iter().enumerate() {
                if j != i {
                    network.emit_event(Event::SyncConnected {
                        remote: peer_id.clone().into()
                    })
                }
            }
        }
        use futures::future::select_all;
        loop {
            match select_all(self.receivers.iter_mut().map(|receivers| receivers.send_message.next())).await {
                (Some((recipient, protocol, data)), i, _) => {
                    assert_eq!(protocol, self.protocol_name);
                    let m = InternalMessage::<MockData>::decode_all(data.as_slice()).expect("a correct message");
                    let j = self
                        .peer_ids
                        .iter()
                        .position(|peer_id| *peer_id == recipient)
                        .expect("message should be sent to a known peer_id");
                    self.networks[j].emit_event(Event::NotificationsReceived {
                        remote: self.peer_ids[i].into(),
                        messages: vec![(self.protocol_name.clone(), data.into())]
                    })
                }
                (None, i, _) => {
                    panic!("the message stream of network {} ended", i)
                }
            }
            panic!("message was actually sent");
        }
    }

}

struct Authority {
    peer_id: PeerId,
    keychain: MultiKeychain,
}

async fn generate_authorities(ss: &[String]) -> Vec<Authority> {
    let key_store = Arc::new(KeyStore::new());
    let mut auth_ids = Vec::with_capacity(ss.len());
    for s in ss {
        let pk = key_store
            .ed25519_generate_new(KEY_TYPE, Some(s))
            .await
            .unwrap();
        auth_ids.push(AuthorityId::from(pk));
    }
    let mut result = Vec::with_capacity(ss.len());
    for i in 0..ss.len() {
        let keybox = KeyBox {
            id: NodeIndex(i),
            auth_keystore: AuthorityKeystore::new(auth_ids[i].clone(), key_store.clone()),
            authorities: auth_ids.clone(),
        };
        let keychain = MultiKeychain::new(keybox);
        result.push(Authority {
            peer_id: ScPeerId::random().into(),
            keychain,
        });
    }
    assert_eq!(key_store.keys(KEY_TYPE).await.unwrap().len(), 3 * ss.len());
    result
}

type MockData = Vec<u8>;

struct TestData {
    network: TestNetwork<Block>,
    authorities: Vec<Authority>,
    consensus_network_handle: tokio::task::JoinHandle<()>,
    data_network: DataNetwork<MockData>,
    receivers: TestNetworkReceivers<Block>,
}

impl TestData {
    // consumes the test data asserting there are no unread messages in the channels
    // and awaits for the consensus_network task.
    async fn complete(mut self) {
        self.network.close_channels();
        assert!(self.data_network.next().await.is_none());
        self.consensus_network_handle.await.unwrap();
        self.receivers.close();
    }
}

const PROTOCOL_NAME: &str = "/test/1";

async fn prepare_one_session_test_data() -> TestData {
    let authority_names: Vec<_> = ["//Alice", "//Bob", "//Charlie"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let authorities = generate_authorities(authority_names.as_slice()).await;
    let peer_id = authorities[0].peer_id;

    let (oneshot_tx, oneshot_rx) = oneshot::channel();
    let (network, receivers) = TestNetwork::<Block>::new(peer_id, oneshot_tx);
    let consensus_network = ConsensusNetwork::<MockData, Block, TestNetwork<Block>>::new(
        network.clone(),
        PROTOCOL_NAME.into(),
    );

    let session_id = SessionId(0);

    let data_network = consensus_network
        .session_manager()
        .start_session(session_id, authorities[0].keychain.clone())
        .await;
    let consensus_network_handle = tokio::spawn(async move { consensus_network.run().await });

    // wait till consensus_network takes the event_stream
    oneshot_rx.await.unwrap();

    TestData {
        network,
        authorities,
        consensus_network_handle,
        data_network,
        receivers,
    }
}

#[tokio::test]
async fn test_network_event_sync_connnected() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    data.network.emit_event(Event::SyncConnected {
        remote: bob_peer_id.into(),
    });
    let (peer_id, protocol) = data.receivers.add_set_reserved.next().await.unwrap();
    assert_eq!(peer_id, bob_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    data.complete().await;
}

#[tokio::test]
async fn test_network_event_sync_disconnected() {
    let mut data = prepare_one_session_test_data().await;
    let charlie_peer_id = data.authorities[2].peer_id;
    data.network.emit_event(Event::SyncDisconnected {
        remote: charlie_peer_id.into(),
    });
    let (peer_id, protocol) = data
        .receivers
        .remove_set_reserved
        .next()
        .await
        .unwrap();
    assert_eq!(peer_id, charlie_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    data.complete().await;
}

#[tokio::test]
async fn authenticates_to_connected() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    data.network.emit_event(Event::NotificationStreamOpened {
        remote: bob_peer_id.into(),
        protocol: Cow::Borrowed(PROTOCOL_NAME),
        role: ObservedRole::Authority,
        negotiated_fallback: None,
    });
    let (peer_id, protocol, message) = data
        .receivers
        .send_message
        .next()
        .await
        .expect("got auth message");
    let alice_peer_id = data.authorities[0].peer_id;
    assert_eq!(peer_id, bob_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    let message =
        InternalMessage::<MockData>::decode_all(message.as_slice()).expect("a correct message");
    if let InternalMessage::Meta(MetaMessage::Authentication(auth_data, _)) = message {
        assert_eq!(auth_data.peer_id, alice_peer_id);
    } else {
        panic!("Expected an authentication message.")
    }
    data.complete().await;
}

#[tokio::test]
async fn authenticates_when_requested() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    let auth_message =
        InternalMessage::<MockData>::Meta(MetaMessage::AuthenticationRequest(SessionId(0)))
            .encode();
    let messages = vec![(PROTOCOL_NAME.into(), auth_message.into())];

    data.network.emit_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    let (peer_id, protocol, message) = data
        .receivers
        .send_message
        .next()
        .await
        .expect("got auth message");
    let alice_peer_id = data.authorities[0].peer_id;
    assert_eq!(peer_id, bob_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    let message =
        InternalMessage::<MockData>::decode_all(message.as_slice()).expect("a correct message");
    if let InternalMessage::Meta(MetaMessage::Authentication(auth_data, _)) = message {
        assert_eq!(auth_data.peer_id, alice_peer_id);
    } else {
        panic!("Expected an authentication message.")
    }
    data.complete().await;
}

#[tokio::test]
async fn test_network_event_notifications_received() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    let bob_node_id = data.authorities[1].keychain.index();
    let auth_data = AuthData {
        session_id: SessionId(0),
        peer_id: bob_peer_id,
        node_id: bob_node_id,
    };
    let signature = data.authorities[1].keychain.sign(&auth_data.encode()).await;
    let auth_message =
        InternalMessage::<MockData>::Meta(MetaMessage::Authentication(auth_data, signature))
            .encode();
    let note = vec![157];
    let message = InternalMessage::Data(SessionId(0), note.clone()).encode();
    let messages = vec![
        (PROTOCOL_NAME.into(), auth_message.into()),
        (PROTOCOL_NAME.into(), message.clone().into()),
    ];

    data.network.emit_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    if let Some(incoming_data) = data.data_network.next().await {
        assert_eq!(incoming_data, note);
    } else {
        panic!("expected message received nothing")
    }
    data.complete().await;
}

#[tokio::test]
async fn requests_authentication_from_unauthenticated() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    let cur_session_id = SessionId(0);
    let note = vec![157];
    let message = InternalMessage::Data(cur_session_id, note).encode();
    let messages = vec![(PROTOCOL_NAME.into(), message.into())];

    data.network.emit_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    let (peer_id, protocol, message) = data
        .receivers
        .send_message
        .next()
        .await
        .expect("got auth request");
    assert_eq!(peer_id, bob_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    let message =
        InternalMessage::<MockData>::decode_all(message.as_slice()).expect("a correct message");
    if let InternalMessage::Meta(MetaMessage::AuthenticationRequest(session_id)) = message {
        assert_eq!(session_id, cur_session_id);
    } else {
        panic!("Expected an authentication request.")
    }
    data.complete().await;
}

#[tokio::test]
async fn test_send() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    let bob_node_id = data.authorities[1].keychain.index();
    let cur_session_id = SessionId(0);
    let auth_data = AuthData {
        session_id: cur_session_id,
        peer_id: bob_peer_id,
        node_id: bob_node_id,
    };
    let signature = data.authorities[1].keychain.sign(&auth_data.encode()).await;
    let auth_message =
        InternalMessage::<MockData>::Meta(MetaMessage::Authentication(auth_data, signature))
            .encode();
    let messages = vec![(PROTOCOL_NAME.into(), auth_message.into())];

    data.network.emit_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    data.network.emit_event(Event::NotificationStreamOpened {
        remote: bob_peer_id.into(),
        protocol: Cow::Borrowed(PROTOCOL_NAME),
        role: ObservedRole::Authority,
        negotiated_fallback: None,
    });
    // Wait for acknowledgement that Alice noted Bob's presence.
    data.receivers
        .send_message
        .next()
        .await
        .expect("got auth message");
    let note = vec![157];
    data.data_network
        .send(note.clone(), Recipient::Target(bob_node_id))
        .expect("sending works");
    match data.receivers.send_message.next().await {
        Some((peer_id, protocol, message)) => {
            assert_eq!(peer_id, bob_peer_id);
            assert_eq!(protocol, PROTOCOL_NAME);
            match InternalMessage::<MockData>::decode_all(message.as_slice()) {
                Ok(InternalMessage::Data(session_id, data)) => {
                    assert_eq!(session_id, cur_session_id);
                    assert_eq!(data, note);
                }
                _ => panic!("Expected a properly encoded message"),
            }
        }
        _ => panic!("Expecting a message"),
    }
    data.complete().await;
}

#[tokio::test]
async fn test_broadcast() {
    let mut data = prepare_one_session_test_data().await;
    let cur_session_id = SessionId(0);
    for i in 1..2 {
        let peer_id = data.authorities[i].peer_id;
        let node_id = data.authorities[i].keychain.index();
        let auth_data = AuthData {
            session_id: cur_session_id,
            peer_id,
            node_id,
        };
        let signature = data.authorities[1].keychain.sign(&auth_data.encode()).await;
        let auth_message =
            InternalMessage::<MockData>::Meta(MetaMessage::Authentication(auth_data, signature))
                .encode();
        let messages = vec![(PROTOCOL_NAME.into(), auth_message.into())];

        data.network.emit_event(Event::NotificationsReceived {
            remote: peer_id.0,
            messages,
        });
        data.network.emit_event(Event::NotificationStreamOpened {
            remote: peer_id.0,
            protocol: Cow::Borrowed(PROTOCOL_NAME),
            role: ObservedRole::Authority,
            negotiated_fallback: None,
        });
        // Wait for acknowledgement that Alice noted the nodes presence.
        data.receivers
            .send_message
            .next()
            .await
            .expect("got auth message");
    }
    let note = vec![157];
    data.data_network
        .send(note.clone(), Recipient::All)
        .expect("broadcasting works");
    for _ in 1..2_usize {
        match data.receivers.send_message.next().await {
            Some((_, protocol, message)) => {
                assert_eq!(protocol, PROTOCOL_NAME);
                match InternalMessage::<MockData>::decode_all(message.as_slice()) {
                    Ok(InternalMessage::Data(session_id, data)) => {
                        assert_eq!(session_id, cur_session_id);
                        assert_eq!(data, note);
                    }
                    _ => panic!("Expected a properly encoded message"),
                }
            }
            _ => panic!("Expecting a message"),
        }
    }
    data.complete().await;
}
