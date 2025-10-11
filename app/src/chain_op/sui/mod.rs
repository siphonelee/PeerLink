use crate::{chain_op::{NetworkDetails, NetworkSummary}, common::PeerId};
use super::{
    ChainOperationInterface, ChainResult, ChainOperationError, 
    NetworkInfo, ExitNodeInfo, TransactionResponse
};
use shared_crypto::intent::Intent;
use sui_config::{
    sui_config_dir, Config, PersistedConfig, SUI_CLIENT_CONFIG, SUI_KEYSTORE_FILENAME,
};
use futures::{future, stream::StreamExt};
use sui_json_rpc_types::{BalanceChange, Coin, SuiExecutionStatus, SuiObjectDataOptions, SuiTransactionBlockEffects, SuiTransactionBlockEffectsAPI, SuiTypeTag};
use fastcrypto::{
    ed25519::Ed25519KeyPair,
    encoding::{Base64, Encoding},
    secp256k1::Secp256k1KeyPair,
    secp256r1::Secp256r1KeyPair,
    traits::{EncodeDecodeBase64, KeyPair, ToFromBytes},
};
use sui_keys::keystore::{AccountKeystore, FileBasedKeystore, GenerateOptions};
use sui_sdk::{
    rpc_types::SuiTransactionBlockResponseOptions, 
    sui_client_config::{SuiClientConfig, SuiEnv}, 
    types::{
        base_types::{ObjectID, SuiAddress, SequenceNumber}, crypto::SuiKeyPair, programmable_transaction_builder::ProgrammableTransactionBuilder, quorum_driver_types::ExecuteTransactionRequestType, transaction::{CallArg, ObjectArg, ProgrammableTransaction, Transaction, TransactionData, Argument}
    }, 
    wallet_context::WalletContext, 
    SuiClient, 
    SuiClientBuilder
};
use sui_types::transaction::TransactionDataAPI;
use std::str::FromStr;
use std::hash::Hasher;
use anyhow::bail;
use chrono;
use bcs;

/// Sui blockchain implementation of chain operations
/// This implementation handles network management on the Sui blockchain
pub struct SuiChainOperator {
    /// Private key for signing transactions
    pub private_key: String,
    /// Package ID of the deployed peerlink contract
    pub package_id: String,
    /// Registry object ID for network management
    pub registry_id: String,
    /// Registry obj version
    pub registry_version: u64,
    /// Registry obj digest
    pub registry_digest: String,
}

impl SuiChainOperator {
    /// Create a new Sui chain operation instance
    pub fn new(private_key: String, 
            package_id: String, registry_version: u64, registry_digest: String, 
            registry_id: String) -> Self {
        Self {
            private_key: private_key,
            package_id: package_id,
            registry_version: registry_version,
            registry_digest: registry_digest,
            registry_id: registry_id,
        }
    }
}

impl SuiChainOperator {
    async fn get_total_balance_change(&self, bc: &Option<Vec<sui_json_rpc_types::BalanceChange>>) -> i128 {
        if bc.is_none() {
            return 0i128;
        }

        bc.as_ref().unwrap().iter()
            .fold(0i128, |total, item| total + item.amount)
    }

    pub async fn retrieve_wallet(&self) -> Result<WalletContext, anyhow::Error> {
        let wallet_conf = sui_config_dir()?.join(SUI_CLIENT_CONFIG);
        let keystore_path = sui_config_dir()?.join(SUI_KEYSTORE_FILENAME);

        // check if a wallet exists and if not, create a wallet and a sui client config
        if !keystore_path.exists() {
            let keystore = FileBasedKeystore::load_or_create(&keystore_path)?;
            keystore.save().await?;
        }

        if !wallet_conf.exists() {
            let keystore = FileBasedKeystore::load_or_create(&keystore_path)?;
            let mut client_config = SuiClientConfig::new(keystore.into());

            client_config.add_env(SuiEnv::testnet());
            client_config.add_env(SuiEnv::devnet());
            client_config.add_env(SuiEnv::localnet());

            if client_config.active_env.is_none() {
                client_config.active_env = client_config.envs.first().map(|env| env.alias.clone());
            }

            client_config.save(&wallet_conf)?;
            tracing::info!("Client config file is stored in {:?}.", &wallet_conf);
        }

        let mut keystore = FileBasedKeystore::load_or_create(&keystore_path)?;
        let mut client_config: SuiClientConfig = PersistedConfig::read(&wallet_conf)?;

        let default_active_address = if let Some(address) = keystore.addresses().first() {
            *address
        } else {
            keystore
                .generate(None, GenerateOptions::Default).await?
                .address
        };

        if keystore.addresses().len() < 2 {
            keystore.generate(None, GenerateOptions::Default).await?;
        }

        client_config.active_address = Some(default_active_address);
        client_config.save(&wallet_conf)?;

        let wallet =
            WalletContext::new(&wallet_conf)?;

        Ok(wallet)
    }   

