use sc_service::{ Configuration, Error, TFullBackend, TLightBackend};

use crate::chain_spec::{development_config, ChainParams};
use aleph_runtime::{Block, RuntimeApi};

sc_executor::native_executor_instance!(
    pub Executor,
    aleph_runtime::api::dispatch,
    aleph_runtime::native_version,
);
use sc_service_test::TestNetComponents;

type FullClient = sc_service::TFullClient<Block, RuntimeApi, Executor>;
type FullTransactionPool = sc_transaction_pool::FullPool<Block, FullClient>;
type FullBackend = TFullBackend<Block>;
type FullTestNetNode = TestNetComponents<Block, FullBackend, Executor, RuntimeApi, FullTransactionPool>;

type LightClient = sc_service::TLightClient<Block, RuntimeApi, Executor>;
type LightTransactionPool = sc_transaction_pool::LightPool<Block, LightClient, sc_network::config::OnDemand<Block>>;
type LightBackend = TLightBackend<Block>;
type LightTestNetNode = TestNetComponents<Block, LightBackend, Executor, RuntimeApi, LightTransactionPool>;

fn light_builder(config: Configuration) -> Result<LightTestNetNode, Error>{
    Err(Error::Other("light_builder".into()))
}

fn full_builder(config: Configuration) -> Result<FullTestNetNode, Error>{
    Err(Error::Other("full_builder".into()))
}


#[test]
fn test_consensus() {
    let chain_params = ChainParams {
        chain_id: "".to_string(),
        base_path: Default::default(),
        node_key_file: "".to_string(),
        session_period: Some(10),
        millisecs_per_block: Some(400),
        chain_name: None,
        token_symbol: None,
        account_ids: None,
        n_members: None
    };
    let chain_spec = development_config(chain_params, Vec::new()).unwrap();
    sc_service_test::consensus(
        chain_spec,
        full_builder,
        light_builder,
        Vec::new()
    )
}
