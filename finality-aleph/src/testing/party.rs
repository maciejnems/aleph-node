use super::network::{self, TestNetworkHub};
use crate::{run_consensus_party, AlephConfig, AlephParams, KEY_TYPE};
use aleph_primitives::{AuthorityId, MillisecsPerBlock, SessionPeriod};
use core::time::Duration;
use futures::FutureExt;
// use sc_block_builder::BlockBuilderProvider;
use sc_client_api::HeaderBackend;
use sc_keystore::LocalKeystore;
use sc_service::{config::TaskType, BasePath, TaskManager};
use sp_keystore::testing::KeyStore;
// use sp_consensus::BlockOrigin;
use sp_keystore::CryptoStore;
use sp_runtime::{generic::BlockId, traits::Zero};
use std::collections::BTreeMap;
use std::{path::PathBuf, sync::Arc};

const ACCOUNT_IDS: [&str; 4] = [
    "//5D34dL5prEUaGNQtPPZ3yN5Y6BnkfXunKXXz6fo7ZJbLwRRH",
    "//5GBNeWRhZc2jXu7D55rBimKYDk8PGk8itRYFTPfC8RJLKG5o",
    "//5Dfis6XL8J2P6JHUnUtArnFWndn62SydeP8ee8sG2ky9nfm9",
    "//5F4H97f7nQovyrbiq4ZetaaviNwThSVcFobcA5aGab6167dK",
];

fn base_path(i: usize) -> BasePath {
    BasePath::Permanenent(PathBuf::from(format!("/tmp/alephtest{}/", i)))
}

use substrate_test_runtime_client::{prelude::*, TestClientBuilder};

pub(crate) mod client;

// #[derive(Clone)]
struct Authority {
    authority_id: AuthorityId,
    keystore: Arc<KeyStore>,
}

async fn generate_authorities(ss: &[&str]) -> Vec<Authority> {
    let mut auth_ids = Vec::with_capacity(ss.len());
    let mut keystores = Vec::with_capacity(ss.len());
    for s in ss {
        let keystore = KeyStore::new();
        let pk = keystore
            .ed25519_generate_new(KEY_TYPE, Some(s))
            .await
            .unwrap();
        auth_ids.push(AuthorityId::from(pk));
        keystores.push(Arc::new(keystore));
    }
    let mut authorities = Vec::with_capacity(ss.len());
    for i in 0..ss.len() {
        authorities.push(Authority {
            authority_id: auth_ids[i].clone(),
            keystore: keystores[i].clone(),
        });
    }
    authorities
}

#[tokio::test]
async fn run() {
    let size = ACCOUNT_IDS.len();

    let (network_hub, networks) = TestNetworkHub::new(size);

    let mut justification_txs = Vec::with_capacity(size);

    let authorities = generate_authorities(&ACCOUNT_IDS).await;
    let authority_ids = authorities
        .iter()
        .map(|a| a.authority_id.clone())
        .collect::<Vec<_>>();

    let mut clients = vec![];

    for (i, network) in networks.into_iter().enumerate() {
        let builder = TestClientBuilder::with_default_backend();
        let backend = builder.backend();
        let select_chain = sc_consensus::LongestChain::new(backend.clone());
        let authority_schedule = [(Zero::zero(), authority_ids.clone())]
            .iter()
            .cloned()
            .collect();
        let client = Arc::new(client::TestClient::new(
            builder.build(),
            authority_schedule,
            SessionPeriod(4),
            MillisecsPerBlock(1000),
        ));
        clients.push(client.clone());

        let (justification_tx, justification_rx) = futures::channel::mpsc::unbounded();
        let keystore = authorities[i].keystore.clone();
        justification_txs.push(justification_tx);

        // let tokio_runtime = tokio::runtime::Builder::new()
        //     .threaded_scheduler()
        //     .enable_all()
        //     .build()
        //     .unwrap();

        // let task_executor = move |fut, task_type| match task_type {
        //     TaskType::Async => tokio_runtime.handle().spawn(fut).map(drop),
        //     TaskType::Blocking => tokio_runtime
        //         .handle()
        //         .spawn_blocking(move || futures::executor::block_on(fut))
        //         .map(drop),
        // };

        let runtime = tokio::runtime::Runtime::new().expect("Creates tokio runtime");
        let tokio_handle = runtime.handle().clone();

        let manager = TaskManager::new(tokio_handle, None).unwrap();

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
                millisecs_per_block: Default::default(),
                unit_creation_delay: Default::default(),
            },
        }));
    }
    tokio::spawn(network_hub.run());
    futures_timer::Delay::new(Duration::from_secs(2)).await;
    println!("{:?}", clients[0].info().best_number);
    println!("{:?}", clients[0].info().finalized_number);
    panic!("End of the test");
}
