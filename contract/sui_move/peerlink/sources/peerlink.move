/// Module: peerlink
module peerlink::peerlink;

use sui::object::{Self, UID};
use sui::tx_context::{Self, TxContext};
use sui::transfer;
use sui::table::{Self, Table};
use std::string::{Self, String};
use std::vector;
use sui::clock::{Self, Clock};
use sui::event;
use std::option::{Self, Option};

/// Error codes
const E_NETWORK_NOT_FOUND: u64 = 1;
const E_NETWORK_ALREADY_EXISTS: u64 = 2;
const E_UNAUTHORIZED: u64 = 3;
const E_INVALID_NETWORK_NAME: u64 = 4;
const E_NETWORK_EXPIRED: u64 = 5;
const E_PEER_NOT_AUTHORIZED: u64 = 6;
const E_PEER_NOT_FOUND: u64 = 7;

/// Exit node information
public struct ExitNodeInfo has copy, drop, store {
    /// Exit node peer ID
    peer_id: u32,
    /// List of connector URIs for this exit node
    connector_uri_list: vector<String>,
    /// Hostname for the exit node
    hostname: String,
    /// Whether this exit node is currently active
    is_active: bool,
    /// Registration timestamp
    registered_at: u64,
}

/// Network information structure
public struct NetworkInfo has key, store {
    id: UID,
    /// Network name identifier
    network_name: String,
    /// Network secret (plaintext, readable by authorized peers)
    network_secret: vector<u8>,
    /// Network administrator address
    admin: address,
    /// Network creation timestamp
    created_at: u64,
    /// Whether the network is active
    is_active: bool,
    /// Network description
    description: String,
    /// Maximum number of peers allowed
    max_peers: u64,
    /// Table of registered exit nodes (peer_id -> ExitNodeInfo)
    exit_nodes: Table<u32, ExitNodeInfo>,
    /// List of exit node peer IDs for iteration
    exit_node_peer_ids: vector<u32>,
}

/// Network summary for public listings
public struct NetworkSummary has copy, drop, store {
    network_name: String,
    admin: String,
    created_at: u64,
    description: String,
    max_peers: u64,
}

/// Global network registry
public struct NetworkRegistry has key {
    id: UID,
    /// network name -> NetworkInfo
    networks: Table<String, NetworkInfo>,
    /// List of all network names for iteration
    network_names: vector<String>,
    /// Global admin (can manage the registry itself)
    global_admin: address,
}

/// Events
public struct NetworkCreated has copy, drop {
    network_name: String,
    admin: address,
    created_at: u64,
}

public struct NetworkUpdated has copy, drop {
    network_name: String,
    admin: address,
    updated_at: u64,
}

/// Initialize the network registry
fun init(ctx: &mut TxContext) {
    let registry = NetworkRegistry {
        id: object::new(ctx),
        networks: table::new(ctx),
        network_names: vector::empty<String>(),
        global_admin: tx_context::sender(ctx),
    };
    transfer::share_object(registry);
}

/// Create a new network
public entry fun create_network(
    registry: &mut NetworkRegistry,
    network_name: String,
    network_secret: vector<u8>,
    description: String,
    max_peers: u64,
    clock: &Clock,
    ctx: &mut TxContext
) {
    let sender = tx_context::sender(ctx);
    let current_time = clock::timestamp_ms(clock);
    
    // Validate network name is not empty
    assert!(!string::is_empty(&network_name), E_INVALID_NETWORK_NAME);
    
    // Check if network already exists
    assert!(!table::contains(&registry.networks, network_name), E_NETWORK_ALREADY_EXISTS);
    
    let network_info = NetworkInfo {
        id: object::new(ctx),
        network_name: network_name,
        network_secret,
        admin: sender,
        created_at: current_time,
        is_active: true,
        description,
        max_peers,
        exit_nodes: table::new<u32, ExitNodeInfo>(ctx),
        exit_node_peer_ids: vector::empty<u32>(),
    };
    
    // Add network to registry
    table::add(&mut registry.networks, network_name, network_info);
    vector::push_back(&mut registry.network_names, network_name);
    
    // Emit event
    event::emit(NetworkCreated {
        network_name,
        admin: sender,
        created_at: current_time,
    });
}