    pub async fn setup_for_read(&self) -> Result<(SuiClient, SuiAddress), anyhow::Error> {
        let client = SuiClientBuilder::default().build_testnet().await?;
        println!("Sui testnet version is: {}", client.api_version());
        let mut wallet = self.retrieve_wallet().await?;
        assert!(wallet.get_addresses().len() >= 2);
        let active_address = wallet.active_address()?;

        println!("Wallet active address is: {active_address}");
        Ok((client, active_address))
    }

    pub async fn fetch_coin(
        &self,
        sui: &SuiClient,
        sender: &SuiAddress,
    ) -> Result<Option<Coin>, anyhow::Error> {
        let coin_type = "0x2::sui::SUI".to_string();
        let coins_stream = sui
            .coin_read_api()
            .get_coins_stream(*sender, Some(coin_type));

        let mut coins = coins_stream
            .skip_while(|c| future::ready(c.balance < 5_000_000))
            .boxed();
        let coin = coins.next().await;
        Ok(coin)
    }

    pub async fn setup_for_write(&self) -> Result<(SuiClient, SuiAddress, SuiAddress, Coin), anyhow::Error> {
        let (client, active_address) = self.setup_for_read().await?;

        // make sure we have some SUI (5_000_000 MIST) on this address
        let coin = self.fetch_coin(&client, &active_address).await?;
        if coin.is_none() {
            bail!("No coin found, aborting...")
        }
        let wallet = self.retrieve_wallet().await?;
        let addresses = wallet.get_addresses();
        let addresses = addresses
            .into_iter()
            .filter(|address| address != &active_address)
            .collect::<Vec<_>>();
        let recipient = addresses
            .first()
            .expect("Cannot get the recipient address needed for writing operations. Aborting");

        Ok((client, active_address, *recipient, coin.unwrap()))
    }

    /// Parse network data from BCS-encoded bytes returned by the contract
    fn parse_network_data(&self, data: &[u8]) -> Result<Vec<NetworkSummary>, Box<dyn std::error::Error>> {
        let mut networks = Vec::new();        
        
        // Handle empty data case - return empty vector instead of error
        if data.is_empty() {
            return Ok(networks);
        }
                        
        // If data is too short to contain a proper vector length (1 byte for small vectors),
        // it's likely an empty vector or invalid data - return empty result
        if data.len() < 1 {
            tracing::info!("Data length {} is too short for BCS vector, treating as empty", data.len());
            return Ok(networks);
        }
        
        // Read vector length (first byte for small vectors in BCS encoding)
        let vector_len = data[0] as usize;
                
        let mut offset = 1; // Start after the vector length byte
        for i in 0..vector_len {
            if offset >= data.len() {
                tracing::warn!("Reached end of data while parsing network {}", i);
                break;
            }
            
            // Parse individual network
            match self.parse_single_network(&data[offset..]) {
                Ok((network, consumed)) => {
                    networks.push(network);
                    offset += consumed;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse network {}: {}", i, e);
                    tracing::warn!("Remaining data length: {}, first 10 bytes: {:?}", data[offset..].len(), &data[offset..std::cmp::min(offset + 10, data.len())]);
                    return Err(format!("Failed to parse network {} at offset {}: {}", i, offset, e).into());
                }
            }
        }
        
        Ok(networks)
    }

