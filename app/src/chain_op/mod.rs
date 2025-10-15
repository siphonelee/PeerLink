use std::collections::HashMap;

use crate::common::PeerId;

/// Result type for chain operations
pub type ChainResult<T> = Result<T, ChainOperationError>;

/// Error types for chain operations
#[derive(Debug, Clone)]
pub enum ChainOperationError {
    NetworkCreationFailed(String),
    NetworkAlreadyExists(String),
    InvalidSecret(String),
    InvalidNetworkName(String),
    
    ExecuteTransactionBlockFailed(String),
    DevInspectTransactionBlockFailed(String),
    TransactionFailed(String),
    InvalidPrivateKey(String),
    ConnectionError(String),
    InsufficientFunds(String),
    SetupWriteFailed(String),
    SetupReadFailed(String),
    GetSuiPriceFailed(String),
    SuiConfigDirError(String),
    SuiLoadKeyStoreError(String),
    SuiSignTransError(String),
    ResultParseError(String),
    Other(String),
}

impl std::fmt::Display for ChainOperationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainOperationError::NetworkCreationFailed(msg) => write!(f, "Network creation failed: {}", msg),
            ChainOperationError::NetworkAlreadyExists(name) => write!(f, "Network '{}' already exists", name),
            ChainOperationError::InvalidSecret(msg) => write!(f, "Invalid secret: {}", msg),
            ChainOperationError::InvalidNetworkName(name) => write!(f, "Invalid network name: {}", name),
            ChainOperationError::ExecuteTransactionBlockFailed(msg) => write!(f, "Executing Sui transaction block failed: {}", msg),
            ChainOperationError::DevInspectTransactionBlockFailed(msg) => write!(f, "Dev-inspecting Sui transaction block failed: {}", msg),
            ChainOperationError::TransactionFailed(msg) => write!(f, "Transaction failed: {}", msg),
            ChainOperationError::InvalidPrivateKey(msg) => write!(f, "Invalid wallet private key: {}", msg),
            ChainOperationError::ConnectionError(msg) => write!(f, "Connection error: {}", msg),
            ChainOperationError::InsufficientFunds(msg) => write!(f, "Insufficient funds: {}", msg),
            ChainOperationError::SetupWriteFailed(msg) => write!(f, "Setup Sui client for writing failed: {}", msg),
            ChainOperationError::SetupReadFailed(msg) => write!(f, "Setup Sui client for reading failed: {}", msg),
            ChainOperationError::GetSuiPriceFailed(msg) => write!(f, "Get Sui price failed: {}", msg),
            ChainOperationError::SuiConfigDirError(msg) => write!(f, "Get Sui config dir error: {}", msg),
            ChainOperationError::SuiLoadKeyStoreError(msg) => write!(f, "Load Sui key store error: {}", msg),
            ChainOperationError::SuiSignTransError(msg) => write!(f, "Failed to sign Sui transaction: {}", msg),
            ChainOperationError::ResultParseError(msg) => write!(f, "Failed to parse move call result: {}", msg),
            ChainOperationError::Other(msg) => write!(f, "Chain operation error: {}", msg),
        }
    }
}

impl std::error::Error for ChainOperationError {}

/// Network information structure for chain operations
#[derive(Debug, Clone)]
pub struct NetworkInfo {
    pub name: String,
    pub secret: String,
    pub admin: String,
    pub created_at: u64,
    pub is_active: bool,
    pub description: Option<String>,
    pub max_peers: Option<u64>,
    pub exit_nodes: Vec<ExitNodeInfo>,
}

#[derive(Debug, Clone, tabled::Tabled, serde::Serialize)]
pub struct NetworkSummary {
    pub name: String,
    pub admin: String,
    pub created_at: u64,
    pub description: String,
    pub max_peers: u64,
}

#[derive(Debug, Clone)]
pub struct NetworkDetails {
    pub name: String,
    pub secret: String,
    pub admin: String,
    pub created_at: u64,
    pub is_active: bool,
    pub description: Option<String>,
    pub max_peers: u64,
    pub total_exit_nodes: u64,
}

/// Exit node information for networks
#[derive(Debug, Clone)]
pub struct ExitNodeInfo {
    pub peer_id: PeerId,
    pub connector_uri_list: Vec<String>,
    pub hostname: String,
    pub is_active: bool,
    pub registered_at: u64,
}

/// Transaction response information
#[derive(Debug, Clone)]
pub struct TransactionResponse {
    pub transaction_id: String,
    pub block_height: Option<u64>,
    pub gas_used: Option<i128>,
    pub success: bool,
}

/// Interface for blockchain chain operations
/// This trait defines the common operations that can be performed on different blockchains
/// for managing peerlink networks
#[async_trait::async_trait]
pub trait ChainOperationInterface: Send + Sync {
    /// Create a new network on the blockchain
    /// 
    /// # Arguments
    /// * `name` - The unique name of the network
    /// * `secret` - The network secret for peer authentication
    /// * `description` - Optional description for the network
    /// 
    /// # Returns
    /// * `ChainResult<TransactionResponse>` - Transaction details if successful
    async fn create_network(&self, name: String, secret: String, description: Option<String>) -> ChainResult<TransactionResponse>;