/// Update network information (only admin can do this)
public entry fun update_network(
    registry: &mut NetworkRegistry,
    network_name: String,
    new_network_secret: vector<u8>,
    new_description: String,
    new_max_peers: u64,
    clock: &Clock,
    ctx: &mut TxContext
) {
    let sender = tx_context::sender(ctx);
    let current_time = clock::timestamp_ms(clock);
    
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow_mut(&mut registry.networks, network_name);
    assert!(network_info.admin == sender, E_UNAUTHORIZED);
    
    // Update network information
    network_info.network_secret = new_network_secret;
    network_info.description = new_description;
    network_info.max_peers = new_max_peers;
    
    // Emit event
    event::emit(NetworkUpdated {
        network_name,
        admin: sender,
        updated_at: current_time,
    });
}

/// Add exit node to network (only admin can do this)
public entry fun add_exit_node(
    registry: &mut NetworkRegistry,
    network_name: String,
    peer_id: u32,
    connector_uri_list: vector<String>,
    hostname: String,
    clock: &Clock,
    ctx: &mut TxContext
) {
    let sender = tx_context::sender(ctx);
    let current_time = clock::timestamp_ms(clock);
    
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow_mut(&mut registry.networks, network_name);
    assert!(network_info.admin == sender, E_UNAUTHORIZED);
    
    let exit_node_info = ExitNodeInfo {
        peer_id,
        connector_uri_list,
        hostname,
        is_active: true,
        registered_at: current_time,
    };
    
    // Add or update exit node
    if (table::contains(&network_info.exit_nodes, peer_id)) {
        let existing_node = table::borrow_mut(&mut network_info.exit_nodes, peer_id);
        *existing_node = exit_node_info;
    } else {
        table::add(&mut network_info.exit_nodes, peer_id, exit_node_info);
        vector::push_back(&mut network_info.exit_node_peer_ids, peer_id);
    };
}

/// Remove exit node from network (only admin can do this)
public entry fun remove_exit_node(
    registry: &mut NetworkRegistry,
    network_name: String,
    peer_id: u32,
    ctx: &mut TxContext
) {
    let sender = tx_context::sender(ctx);
    
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow_mut(&mut registry.networks, network_name);
    assert!(network_info.admin == sender, E_UNAUTHORIZED);
    
    // Remove exit node if it exists
    if (table::contains(&network_info.exit_nodes, peer_id)) {
        table::remove(&mut network_info.exit_nodes, peer_id);
        
        // Remove from peer_ids list
        let (found, index) = vector::index_of(&network_info.exit_node_peer_ids, &peer_id);
        if (found) {
            vector::remove(&mut network_info.exit_node_peer_ids, index);
        };
    };
}

/// Get exit nodes for a network
public fun get_exit_nodes(
    registry: &NetworkRegistry,
    network_name: String
): vector<ExitNodeInfo> {
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow(&registry.networks, network_name);
    let mut result = vector::empty<ExitNodeInfo>();
    let mut i = 0;
    let len = vector::length(&network_info.exit_node_peer_ids);
    
    while (i < len) {
        let peer_id = *vector::borrow(&network_info.exit_node_peer_ids, i);
        let exit_node = table::borrow(&network_info.exit_nodes, peer_id);
        
        // Only include active exit nodes
        if (exit_node.is_active) {
            vector::push_back(&mut result, *exit_node);
        };
        
        i = i + 1;
    };
    
    result
}

/// Randomly select an active exit node from a network
public fun pick_exit_node(
    registry: &NetworkRegistry,
    network_name: String,
    ctx: &TxContext
): Option<ExitNodeInfo> {
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow(&registry.networks, network_name);
    let mut active_exit_nodes = vector::empty<ExitNodeInfo>();
    let mut i = 0;
    let len = vector::length(&network_info.exit_node_peer_ids);
    
    // Collect all active exit nodes
    while (i < len) {
        let peer_id = *vector::borrow(&network_info.exit_node_peer_ids, i);
        let exit_node = table::borrow(&network_info.exit_nodes, peer_id);
        
        if (exit_node.is_active) {
            vector::push_back(&mut active_exit_nodes, *exit_node);
        };
        
        i = i + 1;
    };
    
    let active_count = vector::length(&active_exit_nodes);
    if (active_count == 0) {
        return option::none<ExitNodeInfo>()
    };
    
    // Use transaction context for pseudo-randomness
    let tx_hash = tx_context::digest(ctx);
    let random_bytes = std::hash::sha3_256(*tx_hash);
    
    // Use first few bytes to create a simple random number
    let mut random_value: u64 = 0;
    if (vector::length(&random_bytes) > 0) {
        random_value = (*vector::borrow(&random_bytes, 0) as u64);
        if (vector::length(&random_bytes) > 1) {
            random_value = random_value * 256 + (*vector::borrow(&random_bytes, 1) as u64);
        };
        if (vector::length(&random_bytes) > 2) {
            random_value = random_value * 256 + (*vector::borrow(&random_bytes, 2) as u64);
        };
    };
    
    let selected_index = random_value % active_count;
    option::some(*vector::borrow(&active_exit_nodes, selected_index))
}


