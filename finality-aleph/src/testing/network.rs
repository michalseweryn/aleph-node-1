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
    channel::{mpsc},
    stream::{Stream, SelectAll},
    StreamExt,
};
use sc_network::{Event, ObservedRole, PeerId as ScPeerId, ReputationChange};
use sp_api::NumberFor;
use sp_core::Encode;
use sp_keystore::{testing::KeyStore, CryptoStore};
use sp_runtime::traits::Block as BlockT;
use std::{borrow::Cow, pin::Pin, sync::Arc};
use std::collections::HashMap;
use substrate_test_runtime::Block;

#[derive(Debug)]
enum TestNetworkCommand<B: BlockT> {
    ReportPeer(PeerId, ReputationChange),
    DisconnectPeer(PeerId, Cow<'static, str>),
    SendMessage(PeerId, Cow<'static, str>, Vec<u8>),
    Announce(B::Hash, Option<Vec<u8>>),
    AddSetReserved(PeerId, Cow<'static, str>),
    RemoveSetReserved(PeerId, Cow<'static, str>),
    RequestJustification(B::Hash, NumberFor<B>),
}

impl<B: BlockT> TestNetworkCommand<B> {
    fn unwrap_send_message(self) -> (PeerId, Cow<'static, str>, Vec<u8>) {
        match self {
            TestNetworkCommand::SendMessage(peer_id, protocol, data) => (peer_id, protocol, data),
            _ => panic!("called `TestNetworkCommand::unwrap_send_message()` on `{:?}`", self)
        }
    }

    fn unwrap_add_set_reserved(self) -> (PeerId, Cow<'static, str>) {
        match self {
            TestNetworkCommand::AddSetReserved(peer_id, protocol) => (peer_id, protocol),
            _ => panic!("called `TestNetworkCommand::unwrap_add_set_reserved()` on `{:?}`", self)
        }
    }

    fn unwrap_remove_set_reserved(self) -> (PeerId, Cow<'static, str>) {
        match self {
            TestNetworkCommand::RemoveSetReserved(peer_id, protocol) => (peer_id, protocol),
            _ => panic!("called `TestNetworkCommand::unwrap_remove_set_reserved()` on `{:?}`", self)
        }
    }
}

type EventSink = mpsc::UnboundedSender<Event>;

#[derive(Clone)]
struct TestNetwork<B: BlockT> {
    event_sink_tx: mpsc::UnboundedSender<EventSink>,
    command_tx: mpsc::UnboundedSender<TestNetworkCommand<B>>,
    peer_id: PeerId,
}

struct MemoizingReceiver<T> {
    data: Vec<T>,
    rx: mpsc::UnboundedReceiver<T>,
}

impl<T> MemoizingReceiver<T> {
    fn new(rx: mpsc::UnboundedReceiver<T>) -> Self {
        MemoizingReceiver {
            data: Vec::new(),
            rx
        }
    }
    fn try_fetch_many(&mut self) {
        while let Ok(Some(item)) = self.rx.try_next() {
            self.data.push(item);
        }
    }

    // await till at least one element is received
    async fn first(&mut self) {
        if !self.data.is_empty() {
            return;
        }
        let item = self.rx.next().await.unwrap();
        self.data.push(item);
    }

    fn iter(&mut self) -> impl Iterator<Item=&T> {
        self.try_fetch_many();
        self.data.iter()
    }
}

struct TestNetworkReceivers<B: BlockT> {
    event_sinks: MemoizingReceiver<EventSink>,
    command_rx: mpsc::UnboundedReceiver<TestNetworkCommand<B>>,
}

impl<B: BlockT> TestNetwork<B> {
    fn new(peer_id: PeerId) -> (Self, TestNetworkReceivers<B>) {
        let (event_sink_tx, event_sink_rx) = mpsc::unbounded();
        let (command_tx, command_rx) = mpsc::unbounded();
        let network = TestNetwork {
            event_sink_tx,
            command_tx,
            peer_id,
        };
        let receivers = TestNetworkReceivers {
            event_sinks: MemoizingReceiver::new(event_sink_rx),
            command_rx,
        };
        (network, receivers)
    }
}

impl<B: BlockT> Network<B> for TestNetwork<B> {
    fn event_stream(&self) -> Pin<Box<dyn Stream<Item = Event> + Send>> {
        let (tx, rx) = mpsc::unbounded();
        self.event_sink_tx.unbounded_send(tx).unwrap();
        Box::pin(rx)
    }

    fn _report_peer(&self, peer_id: PeerId, reputation: ReputationChange) {
        self.command_tx
            .unbounded_send(TestNetworkCommand::ReportPeer(peer_id, reputation))
            .unwrap();
    }

    fn _disconnect_peer(&self, peer_id: PeerId, protocol: Cow<'static, str>) {
        self.command_tx
            .unbounded_send(TestNetworkCommand::DisconnectPeer(peer_id, protocol))
            .unwrap();
    }

    fn send_message(&self, peer_id: PeerId, protocol: Cow<'static, str>, message: Vec<u8>) {
        self.command_tx
            .unbounded_send(TestNetworkCommand::SendMessage(peer_id, protocol, message))
            .unwrap();
    }

    fn _announce(&self, block: <B as BlockT>::Hash, associated_data: Option<Vec<u8>>) {
        self.command_tx.unbounded_send(TestNetworkCommand::Announce(block, associated_data))
            .unwrap();
    }

    fn add_set_reserved(&self, who: PeerId, protocol: Cow<'static, str>) {
        self.command_tx
            .unbounded_send(TestNetworkCommand::AddSetReserved(who, protocol))
            .unwrap();
    }

    fn remove_set_reserved(&self, who: PeerId, protocol: Cow<'static, str>) {
        self.command_tx
            .unbounded_send(TestNetworkCommand::RemoveSetReserved(who, protocol))
            .unwrap();
    }

    fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    fn request_justification(&self, hash: &B::Hash, number: NumberFor<B>) {
        self.command_tx
            .unbounded_send(TestNetworkCommand::RequestJustification(*hash, number))
            .unwrap();
    }
}

impl<B: BlockT> TestNetworkReceivers<B> {
    fn send_event(&mut self, event: Event) {
        for event_sink in self.event_sinks.iter() {
            event_sink.unbounded_send(event.clone()).unwrap()
        }
    }
    async fn next_command(&mut self) -> TestNetworkCommand<B> {
        self.command_rx.next().await.unwrap()
    }
    async fn first_event_sink(&mut self) {
        self.event_sinks.first().await;
    }
}

pub(crate) struct TestNetworkHub<B: BlockT> {
    all_receivers: HashMap<PeerId, TestNetworkReceivers<B>>,
}

impl<B: BlockT> TestNetworkHub<B> {
    pub(crate) fn new(n: usize) -> (Self, Vec<TestNetwork<B>>) {
        let peer_ids : Vec<PeerId> = (0..n).map(|_| ScPeerId::random().into()).collect();
        let (networks, all_receivers) = peer_ids.iter().cloned().map(|peer_id| {
            let (network, receivers) = TestNetwork::new(peer_id);
            (network, (peer_id, receivers))
        }).unzip();
        let hub = TestNetworkHub {
            all_receivers,
        };
        (hub, networks)
    }
    pub(crate) async fn run(self) {
        let (mut all_event_sinks, mut peers_commands) : (HashMap<_, _>, SelectAll<_>) =
            self.all_receivers
                .into_iter()
                .map(|(peer_id, receivers)| {
                    (
                        (peer_id, receivers.event_sinks),
                        receivers.command_rx.map(move |cmd| (peer_id.clone(), cmd))
                    )
                }).unzip();
        while let Some((sender, cmd))  = peers_commands.next().await {
            match cmd {
                TestNetworkCommand::SendMessage(recipient, protocol, data) => {
                    for event_sink in all_event_sinks.get_mut(&recipient).unwrap().iter() {
                        event_sink.unbounded_send(Event::NotificationsReceived {
                            remote: sender.into(),
                            messages: vec![(protocol.clone(), data.clone().into())]
                        }).unwrap();
                    }
                }
                _ => {}
            }
        }
    }

}

struct Authority {
    peer_id: PeerId,
    keychain: MultiKeychain
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
    let mut authorities = Vec::with_capacity(ss.len());
    for i in 0..ss.len() {
        let keybox = KeyBox {
            id: NodeIndex(i),
            auth_keystore: AuthorityKeystore::new(auth_ids[i].clone(), key_store.clone()),
            authorities: auth_ids.clone(),
        };
        let keychain = MultiKeychain::new(keybox);
        authorities.push(Authority { peer_id: ScPeerId::random().into(), keychain });
    }
    assert_eq!(key_store.keys(KEY_TYPE).await.unwrap().len(), 3 * ss.len());
    authorities
}

type MockData = Vec<u8>;

struct TestData {
    network: TestNetwork<Block>,
    receivers: TestNetworkReceivers<Block>,
    authorities: Vec<Authority>,
    _consensus_network_handle: tokio::task::JoinHandle<()>,
    data_network: DataNetwork<MockData>,
}


const PROTOCOL_NAME: &str = "/test/1";

async fn prepare_one_session_test_data() -> TestData {
    let authority_names: Vec<_> = ["//Alice", "//Bob", "//Charlie"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let authorities = generate_authorities(authority_names.as_slice()).await;
    let peer_id = authorities[0].peer_id;

    let (network, mut receivers) = TestNetwork::<Block>::new(peer_id);
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
    receivers.first_event_sink().await;

    TestData {
        network,
        receivers,
        authorities,
        _consensus_network_handle: consensus_network_handle,
        data_network,
    }
}

#[tokio::test]
async fn test_network_event_sync_connnected() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    data.receivers.send_event(Event::SyncConnected {
        remote: bob_peer_id.into(),
    });
    let (peer_id, protocol) = data.receivers.next_command().await.unwrap_add_set_reserved();
    assert_eq!(peer_id, bob_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    assert!(data.receivers.command_rx.try_next().is_err());
}

#[tokio::test]
async fn test_network_event_sync_disconnected() {
    let mut data = prepare_one_session_test_data().await;
    let charlie_peer_id = data.authorities[2].peer_id;
    data.receivers.send_event(Event::SyncDisconnected {
        remote: charlie_peer_id.into(),
    });
    let (peer_id, protocol) = data.receivers.next_command().await.unwrap_remove_set_reserved();
    assert_eq!(peer_id, charlie_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    assert!(data.receivers.command_rx.try_next().is_err());
}

#[tokio::test]
async fn authenticates_to_connected() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    data.receivers.send_event(Event::NotificationStreamOpened {
        remote: bob_peer_id.into(),
        protocol: Cow::Borrowed(PROTOCOL_NAME),
        role: ObservedRole::Authority,
        negotiated_fallback: None,
    });
    let (peer_id, protocol, message) = data.receivers.next_command().await.unwrap_send_message();
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
    assert!(data.receivers.command_rx.try_next().is_err());
}

#[tokio::test]
async fn authenticates_when_requested() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    let auth_message =
        InternalMessage::<MockData>::Meta(MetaMessage::AuthenticationRequest(SessionId(0)))
            .encode();
    let messages = vec![(PROTOCOL_NAME.into(), auth_message.into())];

    data.receivers.send_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    let (peer_id, protocol, message) = data.receivers.next_command().await.unwrap_send_message();
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
    assert!(data.receivers.command_rx.try_next().is_err());
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

    data.receivers.send_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    if let Some(incoming_data) = data.data_network.next().await {
        assert_eq!(incoming_data, note);
    } else {
        panic!("expected message received nothing")
    }
    assert!(data.receivers.command_rx.try_next().is_err());
}

#[tokio::test]
async fn requests_authentication_from_unauthenticated() {
    let mut data = prepare_one_session_test_data().await;
    let bob_peer_id = data.authorities[1].peer_id;
    let cur_session_id = SessionId(0);
    let note = vec![157];
    let message = InternalMessage::Data(cur_session_id, note).encode();
    let messages = vec![(PROTOCOL_NAME.into(), message.into())];

    data.receivers.send_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    let (peer_id, protocol, message) = data.receivers.next_command().await.unwrap_send_message();
    assert_eq!(peer_id, bob_peer_id);
    assert_eq!(protocol, PROTOCOL_NAME);
    let message =
        InternalMessage::<MockData>::decode_all(message.as_slice()).expect("a correct message");
    if let InternalMessage::Meta(MetaMessage::AuthenticationRequest(session_id)) = message {
        assert_eq!(session_id, cur_session_id);
    } else {
        panic!("Expected an authentication request.")
    }
    assert!(data.receivers.command_rx.try_next().is_err());
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

    data.receivers.send_event(Event::NotificationsReceived {
        remote: bob_peer_id.into(),
        messages,
    });
    data.receivers.send_event(Event::NotificationStreamOpened {
        remote: bob_peer_id.into(),
        protocol: Cow::Borrowed(PROTOCOL_NAME),
        role: ObservedRole::Authority,
        negotiated_fallback: None,
    });
    // Wait for acknowledgement that Alice noted Bob's presence.
    data.receivers.next_command().await.unwrap_send_message();

    let note = vec![157];
    data.data_network
        .send(note.clone(), Recipient::Target(bob_node_id))
        .expect("sending works");
    let (peer_id, protocol, message) = data.receivers.next_command().await.unwrap_send_message();
            assert_eq!(peer_id, bob_peer_id);
            assert_eq!(protocol, PROTOCOL_NAME);
            match InternalMessage::<MockData>::decode_all(message.as_slice()) {
                Ok(InternalMessage::Data(session_id, data)) => {
                    assert_eq!(session_id, cur_session_id);
                    assert_eq!(data, note);
                }
                _ => panic!("Expected a properly encoded message"),
            }
    assert!(data.receivers.command_rx.try_next().is_err());
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

        data.receivers.send_event(Event::NotificationsReceived {
            remote: peer_id.0,
            messages,
        });
        data.receivers.send_event(Event::NotificationStreamOpened {
            remote: peer_id.0,
            protocol: Cow::Borrowed(PROTOCOL_NAME),
            role: ObservedRole::Authority,
            negotiated_fallback: None,
        });

        // Wait for acknowledgement that Alice noted the nodes presence.
        data.receivers.next_command().await.unwrap_send_message();
    }
    let note = vec![157];
    data.data_network
        .send(note.clone(), Recipient::All)
        .expect("broadcasting works");
    for _ in 1..2_usize {
        let (_, protocol, message) = data.receivers.next_command().await.unwrap_send_message();
        assert_eq!(protocol, PROTOCOL_NAME);
        match InternalMessage::<MockData>::decode_all(message.as_slice()) {
                    Ok(InternalMessage::Data(session_id, data)) => {
                        assert_eq!(session_id, cur_session_id);
                        assert_eq!(data, note);
                    }
                    _ => panic!("Expected a properly encoded message"),
        }
    }
    assert!(data.receivers.command_rx.try_next().is_err());
}