    /// Parse a single network from BCS data
    fn parse_single_network(&self, data: &[u8]) -> Result<(NetworkSummary, usize), Box<dyn std::error::Error>> {
        let mut offset = 0;
                
        // Parse name (length + string) - BCS uses variable length encoding for strings
        if offset + 1 > data.len() {
            return Err(format!("Insufficient data for name length: need 1 byte at offset {}, but only {} bytes available", offset, data.len()).into());
        }
        let name_len = data[offset] as usize;
        offset += 1;
        
        if name_len > 1000 {
            return Err(format!("Name length {} seems unreasonably large", name_len).into());
        }
        
        if offset + name_len > data.len() {
            return Err(format!("Insufficient data for name: need {} bytes at offset {}, but only {} bytes available", name_len, offset, data.len()).into());
        }
        let name = String::from_utf8(data[offset..offset + name_len].to_vec())
            .map_err(|e| format!("Failed to parse name as UTF-8: {}", e))?;
        offset += name_len;
                
        // Parse admin (length + string)
        if offset + 1 > data.len() {
            return Err(format!("Insufficient data for admin length: need 1 byte at offset {}, but only {} bytes available", offset, data.len()).into());
        }
        let admin_len = data[offset] as usize;
        offset += 1;
        
        if admin_len > 1000 {
            return Err(format!("Admin length {} seems unreasonably large", admin_len).into());
        }
        
        if offset + admin_len > data.len() {
            return Err(format!("Insufficient data for admin: need {} bytes at offset {}, but only {} bytes available", admin_len, offset, data.len()).into());
        }
        let admin = String::from_utf8(data[offset..offset + admin_len].to_vec())
            .map_err(|e| format!("Failed to parse admin as UTF-8: {}", e))?;
        offset += admin_len;
        
        // Parse created_at (8 bytes u64)
        if offset + 8 > data.len() {
            return Err(format!("Insufficient data for created_at: need 8 bytes at offset {}, but only {} bytes available", offset, data.len()).into());
        }
        let created_at = u64::from_le_bytes([
            data[offset], data[offset+1], data[offset+2], data[offset+3],
            data[offset+4], data[offset+5], data[offset+6], data[offset+7]
        ]);
        offset += 8;
                
        // Parse description (length + string)        
        if offset + 1 > data.len() {
            return Err(format!("Insufficient data for description length: need 1 byte at offset {}, but only {} bytes available", offset, data.len()).into());
        }
        let desc_len = data[offset] as usize;
        offset += 1;
        
        if desc_len > 10000 {
            return Err(format!("Description length {} seems unreasonably large", desc_len).into());
        }
        
        if offset + desc_len > data.len() {
            return Err(format!("Insufficient data for description: need {} bytes at offset {}, but only {} bytes available", desc_len, offset, data.len()).into());
        }
        let desc = String::from_utf8(data[offset..offset + desc_len].to_vec())
            .map_err(|e| format!("Failed to parse description as UTF-8: {}", e))?;
        offset += desc_len;
        
        // Parse max_peers (8 bytes u64)
        if offset + 8 > data.len() {
            return Err(format!("Insufficient data for max_peers: need 8 bytes at offset {}, but only {} bytes available", offset, data.len()).into());
        }
        let max_peers = u64::from_le_bytes([
            data[offset], data[offset+1], data[offset+2], data[offset+3],
            data[offset+4], data[offset+5], data[offset+6], data[offset+7]
        ]);
        offset += 8;

        let network = NetworkSummary {
            name,
            admin,
            created_at,
            description: desc,
            max_peers,
        };
        
        Ok((network, offset))
    }

