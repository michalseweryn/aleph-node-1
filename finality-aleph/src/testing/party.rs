use super::network::TestNetworkHub;
use crate::{run_consensus_party, AlephConfig, AlephParams};
use aleph_primitives::SessionPeriod;
use core::time::Duration;
use futures::FutureExt;
use sc_keystore::LocalKeystore;
use sc_service::{config::TaskType, BasePath, TaskExecutor, TaskManager};
use std::{path::PathBuf, sync::Arc};

const ACCOUNT_IDS: [&str; 4] = [
    "5D34dL5prEUaGNQtPPZ3yN5Y6BnkfXunKXXz6fo7ZJbLwRRH",
    "5GBNeWRhZc2jXu7D55rBimKYDk8PGk8itRYFTPfC8RJLKG5o",
    "5Dfis6XL8J2P6JHUnUtArnFWndn62SydeP8ee8sG2ky9nfm9",
    "5F4H97f7nQovyrbiq4ZetaaviNwThSVcFobcA5aGab6167dK",
];

fn base_path(i: usize) -> BasePath {
    BasePath::Permanenent(PathBuf::from(format!("/tmp/alephtest{}/", i)))
}

use substrate_test_runtime_client::{prelude::*, TestClientBuilder};

pub(crate) mod client;

#[tokio::test]
async fn run() {
    let size = ACCOUNT_IDS.len();

    let (network_hub, networks) = TestNetworkHub::new(size);

    let mut justification_txs = Vec::with_capacity(size);

    for (i, network) in networks.into_iter().enumerate() {
        let builder = TestClientBuilder::with_default_backend();
        let backend = builder.backend();
        let select_chain = sc_consensus::LongestChain::new(backend.clone());
        let client = Arc::new(client::TestClient::new(builder.build()));

        let (justification_tx, justification_rx) = futures::channel::mpsc::unbounded();
        let keystore = Arc::new(LocalKeystore::open(base_path(i).path(), None).unwrap());
        justification_txs.push(justification_tx);

        let tokio_runtime = tokio::runtime::Builder::new()
            .threaded_scheduler()
            .enable_all()
            .build()
            .unwrap();

        let task_executor = move |fut, task_type| match task_type {
            TaskType::Async => tokio_runtime.handle().spawn(fut).map(drop),
            TaskType::Blocking => tokio_runtime
                .handle()
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
                millisecs_per_block: Default::default(),
                unit_creation_delay: Default::default(),
            },
        }));
    }
    tokio::spawn(network_hub.run());
    futures_timer::Delay::new(Duration::from_secs(10)).await;
}
