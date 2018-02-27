extern crate curl;
extern crate exonum;
extern crate exonum_configuration;
extern crate serde;
#[macro_use]
extern crate serde_derive;
extern crate serde_json;

extern crate dmbc;

mod net_config;

use std::path::Path;
use std::io;
use std::io::Read;
use std::fs::File;

use exonum::blockchain;
use exonum::blockchain::{ConsensusConfig, GenesisConfig, TimeoutAdjusterConfig,
                         ValidatorKeys};
use exonum::crypto::{PublicKey, SecretKey};
use exonum::encoding::serialize::FromHex;
use exonum::node::{Node, NodeApiConfig, NodeConfig};
use exonum::storage::{RocksDB, RocksDBOptions};
use exonum_configuration::ConfigurationService;
use dmbc::config;
use dmbc::currency::Service;

const GENESIS_VALIDATOR_PUBLIC: &str =
    "4e298e435018ab0a1430b6ebd0a0656be15493966d5ce86ed36416e24c411b9f";
const GENESIS_SERVICE_PUBLIC: &str =
    "68e774a4339cccfae644dcf3e44360839c84a6475c7d2943ed59b81d7eb6e9f0";

fn main() {
    exonum::helpers::init_logger().unwrap();

    /** Create Keys */
    println!(
        "Initializing node: {}",
        config::config().api().current_node()
    );

    let keys_path = config::config().api().keys_path();

    let consensus_public_key =
        PublicKey::from_hex(slurp(keys_path.clone() + "/consensus.pub").unwrap()).unwrap();
    let consensus_secret_key =
        SecretKey::from_hex(slurp(keys_path.clone() + "/consensus").unwrap()).unwrap();
    let service_public_key =
        PublicKey::from_hex(slurp(keys_path.clone() + "/service.pub").unwrap()).unwrap();
    let service_secret_key =
        SecretKey::from_hex(slurp(keys_path.clone() + "/service").unwrap()).unwrap();

    let public_api = config::config().api().address().parse().unwrap();
    let private_api = config::config().api().private_address().parse().unwrap();
    let peer_address = config::config().api().peer_address().parse().unwrap();

    let info = net_config::ValidatorInfo {
        public: public_api,
        private: private_api,
        peer: peer_address,
        consensus: consensus_public_key,
        service: service_public_key,
    };
    eprintln!("Node info: {:?}", &info);

    let is_validator = config::config().api().is_validator();
    eprintln!(
        "Connecting {}",
        if is_validator {
            "as validator"
        } else {
            "as auditor"
        }
    );

    let peers = match net_config::connect(&info, is_validator) {
        Ok(peers) => {
            eprintln!("Connected as validator, peers: {:?}", &peers);
            peers
        }
        Err(e) => {
            eprintln!("Unable to connect as validator: {}", &e);
            eprintln!("Running in loner-mode.");
            Default::default()
        }
    };

    let consensus_config = ConsensusConfig {
        round_timeout: 3000,
        status_timeout: 5000,
        peers_timeout: 10_000,
        txs_block_limit: 1000,
        timeout_adjuster: TimeoutAdjusterConfig::Dynamic {
            min: 200,
            max: 1000,
            threshold: 1,
        },
    };

    // Configure Node
    let validators = Some(ValidatorKeys {
        consensus_key: PublicKey::from_hex(GENESIS_VALIDATOR_PUBLIC).unwrap(),
        service_key: PublicKey::from_hex(GENESIS_SERVICE_PUBLIC).unwrap(),
    });

    let genesis = GenesisConfig::new_with_consensus(consensus_config, validators.into_iter());
    let api_cfg = NodeApiConfig {
        public_api_address: Some(public_api),
        private_api_address: Some(private_api),
        ..Default::default()
    };

    let peer_addrs = peers.iter().map(|(_, p)| p.peer).collect();

    // Complete node configuration
    let node_cfg = NodeConfig {
        listen_address: config::config().api().peer_address().parse().unwrap(),
        peers: peer_addrs,
        service_public_key,
        service_secret_key,
        consensus_public_key,
        consensus_secret_key,
        genesis,
        external_address: None,
        network: Default::default(),
        whitelist: Default::default(),
        api: api_cfg,
        mempool: Default::default(),
        services_configs: Default::default(),
    };

    // Initialize database
    let mut options = RocksDBOptions::default();
    options.create_if_missing(true);
    let path = config::config().db().path();
    let db = Box::new(RocksDB::open(path, &options).unwrap());

    // Initialize services
    let services: Vec<Box<blockchain::Service>> = vec![
        Box::new(ConfigurationService::new()),
        Box::new(Service()),
    ];

    eprintln!("Launching node. What can possibly go wrong?");

    let node = Node::new(db, services, node_cfg);
    node.run().unwrap();
}

fn slurp<P: AsRef<Path>>(filename: P) -> io::Result<String> {
    let mut out = String::new();
    File::open(filename)
        .and_then(|mut file| file.read_to_string(&mut out))
        .map(|_| eprintln!("Slurped: {}", &out))
        .map(move |_| out)
}