    /// Get network information by name
    /// 
    /// # Arguments
    /// * `name` - The name of the network
    /// 
    /// # Returns
    /// * `ChainResult<Option<NetworkInfo>>` - Network info if found
    async fn get_network(&self, name: &str) -> ChainResult<Option<NetworkDetails>>;

    /// List all networks
    /// 
    /// # Returns
    /// * `ChainResult<Vec<NetworkInfo>>` - List of all networks
    async fn list_networks(&self) -> ChainResult<Vec<NetworkSummary>>;

    /// Deactivate a network (only admin can do this)
    /// 
    /// # Arguments
    /// * `name` - The name of the network to deactivate
    /// 
    /// # Returns
    /// * `ChainResult<TransactionResponse>` - Transaction details if successful
    async fn deactivate_network(&self, name: &str) -> ChainResult<TransactionResponse>;

    /// Reactivate a network (only admin can do this)
    /// 
    /// # Arguments
    /// * `name` - The name of the network to reactivate
    /// 
    /// # Returns
    /// * `ChainResult<TransactionResponse>` - Transaction details if successful
    async fn reactivate_network(&self, name: &str) -> ChainResult<TransactionResponse>;

    /// Add an exit node to a network
    /// 
    /// # Arguments
    /// * `network_name` - The name of the network
    /// * `exit_node` - Exit node information
    /// 
    /// # Returns
    /// * `ChainResult<TransactionResponse>` - Transaction details if successful
    async fn add_exit_node(&self, network_name: &str, exit_node: ExitNodeInfo) -> ChainResult<TransactionResponse>;

    /// Remove an exit node from a network
    /// 
    /// # Arguments
    /// * `network_name` - The name of the network
    /// * `peer_id` - The peer ID of the exit node to remove
    /// 
    /// # Returns
    /// * `ChainResult<TransactionResponse>` - Transaction details if successful
    async fn remove_exit_node(&self, network_name: &str, peer_id: PeerId) -> ChainResult<TransactionResponse>;

    /// Get all exit nodes for a network
    /// 
    /// # Arguments
    /// * `network_name` - The name of the network
    /// 
    /// # Returns
    /// * `ChainResult<Vec<ExitNodeInfo>>` - List of active exit nodes
    async fn get_exit_nodes(&self, network_name: &str) -> ChainResult<Vec<ExitNodeInfo>>;

    /// Pick a random exit node from a network
    /// 
    /// # Arguments
    /// * `network_name` - The name of the network
    /// 
    /// # Returns
    /// * `ChainResult<Option<ExitNodeInfo>>` - Selected exit node if any available
    async fn pick_exit_node(&self, network_name: &str) -> ChainResult<Option<ExitNodeInfo>>;

    /// Update exit node status
    /// 
    /// # Arguments
    /// * `network_name` - The name of the network
    /// * `peer_id` - The peer ID of the exit node
    /// * `is_active` - New active status
    /// 
    /// # Returns
    /// * `ChainResult<TransactionResponse>` - Transaction details if successful
    async fn update_exit_node_status(&self, network_name: &str, peer_id: PeerId, is_active: bool) -> ChainResult<TransactionResponse>;

    /// Get network secret for authorized access
    /// 
    /// # Arguments
    /// * `network_name` - The name of the network
    /// 
    /// # Returns
    /// * `ChainResult<Vec<u8>>` - Network secret as bytes if authorized
    async fn get_network_secret(&self, network_name: &str) -> ChainResult<Vec<u8>>;
}

/// Type alias for boxed chain operation interface
pub type ChainOperationBox = Box<dyn ChainOperationInterface>;

/// Mock implementation for testing
pub struct MockChainOperation {
    pub blockchain_name: String,
    pub networks: std::sync::Mutex<HashMap<String, NetworkInfo>>,
}