    /// Parse network info from a tuple returned by get_network_info Move function
    /// The tuple format is: (String, address, u64, bool, String, u64, u64)
    /// which corresponds to: (name, admin, created_at, is_active, description, max_peers)
    fn parse_network_info_tuple(&self, ret_values: &Vec<(Vec<u8>, SuiTypeTag)>, expected_name: &str) -> Result<Option<NetworkDetails>, Box<dyn std::error::Error>> {
        if ret_values.is_empty() {
            return Ok(None);
        }
        if ret_values.len() != 7 {
            return Err(format!("network info has no enough fields, expected: 7, but {} available", ret_values.len()).into());
        }

        let mut name = String::new();
        let mut admin = String::new();
        let mut created_at = 0u64;
        let mut is_active = false;
        let mut description = String::new();
        let mut max_peers = 0u64;
        let mut total_exit_nodes = 0u64;
        for (idx, data) in ret_values.iter().enumerate() {
            let mut offset = 0;

            // Parse name (length + string) - first element of tuple
            if data.0.len() < 1 {
                return Err(format!("Insufficient data for name length: need 1 byte at offset {}, but only {} bytes available", offset, data.0.len()).into());
            }

            match idx {
                0 => {
                    let field_len = data.0[offset] as usize;
                    offset += 1;

                    if field_len > 1000 {
                        return Err(format!("Name length {} seems unreasonably large", field_len).into());
                    }
                    if offset + field_len > data.0.len() {
                        return Err(format!("Insufficient data for name: need {} bytes at offset {}, but only {} bytes available", field_len, offset, data.0.len()).into());
                    }
                    name = String::from_utf8(data.0[offset..offset + field_len].to_vec())
                        .map_err(|e| format!("Failed to parse name as UTF-8: {}", e))?;

                    // Verify this is the network we're looking for
                    if name != expected_name {
                        tracing::warn!("Expected network name '{}' but got '{}'", expected_name, name);
                        return Err(format!("network name mismatch, expectd: {}, got: {}", expected_name, name).into());
                    }

                }
                1 => {
                    // Parse admin address (32 bytes for Sui address)
                    if offset + 32 > data.0.len() {
                        return Err(format!("Insufficient data for admin address: need 32 bytes at offset {}, but only {} bytes available", offset, data.0.len()).into());
                    }
                    let admin_bytes = &data.0[offset..offset + 32];
                    admin = format!("0x{}", hex::encode(admin_bytes));
                }
                2 => {
                    // Parse created_at (8 bytes u64)
                    if offset + 8 > data.0.len() {
                        return Err(format!("Insufficient data for created_at: need 8 bytes at offset {}, but only {} bytes available", offset, data.0.len()).into());
                    }
                    created_at = u64::from_le_bytes([
                        data.0[offset], data.0[offset+1], data.0[offset+2], data.0[offset+3],
                        data.0[offset+4], data.0[offset+5], data.0[offset+6], data.0[offset+7]
                    ]);
                }
                3 => {
                    // Parse is_active (1 byte bool)
                    if offset + 1 > data.0.len() {
                        return Err(format!("Insufficient data for is_active: need 1 byte at offset {}, but only {} bytes available", offset, data.0.len()).into());
                    }
                    is_active = data.0[offset] != 0;
                }
                4 => {
                    // Parse description (length + string)
                    let desc_len = data.0[offset] as usize;
                    offset += 1;
                                        
                    if offset + desc_len > data.0.len() {
                        return Err(format!("Insufficient data for description: need {} bytes at offset {}, but only {} bytes available", desc_len, offset, data.0.len()).into());
                    }
                    description = String::from_utf8(data.0[offset..offset + desc_len].to_vec())
                        .map_err(|e| format!("Failed to parse description as UTF-8: {}", e))?;
                }
                5 => {
                    // Parse max_peers (8 bytes u64)
                    if offset + 8 > data.0.len() {
                        return Err(format!("Insufficient data for max_peers: need 8 bytes at offset {}, but only {} bytes available", offset, data.0.len()).into());
                    }
                    max_peers = u64::from_le_bytes([
                        data.0[offset], data.0[offset+1], data.0[offset+2], data.0[offset+3],
                        data.0[offset+4], data.0[offset+5], data.0[offset+6], data.0[offset+7]
                    ]);
                }
                6 => {
                    // Parse total exit nodes (8 bytes u64)
                    if offset + 8 > data.0.len() {
                        return Err(format!("Insufficient data for total_exit_nodes: need 8 bytes at offset {}, but only {} bytes available", offset, data.0.len()).into());
                    }
                    total_exit_nodes = u64::from_le_bytes([
                        data.0[offset], data.0[offset+1], data.0[offset+2], data.0[offset+3],
                        data.0[offset+4], data.0[offset+5], data.0[offset+6], data.0[offset+7]
                    ]);
                }
                _ => {}
            }            
        }

        let network_details = NetworkDetails {
            name,
            secret: String::new(), // Empty for security - use get_network_secret separately if needed
            admin,
            created_at,
            is_active,
            description: Some(description),
            max_peers,
            total_exit_nodes,
        };
        
        Ok(Some(network_details))
    }
}

