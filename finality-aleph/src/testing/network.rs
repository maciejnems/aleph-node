use crate::{
    crypto::{AuthorityPen, AuthorityVerifier},
    network::{
        testing::{
            crypto_basics, Authentication, DiscoveryMessage, MockNetwork, MockNetworkIdentity,
            NetworkData, SessionHandler,
        },
        ConnectionIO, ConnectionManager, ConnectionManagerConfig, DataNetwork, NetworkIdentity,
        PeerId, Protocol, Service as NetworkService, SessionManager, SessionNetwork,
        IO as NetworkIO,
    },
    MillisecsPerBlock, NodeIndex, SessionId, SessionPeriod,
};

use codec::Encode;
use futures::channel::{mpsc, oneshot};
use sc_network::{Event, Multiaddr as ScMultiaddr, ObservedRole};
use sc_service::TaskManager;
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};
use tokio::{runtime::Handle, task::JoinHandle, time::timeout};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(10);
const NODES_N: usize = 3;
type MockData = Vec<u8>;

#[derive(Clone)]
struct Authority {
    pen: AuthorityPen,
    addresses: Vec<ScMultiaddr>,
    peer_id: PeerId,
}

impl Authority {
    fn pen(&self) -> AuthorityPen {
        self.pen.clone()
    }

    fn addresses(&self) -> Vec<ScMultiaddr> {
        self.addresses.clone()
    }

    fn peer_id(&self) -> PeerId {
        self.peer_id
    }
}

impl NetworkIdentity for Authority {
    fn identity(&self) -> (Vec<ScMultiaddr>, PeerId) {
        (self.addresses.clone(), self.peer_id)
    }
}

struct TestData {
    pub authorities: Vec<Authority>,
    pub authority_verifier: AuthorityVerifier,
    pub session_manager: SessionManager<MockData>,
    pub network: MockNetwork<NetworkData<MockData>>,
    network_manager_exit_tx: oneshot::Sender<()>,
    network_service_exit_tx: oneshot::Sender<()>,
    network_manager_handle: JoinHandle<()>,
    network_service_handle: JoinHandle<()>,
    _task_manager: TaskManager,
}

async fn prepare_one_session_test_data() -> TestData {
    let task_manager = TaskManager::new(Handle::current(), None).unwrap();
    let (authority_pens, authority_verifier) = crypto_basics(NODES_N).await;
    let authorities: Vec<_> = authority_pens
        .into_iter()
        .map(|(_, p)| {
            let identity = MockNetworkIdentity::new().identity();
            Authority {
                pen: p,
                addresses: identity.0,
                peer_id: identity.1,
            }
        })
        .collect();

    // Prepare Network
    let (event_stream_tx, event_stream_rx) = oneshot::channel();
    let (network_manager_exit_tx, network_manager_exit_rx) = oneshot::channel();
    let (network_service_exit_tx, network_service_exit_rx) = oneshot::channel();
    let network = MockNetwork::new(event_stream_tx);

    let (commands_for_network, commands_from_io) = mpsc::unbounded();
    let (messages_for_network, messages_from_user) = mpsc::unbounded();
    let (commands_for_service, commands_from_user) = mpsc::unbounded();
    let (messages_for_service, commands_from_manager) = mpsc::unbounded();
    let (messages_for_user, messages_from_network) = mpsc::unbounded();

    let connection_io = ConnectionIO::new(
        commands_for_network,
        messages_for_network,
        commands_from_user,
        commands_from_manager,
        messages_from_network,
    );
    let connection_manager = ConnectionManager::<Authority, MockData>::new(
        authorities[0].clone(),
        ConnectionManagerConfig::with_session_period(&SessionPeriod(10), &MillisecsPerBlock(1000)),
    );
    let session_manager = SessionManager::new(commands_for_service, messages_for_service);
    let network_service = NetworkService::new(
        network.clone(),
        task_manager.spawn_handle(),
        NetworkIO::new(messages_from_user, messages_for_user, commands_from_io),
    );

    let network_manager_task = async move {
        tokio::select! {
            _ = connection_io
            .run(connection_manager) => { },
            _ = network_manager_exit_rx => { },
        };
    };

    let network_service_task = async move {
        tokio::select! {
            _ = network_service.run() => { },
            _ = network_service_exit_rx => { },
        };
    };
    let network_manager_handle = tokio::spawn(network_manager_task);
    let network_service_handle = tokio::spawn(network_service_task);

    event_stream_rx.await.unwrap();

    TestData {
        authorities,
        authority_verifier,
        session_manager,
        network,
        network_manager_exit_tx,
        network_service_exit_tx,
        network_manager_handle,
        network_service_handle,
        _task_manager: task_manager,
    }
}

impl TestData {
    fn connect_identity_to_network(&mut self, peer_id: PeerId, protocol: Protocol) {
        self.network.emit_event(Event::NotificationStreamOpened {
            protocol: protocol.name(),
            remote: peer_id.into(),
            negotiated_fallback: None,
            role: ObservedRole::Full,
        });
    }