/// Get all active networks
public fun get_active_networks(registry: &NetworkRegistry): vector<NetworkSummary> {
    let mut result = vector::empty<NetworkSummary>();
    let mut i = 0;
    let len = vector::length(&registry.network_names);
    
    while (i < len) {
        let network_name = *vector::borrow(&registry.network_names, i);
        let network_info = table::borrow(&registry.networks, network_name);
        
        // Only include active networks
        if (network_info.is_active) {
            let summary = NetworkSummary {
                network_name: network_info.network_name,
                admin: network_info.admin.to_string(),
                created_at: network_info.created_at,
                description: network_info.description,
                max_peers: network_info.max_peers,
            };
            vector::push_back(&mut result, summary);
        };
        
        i = i + 1;
    };
    
    result
}

/// Get network information
public fun get_network_info(
    registry: &NetworkRegistry,
    network_name: String
): (String, address, u64, bool, String, u64, u64) {
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow(&registry.networks, network_name);
    (
        network_info.network_name,
        network_info.admin,
        network_info.created_at,
        network_info.is_active,
        network_info.description,
        network_info.max_peers,
        network_info.exit_node_peer_ids.length(),
    )
}

/// Get network secret
public fun get_network_secret(
    registry: &NetworkRegistry,
    network_name: String
): vector<u8> {
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow(&registry.networks, network_name);
    assert!(network_info.is_active, E_NETWORK_EXPIRED);
    
    network_info.network_secret
}

/// Deactivate a network (only admin can do this)
public entry fun deactivate_network(
    registry: &mut NetworkRegistry,
    network_name: String,
    ctx: &mut TxContext
) {
    let sender = tx_context::sender(ctx);
    
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow_mut(&mut registry.networks, network_name);
    assert!(network_info.admin == sender, E_UNAUTHORIZED);
    
    network_info.is_active = false;
}

/// Reactivate a network (only admin can do this)
public entry fun reactivate_network(
    registry: &mut NetworkRegistry,
    network_name: String,
    ctx: &mut TxContext
) {
    let sender = tx_context::sender(ctx);
    
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow_mut(&mut registry.networks, network_name);
    assert!(network_info.admin == sender, E_UNAUTHORIZED);
    
    network_info.is_active = true;
}

/// Get specific exit node by peer ID
public fun get_exit_node_by_peer_id(
    registry: &NetworkRegistry,
    network_name: String,
    peer_id: u32
): Option<ExitNodeInfo> {
    if (!table::contains(&registry.networks, network_name)) {
        return option::none<ExitNodeInfo>()
    };
    
    let network_info = table::borrow(&registry.networks, network_name);
    
    if (table::contains(&network_info.exit_nodes, peer_id)) {
        option::some(*table::borrow(&network_info.exit_nodes, peer_id))
    } else {
        option::none<ExitNodeInfo>()
    }
}

/// Update exit node status (active/inactive)
public entry fun update_exit_node_status(
    registry: &mut NetworkRegistry,
    network_name: String,
    peer_id: u32,
    is_active: bool,
    _ctx: &mut TxContext
) {
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow_mut(&mut registry.networks, network_name);
    assert!(table::contains(&network_info.exit_nodes, peer_id), E_PEER_NOT_FOUND);
    
    let exit_node = table::borrow_mut(&mut network_info.exit_nodes, peer_id);
    exit_node.is_active = is_active;
}

/// Get all exit node peer IDs for a network
public fun get_exit_node_peer_ids(
    registry: &NetworkRegistry,
    network_name: String
): vector<u32> {
    assert!(table::contains(&registry.networks, network_name), E_NETWORK_NOT_FOUND);
    
    let network_info = table::borrow(&registry.networks, network_name);
    network_info.exit_node_peer_ids
}

