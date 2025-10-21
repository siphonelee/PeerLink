# PeerLink

[![Rust](https://img.shields.io/badge/rust-1.89.0+-blue.svg)](https://www.rust-lang.org)

**PeerLink** is a next-generation peer-to-peer VPN solution that creates a full-mesh network topology with a blockchain-based control plane powered by the Sui network. Connect all your devices seamlessly across the internet with a single command, enabling secure, decentralized networking without traditional VPN servers.

## 🌟 Key Features

- **🕸️ Full Mesh P2P VPN**: Every device connects directly to every other device for optimal performance
- **🔗 Sui Blockchain Control Plane**: Decentralized network management and peer discovery via Sui smart contracts
- **🌐 Exit Node Support**: Route internet traffic through designated exit nodes for enhanced privacy
- **🔐 End-to-End Encryption**: All traffic is encrypted using modern cryptographic protocols
- **📱 Cross-Platform**: Supports Win/MacOS/Linux/FreeBSD/Android and X86/ARM/MIPS architectures
- **⚡ High Performance**: Optimized for low latency and high throughput. Zero-copy throughout the entire link (TCP/UDP/WSS/WG protocols)
- **🛡️ NAT Traversal**: Automatic NAT and firewall traversal for seamless connectivity supporting UDP and IPv6 traversal

## 🏗️ Architecture

PeerLink consists of two main components:

### 📦 Application Layer (`/app`)
- **peerlink-core**: The main VPN daemon that handles peer connections, routing, and traffic forwarding
- **peerlink-cli**: Command-line interface for network management and configuration
- **Gateway Components**: TCP/UDP proxies with NAT for internet routing via exit nodes
- **Peer Management**: OSPF-like routing protocol for optimal path selection

### 🔗 Smart Contract Layer (`/contract`)
- **Sui Move Contracts**: Blockchain-based network registry and peer discovery
- **Network Management**: Decentralized creation and management of VPN networks
- **Exit Node Registry**: Blockchain-managed exit node discovery and selection

## 🚀 Quick Start

### Prerequisites

- Rust 1.89.0 or later
- Sui CLI tools (for blockchain features)
- Platform-specific network permissions (root/administrator access)

### Installation

1. **Clone the repository:**
   ```bash
   git clone https://github.com/siphonelee/peerlink.git
   cd peerlink
   ```

2. **Build the application:**
   ```bash
   cd app
   cargo build --release
   ```

3. **Install binaries:**
   ```bash
   # Linux/macOS
   sudo cp target/release/peerlink-core /usr/local/bin/
   sudo cp target/release/peerlink-cli /usr/local/bin/
   
   # Or add to PATH
   export PATH=$PATH:$(pwd)/target/release
   ```

### Basic Usage

1. **Configure Sui blockchain credentials:**
   ```bash
   cd app
   cp .sui.env.example .sui.env
   # Edit .sui.env with your Sui credentials
   ```

2. **Create or join a network:**
   ```bash
   # Create a new network
   peerlink-cli network create my-network secret123 -d "My private network"
   
   # Or join an existing network
   peerlink-core --network-name my-network --network-secret secret123
   ```

3. **Start the VPN daemon:**
   ```bash
   sudo peerlink-core --config-file config.toml
   ```

4. **Manage exit nodes:**
   ```bash
   # List available exit nodes
   peerlink-cli network exit-nodes my-network   
   ```

## 📖 Documentation

### Network Management Commands

```bash
# List all networks
peerlink-cli network list

# Get network information
peerlink-cli network info <network-name>

# Get network secret (for authorized users)
peerlink-cli network secret <network-name>

# Exit node management
peerlink-cli network exit-nodes <network-name>
peerlink-cli network remove-exit-node <network-name> <peer-id>
```

### Configuration

PeerLink uses TOML configuration files. See `app/peerlink.toml.example` for a complete configuration template.

Key configuration sections:
- **Network settings**: IP ranges, interface configuration
- **Sui blockchain**: Contract addresses and credentials
- **Security**: Encryption settings and access control
- **Performance**: Connection timeouts and buffer sizes

## 🔧 Development

### Project Structure

```
peerlink/
├── app/                    # Main application code
│   ├── src/
│   │   ├── peerlink-core.rs   # VPN daemon
│   │   ├── peerlink-cli.rs    # CLI tool
│   │   ├── gateway/           # Traffic routing and NAT
│   │   ├── peers/             # Peer management and routing
│   │   ├── tunnel/            # Network tunneling protocols
│   │   └── chain_op/          # Blockchain operations
│   └── Cargo.toml
├── contract/               # Sui smart contracts
│   └── sui_move/
└── README.md
```

### Building for Different Platforms

```bash
# macOS
cargo build --release --target x86_64-apple-darwin

# Linux
cargo build --release --target x86_64-unknown-linux-gnu

# Windows
cargo build --release --target x86_64-pc-windows-gnu
```

### Running Tests

```bash
cd app
cargo test
```

## 🌐 How It Works

### Peer Discovery
1. Networks are registered on the Sui blockchain
2. Peers discover each other through blockchain queries
3. Direct P2P connections are established using STUN/TURN protocols

### Traffic Routing
1. **Direct P2P**: Traffic between peers flows directly through encrypted tunnels
2. **Exit Node Routing**: Internet traffic is routed through designated exit nodes
3. **NAT Traversal**: Automatic hole punching for connections behind NAT/firewalls

### Security Model
- All peer communications are end-to-end encrypted
- Blockchain provides tamper-proof network configuration
- No central servers that can be compromised
- Exit nodes provide optional internet access without compromising peer privacy

## 🤝 Contributing

We welcome contributions! 

### Development Setup

1. Install Rust and Sui CLI tools
2. Fork and clone the repository
3. Create a feature branch
4. Make your changes and add tests
5. Submit a pull request