#[async_trait::async_trait]
impl ChainOperationInterface for SuiChainOperator {
    async fn create_network(&self, name: String, secret: String, description: Option<String>) -> ChainResult<TransactionResponse> {
        tracing::info!("Creating network '{}' on Sui blockchain with description: {:?}", name, description);
        
        // Validate inputs
        if name.is_empty() {
            return Err(ChainOperationError::InvalidNetworkName(name));
        }
        
        if secret.len() < 8 {
            return Err(ChainOperationError::InvalidSecret("Secret must be at least 8 characters".to_string()));
        }

        // Parse object IDs to validate them
        let package_object_id = ObjectID::from_str(&self.package_id)
            .map_err(|e| ChainOperationError::ConnectionError(format!("Invalid package ID: {}", e)))?;
        let registry_object_id = ObjectID::from_str(&self.registry_id)
            .map_err(|e| ChainOperationError::ConnectionError(format!("Invalid registry ID: {}", e)))?;

            
        // 1) get the Sui client, the sender and recipient that we will use
        // for the transaction, and find the coin we use as gas 
        let (sui, sender, _recipient, coin) = self.setup_for_write().await
                                    .map_err(|e| ChainOperationError::SetupWriteFailed(format!("Sui Client setup for writing failed: {}", e)))?;

        // create a programmable transaction builder to add commands and create a PTB
        let mut ptb = ProgrammableTransactionBuilder::new();

        // Add pure arguments for the function call with proper BCS encoding
        let network_name_arg = CallArg::Pure(bcs::to_bytes(&name.clone()).map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to serialize network name: {}", e)))?);
        let network_secret_arg = CallArg::Pure(bcs::to_bytes(&secret.as_bytes().to_vec()).map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to serialize network secret: {}", e)))?);
        let description_arg = CallArg::Pure(bcs::to_bytes(&description.unwrap_or_else(|| "Network created via peerlink".to_string())).map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to serialize description: {}", e)))?);
        let max_peers_arg = CallArg::Pure(bcs::to_bytes(&1000u64).map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to serialize max_peers: {}", e)))?);
        
        // Add clock object (0x6 is the well-known clock object ID on Sui)
        let clock_object_id = ObjectID::from_str("0x0000000000000000000000000000000000000000000000000000000000000006")
            .map_err(|e| ChainOperationError::ConnectionError(format!("Invalid clock object ID: {}", e)))?;
        let clock_arg = CallArg::Object(ObjectArg::SharedObject{
            id: clock_object_id,
            initial_shared_version: SequenceNumber::from(1),
            mutable: false,
        });
            
        // Add registry object as shared object (first argument)
        let registry_arg = CallArg::Object(ObjectArg::SharedObject {
            id: registry_object_id,
            initial_shared_version: SequenceNumber::from(self.registry_version),
            mutable: true, // Registry needs to be mutable for creating networks
        });
                
        // Build the Move call to create_network function
        ptb.move_call(
            package_object_id,
            "peerlink".parse().unwrap(),
            "create_network".parse().unwrap(),
            vec![], // No type arguments
            vec![
                registry_arg,
                network_name_arg,
                network_secret_arg,
                description_arg,
                max_peers_arg,
                clock_arg,
            ],
        ).map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to build Move call: {:?}", e)))?;

        let programmable_transaction = ptb.finish();

        let gas_budget = 10_000_000;
        let gas_price = sui.read_api().get_reference_gas_price().await
            .map_err(|e| ChainOperationError::GetSuiPriceFailed(format!("Failed to get Sui gas price: {:?}", e)))?;

        // create the transaction data that will be sent to the network
        let tx_data = TransactionData::new_programmable(
            sender,
            vec![coin.object_ref()],
            programmable_transaction,
            gas_budget,
            gas_price,
        );

        // sign transaction
        let keystore_path = sui_config_dir()
                    .map_err(|e| ChainOperationError::SuiConfigDirError(format!("Failed to get Sui config dir: {:?}", e)))?
                    .join(SUI_KEYSTORE_FILENAME);
        let keystore = FileBasedKeystore::load_or_create(&keystore_path)
                .map_err(|e| ChainOperationError::GetSuiPriceFailed(format!("Failed to get Sui gas price: {:?}", e)))?;
        let signature = keystore.sign_secure(&sender, &tx_data, Intent::sui_transaction())
                .await
                .map_err(|e| ChainOperationError::GetSuiPriceFailed(format!("Failed to sign Sui transaction: {:?}", e)))?;

        // execute the transaction
        let transaction_response = sui
            .quorum_driver_api()
            .execute_transaction_block(
                Transaction::from_data(tx_data, vec![signature]),
                SuiTransactionBlockResponseOptions::full_content(),
                Some(ExecuteTransactionRequestType::WaitForLocalExecution),
            )
            .await
            .map_err(|e| ChainOperationError::ExecuteTransactionBlockFailed(format!("Failed to execute_transaction_block: {}", e)))?;

        let res: Result<(), ChainOperationError> = match transaction_response.effects.unwrap() {
            SuiTransactionBlockEffects::V1(t) => {
                match t.status {
                    SuiExecutionStatus::Success => Ok(()),
                    SuiExecutionStatus::Failure {error: e} => {
                        tracing::error!("contract error: {}", e);
                        return Err(ChainOperationError::ExecuteTransactionBlockFailed(format!("Failed to execute_transaction_block: {}", e)));
                    }
                }
            }
        };
                
        if res.is_ok() {
            // Return the successful transaction response
            Ok(TransactionResponse {
                transaction_id: transaction_response.digest.to_string(),
                block_height: None, // Sui uses epochs instead of block heights
                gas_used: Some(self.get_total_balance_change(&transaction_response.balance_changes).await),
                success: true,
            })
        } else {
            Err(res.unwrap_err())
        }
    }

    async fn get_network(&self, name: &str) -> ChainResult<Option<NetworkDetails>> {
        tracing::info!("Getting network '{}' from Sui blockchain", name);
        
        // Validate input
        if name.is_empty() {
            return Err(ChainOperationError::InvalidNetworkName(name.to_string()));
        }
        
        // Connect to Sui testnet
        let sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // Parse object IDs to validate them
        let package_object_id = ObjectID::from_str(&self.package_id)
            .map_err(|e| ChainOperationError::ConnectionError(format!("Invalid package ID: {}", e)))?;
        let registry_object_id = ObjectID::from_str(&self.registry_id)
            .map_err(|e| ChainOperationError::ConnectionError(format!("Invalid registry ID: {}", e)))?;
        
        // Call get_network_info() Move function using inspect_transaction_block
        // This is a read-only call that doesn't require gas or signatures
        let mut ptb = ProgrammableTransactionBuilder::new();
        
        // Add registry object as shared object for the read call
        let registry_arg = CallArg::Object(ObjectArg::SharedObject {
            id: registry_object_id,
            initial_shared_version: SequenceNumber::from(self.registry_version),
            mutable: false, // Read-only access for getting network info
        });
        
        // Add network name argument with proper BCS encoding
        let network_name_arg = CallArg::Pure(bcs::to_bytes(&name.to_string())
            .map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to serialize network name: {}", e)))?);
        
        // Build the Move call to get_network_info function
        let result = ptb.move_call(
            package_object_id,
            "peerlink".parse().unwrap(),
            "get_network_info".parse().unwrap(),
            vec![], // No type arguments
            vec![registry_arg, network_name_arg],
        );
        
        if let Err(e) = result {
            return Err(ChainOperationError::TransactionFailed(format!("Failed to build get_network_info call: {:?}", e)));
        }
        
        let programmable_transaction = ptb.finish();
        
        // Use a dummy sender address for read-only inspection
        let (sui, sender) = self.setup_for_read().await
                                    .map_err(|e| ChainOperationError::SetupReadFailed(format!("Sui Client setup for reading failed: {}", e)))?;
        
        let gas_budget = 10_000_000;
        let gas_price = sui.read_api().get_reference_gas_price().await
            .map_err(|e| ChainOperationError::GetSuiPriceFailed(format!("Failed to get Sui gas price: {:?}", e)))?;

        // Build transaction data for inspection (read-only, no execution)
        let tx_data = TransactionData::new_programmable(
            sender,
            vec![], // No gas coins needed for inspection
            programmable_transaction,
            gas_budget,
            gas_price
        );
                
        // Use dev_inspect_transaction_block for read-only execution
        let inspect_result = sui_client
            .read_api()
            .dev_inspect_transaction_block(
                sender,
                tx_data.kind().clone(),
                Some(gas_price.into()),
                None, // No specific epoch
                None, // No additional options
            )
            .await
            .map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to inspect get_network_info transaction: {}", e)))?;

        // Parse the results from the inspection
        let res = match inspect_result.effects.status() {
            SuiExecutionStatus::Success => Ok(()),
            SuiExecutionStatus::Failure {error: e} => {
                // Check if the error is due to network not found
                if e.contains("E_NETWORK_NOT_FOUND") || e.contains("NETWORK_NOT_FOUND") {
                    tracing::info!("Network '{}' not found on blockchain", name);
                    return Ok(None);
                }
                tracing::error!("contract error: {}", e);
                return Err(ChainOperationError::DevInspectTransactionBlockFailed(format!("Failed to dev_inspect_transaction_block: {}", e)));
            }
        };

        if res.is_err() {
            return Err(res.unwrap_err());
        }

        if let Some(results) = inspect_result.results {
            for result in results.iter() {
                match self.parse_network_info_tuple(&result.return_values, name) {
                    Ok(Some(network_details)) => {
                        tracing::info!("Successfully retrieved network '{}' from contract", name);
                        return Ok(Some(network_details));
                    }
                    Ok(None) => {
                        tracing::info!("Network '{}' not found in parsed results", name);
                        continue;
                    }
                    Err(e) => {
                        tracing::warn!("Failed to parse network info for '{}': {}", name, e);
                        return Err(ChainOperationError::ResultParseError(format!("Failed to parse network info result: {}", e)));
                    }
                }
            }
        }
        
        tracing::info!("Network '{}' not found on blockchain", name);
        Ok(None)
    }

    async fn list_networks(&self) -> ChainResult<Vec<NetworkSummary>> {
        // Connect to Sui testnet
        let sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // Parse object IDs to validate them
        let package_object_id = ObjectID::from_str(&self.package_id)
            .map_err(|e| ChainOperationError::ConnectionError(format!("Invalid package ID: {}", e)))?;
        let registry_object_id = ObjectID::from_str(&self.registry_id)
            .map_err(|e| ChainOperationError::ConnectionError(format!("Invalid registry ID: {}", e)))?;
        
        // Call get_active_networks() Move function using inspect_transaction_block
        // This is a read-only call that doesn't require gas or signatures
        let mut ptb = ProgrammableTransactionBuilder::new();
        
        // Add registry object as shared object for the read call
        let registry_arg = CallArg::Object(ObjectArg::SharedObject {
            id: registry_object_id,
            initial_shared_version: SequenceNumber::from(self.registry_version),
            mutable: false, // Read-only access for listing networks
        });
        
        // Build the Move call to get_active_networks function
        let result = ptb.move_call(
            package_object_id,
            "peerlink".parse().unwrap(),
            "get_active_networks".parse().unwrap(),
            vec![], // No type arguments
            vec![registry_arg],
        );
        
        if let Err(e) = result {
            return Err(ChainOperationError::TransactionFailed(format!("Failed to build get_active_networks call: {:?}", e)));
        }
        
        let programmable_transaction = ptb.finish();
        
        // Use a dummy sender address for read-only inspection
        let (sui, sender) = self.setup_for_read().await
                                    .map_err(|e| ChainOperationError::SetupReadFailed(format!("Sui Client setup for reading failed: {}", e)))?;
        
        let gas_budget = 10_000_000;
        let gas_price = sui.read_api().get_reference_gas_price().await
            .map_err(|e| ChainOperationError::GetSuiPriceFailed(format!("Failed to get Sui gas price: {:?}", e)))?;

        // Build transaction data for inspection (read-only, no execution)
        let tx_data = TransactionData::new_programmable(
            sender,
            vec![], // No gas coins needed for inspection
            programmable_transaction,
            gas_budget,
            gas_price
        );
                
        // Use dev_inspect_transaction_block for read-only execution
        let inspect_result = sui_client
            .read_api()
            .dev_inspect_transaction_block(
                sender,
                tx_data.kind().clone(),
                Some(gas_price.into()),
                None, // No specific epoch
                None, // No additional options
            )
            .await
            .map_err(|e| ChainOperationError::TransactionFailed(format!("Failed to inspect get_active_networks transaction: {}", e)))?;

        // Parse the results from the inspection
        let mut networks = Vec::new();
        
        let res = match inspect_result.effects.status() {
            SuiExecutionStatus::Success => Ok(()),
            SuiExecutionStatus::Failure {error: e} => {
                tracing::error!("contract error: {}", e);
                return Err(ChainOperationError::DevInspectTransactionBlockFailed(format!("Failed to dev_inspect_transaction_block: {}", e)));
            }
        };

        if res.is_err() {
            return Err(res.unwrap_err());
        }

        if let Some(results) = inspect_result.results {
            for (i, result) in results.iter().enumerate() {
                for (j, return_value) in result.return_values.iter().enumerate() {
                    // Handle both empty and non-empty return values
                    if return_value.0.is_empty() {
                        tracing::info!("Empty return value at {}.{} - no networks found", i, j);
                        continue;
                    }
                    
                    match self.parse_network_data(&return_value.0) {
                        Ok(parsed_networks) => {
                            networks.extend(parsed_networks);
                        }
                        Err(e) => {
                            tracing::warn!("Failed to parse network data at {}.{}: {}", i, j, e);
                            return Err(ChainOperationError::ResultParseError(format!("Failed to parse move call result: {}", e)));
                        }
                    }
                }
            }
        }
        
        tracing::info!("Found {} networks from contract", networks.len());
        
        Ok(networks)
    }

    async fn deactivate_network(&self, name: &str) -> ChainResult<TransactionResponse> {
        tracing::info!("Deactivating network '{}' on Sui blockchain", name);
        
        // Connect to Sui testnet
        let _sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // TODO: Implement actual deactivation transaction
        
        Ok(TransactionResponse {
            transaction_id: format!("sui_deactivate_tx_{}", uuid::Uuid::new_v4()),
            block_height: None,
            gas_used: Some(3000),
            success: true,
        })
    }

    async fn reactivate_network(&self, name: &str) -> ChainResult<TransactionResponse> {
        tracing::info!("Reactivating network '{}' on Sui blockchain", name);
        
        // Connect to Sui testnet
        let _sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // TODO: Implement actual reactivation transaction
        
        Ok(TransactionResponse {
            transaction_id: format!("sui_reactivate_tx_{}", uuid::Uuid::new_v4()),
            block_height: None,
            gas_used: Some(3000),
            success: true,
        })
    }

    async fn add_exit_node(&self, network_name: &str, exit_node: ExitNodeInfo) -> ChainResult<TransactionResponse> {
        tracing::info!("Adding exit node {} to network '{}' on Sui blockchain", exit_node.peer_id, network_name);
        
        // Connect to Sui testnet
        let _sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // TODO: Implement actual add_exit_node transaction
        
        Ok(TransactionResponse {
            transaction_id: format!("sui_add_exit_node_tx_{}", uuid::Uuid::new_v4()),
            block_height: None,
            gas_used: Some(4000),
            success: true,
        })
    }

    async fn remove_exit_node(&self, network_name: &str, peer_id: PeerId) -> ChainResult<TransactionResponse> {
        tracing::info!("Removing exit node {} from network '{}' on Sui blockchain", peer_id, network_name);
        
        // Connect to Sui testnet
        let _sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // TODO: Implement actual remove_exit_node transaction
        
        Ok(TransactionResponse {
            transaction_id: format!("sui_remove_exit_node_tx_{}", uuid::Uuid::new_v4()),
            block_height: None,
            gas_used: Some(3500),
            success: true,
        })
    }

    async fn get_exit_nodes(&self, network_name: &str) -> ChainResult<Vec<ExitNodeInfo>> {
        tracing::info!("Getting exit nodes for network '{}' from Sui blockchain", network_name);
        
        // Connect to Sui testnet
        let _sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // TODO: Implement actual exit node retrieval
        
        Ok(vec![])
    }

    async fn update_exit_node_status(&self, network_name: &str, peer_id: PeerId, is_active: bool) -> ChainResult<TransactionResponse> {
        tracing::info!("Updating exit node {} status to {} in network '{}' on Sui blockchain", peer_id, is_active, network_name);
        
        // Connect to Sui testnet
        let _sui_client = SuiClientBuilder::default()
            .build_testnet()
            .await
            .map_err(|e| ChainOperationError::ConnectionError(format!("Failed to connect to Sui testnet: {}", e)))?;
        
        // TODO: Implement actual status update transaction
        
        Ok(TransactionResponse {
            transaction_id: format!("sui_update_exit_node_tx_{}", uuid::Uuid::new_v4()),
            block_height: None,
            gas_used: Some(2500),
            success: true,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_sui_chain_operation_creation() {
        let sui_op = SuiChainOperator::new(
            "test_private_key".to_string(),
            "0x1234567890abcdef".to_string(),
            1u64,
            "test_digest".to_string(),
            "0xabcdef1234567890".to_string()
        );
        
        assert_eq!(sui_op.package_id, "0x1234567890abcdef".to_string());
        assert_eq!(sui_op.registry_id, "0xabcdef1234567890".to_string());
        assert_eq!(sui_op.private_key, "test_private_key".to_string());
        assert_eq!(sui_op.registry_version, 1u64);
        assert_eq!(sui_op.registry_digest, "test_digest".to_string());
    }

    #[tokio::test]
    async fn test_create_network_validation() {
        let sui_op = SuiChainOperator::new(
            "test_private_key".to_string(),
            "0x1234567890abcdef".to_string(),
            1u64,
            "test_digest".to_string(),
            "0xabcdef1234567890".to_string()
        );
        
        // Test empty network name
        let result = sui_op.create_network("".to_string(), "valid_secret".to_string(), None).await;
        assert!(result.is_err());
        
        // Test short secret
        let result = sui_op.create_network("test_network".to_string(), "short".to_string(), Some("Test description".to_string())).await;
        assert!(result.is_err());
        
        // Test valid input (should succeed with proper configuration)
        let result = sui_op.create_network("test_network".to_string(), "valid_secret_123".to_string(), Some("Valid network".to_string())).await;
        assert!(result.is_ok()); // Should succeed now that all configuration is provided
    }
}