impl MockChainOperation {
    pub fn new(blockchain_name: String) -> Self {
        Self {
            blockchain_name,
            networks: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

#[async_trait::async_trait]
impl ChainOperationInterface for MockChainOperation {
    async fn create_network(&self, name: String, secret: String, description: Option<String>) -> ChainResult<TransactionResponse> {
        let mut networks = self.networks.lock().unwrap();
        
        if networks.contains_key(&name) {
            return Err(ChainOperationError::NetworkAlreadyExists(name));
        }

        let network = NetworkInfo {
            name: name.clone(),
            secret,
            admin: "mock_admin".to_string(),
            created_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs(),
            is_active: true,
            description,
            max_peers: None,
            exit_nodes: vec![],
        };

        networks.insert(name, network);

        Ok(TransactionResponse {
            transaction_id: format!("mock_tx_{}", uuid::Uuid::new_v4()),
            block_height: Some(12345),
            gas_used: Some(1000),
            success: true,
        })
    }

    async fn get_network(&self, name: &str) -> ChainResult<Option<NetworkDetails>> {
        let networks = self.networks.lock().unwrap();
        Ok(networks.get(name).map(|network| NetworkDetails {
            name: network.name.clone(),
            secret: network.secret.clone(),
            admin: network.admin.clone(),
            created_at: network.created_at,
            is_active: network.is_active,
            description: network.description.clone(),
            max_peers: network.max_peers.unwrap_or(0),
            total_exit_nodes: network.exit_nodes.len() as u64,
        }))
    }

    async fn list_networks(&self) -> ChainResult<Vec<NetworkSummary>> {
        let networks = self.networks.lock().unwrap();
        Ok(networks.values().map(|network| NetworkSummary {
            name: network.name.clone(),
            admin: network.admin.clone(),
            created_at: network.created_at,
            description: network.description.clone().unwrap_or_default(),
            max_peers: network.max_peers.unwrap_or(0),
        }).collect())
    }

    async fn deactivate_network(&self, name: &str) -> ChainResult<TransactionResponse> {
        let mut networks = self.networks.lock().unwrap();
        if let Some(network) = networks.get_mut(name) {
            network.is_active = false;
            Ok(TransactionResponse {
                transaction_id: format!("mock_deactivate_tx_{}", uuid::Uuid::new_v4()),
                block_height: Some(12346),
                gas_used: Some(500),
                success: true,
            })
        } else {
            Err(ChainOperationError::Other(format!("Network '{}' not found", name)))
        }
    }

    async fn reactivate_network(&self, name: &str) -> ChainResult<TransactionResponse> {
        let mut networks = self.networks.lock().unwrap();
        if let Some(network) = networks.get_mut(name) {
            network.is_active = true;
            Ok(TransactionResponse {
                transaction_id: format!("mock_reactivate_tx_{}", uuid::Uuid::new_v4()),
                block_height: Some(12347),
                gas_used: Some(500),
                success: true,
            })
        } else {
            Err(ChainOperationError::Other(format!("Network '{}' not found", name)))
        }
    }

    async fn add_exit_node(&self, network_name: &str, exit_node: ExitNodeInfo) -> ChainResult<TransactionResponse> {
        let mut networks = self.networks.lock().unwrap();
        if let Some(network) = networks.get_mut(network_name) {
            network.exit_nodes.push(exit_node);
            Ok(TransactionResponse {
                transaction_id: format!("mock_add_exit_node_tx_{}", uuid::Uuid::new_v4()),
                block_height: Some(12348),
                gas_used: Some(800),
                success: true,
            })
        } else {
            Err(ChainOperationError::Other(format!("Network '{}' not found", network_name)))
        }
    }

    async fn remove_exit_node(&self, network_name: &str, peer_id: PeerId) -> ChainResult<TransactionResponse> {
        let mut networks = self.networks.lock().unwrap();
        if let Some(network) = networks.get_mut(network_name) {
            network.exit_nodes.retain(|node| node.peer_id != peer_id);
            Ok(TransactionResponse {
                transaction_id: format!("mock_remove_exit_node_tx_{}", uuid::Uuid::new_v4()),
                block_height: Some(12349),
                gas_used: Some(600),
                success: true,
            })
        } else {
            Err(ChainOperationError::Other(format!("Network '{}' not found", network_name)))
        }
    }

    async fn get_exit_nodes(&self, network_name: &str) -> ChainResult<Vec<ExitNodeInfo>> {
        let networks = self.networks.lock().unwrap();
        if let Some(network) = networks.get(network_name) {
            Ok(network.exit_nodes.iter().filter(|node| node.is_active).cloned().collect())
        } else {
            Err(ChainOperationError::Other(format!("Network '{}' not found", network_name)))
        }
    }

    async fn pick_exit_node(&self, network_name: &str) -> ChainResult<Option<ExitNodeInfo>> {
        let exit_nodes = self.get_exit_nodes(network_name).await?;
        if exit_nodes.is_empty() {
            Ok(None)
        } else {
            // Simple mock random selection using current timestamp
            let index = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos() % exit_nodes.len() as u128) as usize;
            Ok(Some(exit_nodes[index].clone()))
        }
    }

    async fn update_exit_node_status(&self, network_name: &str, peer_id: PeerId, is_active: bool) -> ChainResult<TransactionResponse> {
        let mut networks = self.networks.lock().unwrap();
        if let Some(network) = networks.get_mut(network_name) {
            if let Some(node) = network.exit_nodes.iter_mut().find(|node| node.peer_id == peer_id) {
                node.is_active = is_active;
                Ok(TransactionResponse {
                    transaction_id: format!("mock_update_exit_node_tx_{}", uuid::Uuid::new_v4()),
                    block_height: Some(12350),
                    gas_used: Some(400),
                    success: true,
                })
            } else {
                Err(ChainOperationError::Other(format!("Exit node with peer_id {} not found", peer_id)))
            }
        } else {
            Err(ChainOperationError::Other(format!("Network '{}' not found", network_name)))
        }
    }

    async fn get_network_secret(&self, network_name: &str) -> ChainResult<Vec<u8>> {
        let networks = self.networks.lock().unwrap();
        if let Some(network) = networks.get(network_name) {
            Ok(network.secret.as_bytes().to_vec())
        } else {
            Err(ChainOperationError::Other(format!("Network '{}' not found", network_name)))
        }
    }
}

pub mod sui;