    async fn start_validator_session(
        &self,
        node_id: usize,
        session_id: u32,
    ) -> SessionNetwork<MockData> {
        self.session_manager
            .start_validator_session(
                SessionId(session_id),
                self.authority_verifier.clone(),
                NodeIndex(node_id),
                self.authorities[node_id].pen(),
            )
            .await
            .expect("Failed to start validator session!")
    }

    fn early_start_validator_session(&self, node_id: usize, session_id: u32) {
        self.session_manager
            .early_start_validator_session(
                SessionId(session_id),
                self.authority_verifier.clone(),
                NodeIndex(node_id),
                self.authorities[node_id].pen(),
            )
            .expect("Failed to start validator session!");
    }

    async fn get_session_handler(&self, node_id: usize, session_id: u32) -> SessionHandler {
        SessionHandler::new(
            Some((NodeIndex(node_id), self.authorities[node_id].pen())),
            self.authority_verifier.clone(),
            SessionId(session_id),
            self.authorities[node_id]
                .addresses()
                .iter()
                .map(|i| i.clone().into())
                .collect(),
        )
        .await
        .unwrap()
    }

    async fn check_sends_add_reserved_node(&mut self) {
        let mut reserved_addresses = HashSet::new();
        for _ in self.authorities.iter().skip(1) {
            let (addresses, protocol) = timeout(DEFAULT_TIMEOUT, self.network.add_reserved.next())
                .await
                .ok()
                .flatten()
                .expect("Should add reserved nodes");
            assert_eq!(protocol, Protocol::Validator.name());
            reserved_addresses.extend(addresses.into_iter());
        }

        let mut expected_addresses = HashSet::new();
        for authority in self.authorities.iter().skip(1) {
            expected_addresses.extend(authority.addresses());
        }

        assert_eq!(reserved_addresses, expected_addresses);
    }

    async fn check_sends_authentication(&mut self, authentication: Authentication) {
        let mut sent_auth = HashMap::new();
        while sent_auth.len() < NODES_N - 1 {
            if let Some((
                NetworkData::Meta(DiscoveryMessage::Authentication(auth_data)),
                peer_id,
                protocol,
            )) = timeout(DEFAULT_TIMEOUT, self.network.send_message.next())
                .await
                .expect("Should send authentication")
            {
                assert_eq!(protocol, Protocol::Generic.name());
                sent_auth.insert(peer_id, auth_data);
            }
        }

        let mut expected_auth = HashMap::new();
        for authority in self.authorities.iter().skip(1) {
            expected_auth.insert(authority.peer_id(), authentication.clone());
        }

        assert_eq!(sent_auth, expected_auth);
    }

    async fn connect_session_authorities(&mut self, session_id: u32) {
        for (index, authority) in self.authorities.clone().into_iter().enumerate().skip(1) {
            let handler = self.get_session_handler(index, session_id).await;

            self.connect_identity_to_network(authority.peer_id(), Protocol::Generic);
            self.connect_identity_to_network(authority.peer_id(), Protocol::Validator);

            self.network.emit_event(Event::NotificationsReceived {
                remote: authority.peer_id().into(),
                messages: vec![(
                    Protocol::Generic.name(),
                    NetworkData::<MockData>::Meta(DiscoveryMessage::AuthenticationBroadcast(
                        handler.authentication().unwrap(),
                    ))
                    .encode()
                    .into(),
                )],
            });
        }
    }

    async fn start_session(&mut self, session_id: u32) -> SessionNetwork<MockData> {
        let data_network = self.start_validator_session(0, session_id).await;
        self.connect_session_authorities(session_id).await;
        self.check_sends_add_reserved_node().await;
        self.check_sends_authentication(
            self.get_session_handler(0, session_id)
                .await
                .authentication()
                .unwrap(),
        )
        .await;

        data_network
    }

    fn emit_notifications_received(
        &mut self,
        node_id: usize,
        messages: Vec<NetworkData<MockData>>,
    ) {
        self.network.emit_event(Event::NotificationsReceived {
            remote: self.authorities[node_id].peer_id().into(),
            messages: messages
                .iter()
                .map(|m| (Protocol::Validator.name(), m.encode().into()))
                .collect(),
        });
    }

    async fn cleanup(self) {
        self.network_manager_exit_tx.send(()).unwrap();
        self.network_service_exit_tx.send(()).unwrap();
        self.network_manager_handle.await.unwrap();
        self.network_service_handle.await.unwrap();
        while self.network.send_message.try_next().await.is_some() {}
        self.network.close_channels().await;
    }
}

#[tokio::test]
async fn test_connects_to_others() {
    let mut test_data = prepare_one_session_test_data().await;
    let mut data_network = test_data.start_session(43).await;

    test_data.emit_notifications_received(1, vec![NetworkData::Data(vec![1, 2, 3], SessionId(43))]);
    assert_eq!(
        timeout(DEFAULT_TIMEOUT, data_network.next()).await,
        Ok(Some(vec![1, 2, 3]))
    );

    test_data.cleanup().await;
    assert_eq!(data_network.next().await, None);
}
