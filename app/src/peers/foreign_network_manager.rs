/*
foreign_network_manager is used to forward packets of other networks.  currently
only forward packets of peers that directly connected to this node.

in the future, with the help wo peer center we can forward packets of peers that
connected to any node in the local network.
*/
use std::{
    sync::{Arc, Weak},
    time::SystemTime,
};

use dashmap::{DashMap, DashSet};
use tokio::{
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        Mutex,
    },
    task::JoinSet,
};

use crate::{
    common::{
        config::{ConfigLoader, TomlConfigLoader},
        error::Error,
        global_ctx::{ArcGlobalCtx, GlobalCtx, GlobalCtxEvent, NetworkIdentity},
        join_joinset_background,
        stats_manager::{LabelSet, LabelType, MetricName, StatsManager},
        token_bucket::TokenBucket,
        PeerId,
    },
    peer_center::instance::{PeerCenterInstance, PeerMapWithPeerRpcManager},
    peers::route_trait::{Route, RouteInterface},
    proto::{
        cli::{ForeignNetworkEntryPb, ListForeignNetworkResponse, PeerInfo},
        common::LimiterConfig,
        peer_rpc::DirectConnectorRpcServer,
    },
    tunnel::packet_def::{PacketType, ZCPacket},
    use_global_var,
};

use super::{
    create_packet_recv_chan,
    peer_conn::PeerConn,
    peer_map::PeerMap,
    peer_ospf_route::PeerRoute,
    peer_rpc::{PeerRpcManager, PeerRpcManagerTransport},
    peer_rpc_service::DirectConnectorManagerRpcServer,
    recv_packet_from_chan,
    route_trait::NextHopPolicy,
    PacketRecvChan, PacketRecvChanReceiver, PUBLIC_SERVER_HOSTNAME_PREFIX,
};

#[async_trait::async_trait]
#[auto_impl::auto_impl(&, Box, Arc)]
pub trait GlobalForeignNetworkAccessor: Send + Sync + 'static {
    async fn list_global_foreign_peer(&self, network_identity: &NetworkIdentity) -> Vec<PeerId>;
}

struct ForeignNetworkEntry {
    my_peer_id: PeerId,

    global_ctx: ArcGlobalCtx,
    network: NetworkIdentity,
    peer_map: Arc<PeerMap>,
    relay_data: bool,
    pm_packet_sender: Mutex<Option<PacketRecvChan>>,

    peer_rpc: Arc<PeerRpcManager>,
    rpc_sender: UnboundedSender<ZCPacket>,

    packet_recv: Mutex<Option<PacketRecvChanReceiver>>,

    bps_limiter: Arc<TokenBucket>,

    peer_center: Arc<PeerCenterInstance>,

    stats_mgr: Arc<StatsManager>,

    tasks: Mutex<JoinSet<()>>,

    pub lock: Mutex<()>,
}

impl ForeignNetworkEntry {
    fn new(
        network: NetworkIdentity,
        // NOTICE: ospf route need my_peer_id be changed after restart.
        my_peer_id: PeerId,
        global_ctx: ArcGlobalCtx,
        relay_data: bool,
        pm_packet_sender: PacketRecvChan,
    ) -> Self {
        let stats_mgr = global_ctx.stats_manager().clone();
        let foreign_global_ctx = Self::build_foreign_global_ctx(&network, global_ctx.clone());

        let (packet_sender, packet_recv) = create_packet_recv_chan();

        let peer_map = Arc::new(PeerMap::new(
            packet_sender,
            foreign_global_ctx.clone(),
            my_peer_id,
        ));

        let (peer_rpc, rpc_transport_sender) = Self::build_rpc_tspt(my_peer_id, peer_map.clone());

        peer_rpc.rpc_server().registry().register(
            DirectConnectorRpcServer::new(DirectConnectorManagerRpcServer::new(
                foreign_global_ctx.clone(),
            )),
            &network.network_name,
        );

        let relay_bps_limit = global_ctx.config.get_flags().foreign_relay_bps_limit;
        let limiter_config = LimiterConfig {
            burst_rate: None,
            bps: Some(relay_bps_limit),
            fill_duration_ms: None,
        };
        let bps_limiter = global_ctx
            .token_bucket_manager()
            .get_or_create(&network.network_name, limiter_config.into());

        let peer_center = Arc::new(PeerCenterInstance::new(Arc::new(
            PeerMapWithPeerRpcManager {
                peer_map: peer_map.clone(),
                rpc_mgr: peer_rpc.clone(),
            },
        )));

        Self {
            my_peer_id,

            global_ctx: foreign_global_ctx,
            network,
            peer_map,
            relay_data,
            pm_packet_sender: Mutex::new(Some(pm_packet_sender)),

            peer_rpc,
            rpc_sender: rpc_transport_sender,

            packet_recv: Mutex::new(Some(packet_recv)),

            bps_limiter,

            stats_mgr,

            tasks: Mutex::new(JoinSet::new()),

            peer_center,

            lock: Mutex::new(()),
        }
    }

    fn build_foreign_global_ctx(
        network: &NetworkIdentity,
        global_ctx: ArcGlobalCtx,
    ) -> ArcGlobalCtx {
        let config = TomlConfigLoader::default();
        config.set_network_identity(network.clone());
        config.set_hostname(Some(format!(
            "{}{}",
            PUBLIC_SERVER_HOSTNAME_PREFIX,
            global_ctx.get_hostname()
        )));

        let mut flags = config.get_flags();
        flags.disable_relay_kcp = !global_ctx.get_flags().enable_relay_foreign_network_kcp;
        config.set_flags(flags);

        config.set_mapped_listeners(Some(global_ctx.config.get_mapped_listeners()));

        let foreign_global_ctx = Arc::new(GlobalCtx::new(config));
        foreign_global_ctx
            .replace_stun_info_collector(Box::new(global_ctx.get_stun_info_collector().clone()));

        let mut feature_flag = global_ctx.get_feature_flags();
        feature_flag.is_public_server = true;
        foreign_global_ctx.set_feature_flags(feature_flag);

        for u in global_ctx.get_running_listeners().into_iter() {
            foreign_global_ctx.add_running_listener(u);
        }

        foreign_global_ctx
    }

    fn build_rpc_tspt(
        my_peer_id: PeerId,
        peer_map: Arc<PeerMap>,
    ) -> (Arc<PeerRpcManager>, UnboundedSender<ZCPacket>) {
        struct RpcTransport {
            my_peer_id: PeerId,
            peer_map: Weak<PeerMap>,

            packet_recv: Mutex<UnboundedReceiver<ZCPacket>>,
        }

        #[async_trait::async_trait]
        impl PeerRpcManagerTransport for RpcTransport {
            fn my_peer_id(&self) -> PeerId {
                self.my_peer_id
            }

            async fn send(&self, msg: ZCPacket, dst_peer_id: PeerId) -> Result<(), Error> {
                tracing::debug!(
                    "foreign network manager send rpc to peer: {:?}",
                    dst_peer_id
                );
                let peer_map = self
                    .peer_map
                    .upgrade()
                    .ok_or(anyhow::anyhow!("peer map is gone"))?;

                // send to ourselves so we can handle it in forward logic.
                peer_map.send_msg_directly(msg, self.my_peer_id).await
            }

            async fn recv(&self) -> Result<ZCPacket, Error> {
                if let Some(o) = self.packet_recv.lock().await.recv().await {
                    tracing::info!("recv rpc packet in foreign network manager rpc transport");
                    Ok(o)
                } else {
                    Err(Error::Unknown)
                }
            }
        }

        impl Drop for RpcTransport {
            fn drop(&mut self) {
                tracing::debug!(
                    "drop rpc transport for foreign network manager, my_peer_id: {:?}",
                    self.my_peer_id
                );
            }
        }

        let (rpc_transport_sender, peer_rpc_tspt_recv) = mpsc::unbounded_channel();
        let tspt = RpcTransport {
            my_peer_id,
            peer_map: Arc::downgrade(&peer_map),
            packet_recv: Mutex::new(peer_rpc_tspt_recv),
        };

        let peer_rpc = Arc::new(PeerRpcManager::new(tspt));
        (peer_rpc, rpc_transport_sender)
    }

    async fn prepare_route(&self, accessor: Box<dyn GlobalForeignNetworkAccessor>) {
        struct Interface {
            my_peer_id: PeerId,
            peer_map: Weak<PeerMap>,
            network_identity: NetworkIdentity,
            accessor: Box<dyn GlobalForeignNetworkAccessor>,
        }

        #[async_trait::async_trait]
        impl RouteInterface for Interface {
            async fn list_peers(&self) -> Vec<PeerId> {
                let Some(peer_map) = self.peer_map.upgrade() else {
                    return vec![];
                };

                let mut global = self
                    .accessor
                    .list_global_foreign_peer(&self.network_identity)
                    .await;
                let local = peer_map.list_peers_with_conn().await;
                global.extend(local.iter().cloned());
                global
                    .into_iter()
                    .filter(|x| *x != self.my_peer_id)
                    .collect()
            }

            fn my_peer_id(&self) -> PeerId {
                self.my_peer_id
            }
        }

        let route = PeerRoute::new(
            self.my_peer_id,
            self.global_ctx.clone(),
            self.peer_rpc.clone(),
        );
        route
            .open(Box::new(Interface {
                my_peer_id: self.my_peer_id,
                network_identity: self.network.clone(),
                peer_map: Arc::downgrade(&self.peer_map),
                accessor,
            }))
            .await
            .unwrap();

        route
            .set_route_cost_fn(self.peer_center.get_cost_calculator())
            .await;

        self.peer_map.add_route(Arc::new(Box::new(route))).await;
    }

    async fn start_packet_recv(&self) {
        let mut recv = self.packet_recv.lock().await.take().unwrap();
        let my_node_id = self.my_peer_id;
        let rpc_sender = self.rpc_sender.clone();
        let peer_map = self.peer_map.clone();
        let relay_data = self.relay_data;
        let pm_sender = self.pm_packet_sender.lock().await.take().unwrap();
        let network_name = self.network.network_name.clone();
        let bps_limiter = self.bps_limiter.clone();

        let label_set =
            LabelSet::new().with_label_type(LabelType::NetworkName(network_name.clone()));
        let forward_bytes = self
            .stats_mgr
            .get_counter(MetricName::TrafficBytesForwarded, label_set.clone());
        let forward_packets = self
            .stats_mgr
            .get_counter(MetricName::TrafficPacketsForwarded, label_set.clone());
        let rx_bytes = self
            .stats_mgr
            .get_counter(MetricName::TrafficBytesSelfRx, label_set.clone());
        let rx_packets = self
            .stats_mgr
            .get_counter(MetricName::TrafficPacketsRx, label_set.clone());

        self.tasks.lock().await.spawn(async move {
            while let Ok(zc_packet) = recv_packet_from_chan(&mut recv).await {
                let buf_len = zc_packet.buf_len();
                let Some(hdr) = zc_packet.peer_manager_header() else {
                    tracing::warn!("invalid packet, skip");
                    continue;
                };
                tracing::info!(?hdr, "recv packet in foreign network manager");
                let to_peer_id = hdr.to_peer_id.get();
                if to_peer_id == my_node_id {
                    if hdr.packet_type == PacketType::TaRpc as u8
                        || hdr.packet_type == PacketType::RpcReq as u8
                        || hdr.packet_type == PacketType::RpcResp as u8
                    {
                        rx_bytes.add(buf_len as u64);
                        rx_packets.inc();
                        rpc_sender.send(zc_packet).unwrap();
                        continue;
                    }
                    tracing::trace!(?hdr, "ignore packet in foreign network");
                } else {
                    if hdr.packet_type == PacketType::Data as u8
                        || hdr.packet_type == PacketType::KcpSrc as u8
                        || hdr.packet_type == PacketType::KcpDst as u8
                    {
                        if !relay_data {
                            continue;
                        }
                        if !bps_limiter.try_consume(hdr.len.into()) {
                            continue;
                        }
                    }

                    forward_bytes.add(buf_len as u64);
                    forward_packets.inc();

                    let gateway_peer_id = peer_map
                        .get_gateway_peer_id(to_peer_id, NextHopPolicy::LeastHop)
                        .await;

                    match gateway_peer_id {
                        Some(peer_id) if peer_map.has_peer(peer_id) => {
                            if let Err(e) = peer_map.send_msg_directly(zc_packet, peer_id).await {
                                tracing::error!(
                                    ?e,
                                    "send packet to foreign peer inside peer map failed"
                                );
                            }
                        }
                        _ => {
                            let mut foreign_packet = ZCPacket::new_for_foreign_network(
                                &network_name,
                                to_peer_id,
                                &zc_packet,
                            );
                            let via_peer = gateway_peer_id.unwrap_or(to_peer_id);
                            foreign_packet.fill_peer_manager_hdr(
                                my_node_id,
                                via_peer,
                                PacketType::ForeignNetworkPacket as u8,
                            );
                            if let Err(e) = pm_sender.send(foreign_packet).await {
                                tracing::error!("send packet to peer with pm failed: {:?}", e);
                            }
                        }
                    };
                }
            }
        });
    }

    async fn prepare(&self, accessor: Box<dyn GlobalForeignNetworkAccessor>) {
        self.prepare_route(accessor).await;
        self.start_packet_recv().await;
        self.peer_rpc.run();
        self.peer_center.init().await;
    }
}

impl Drop for ForeignNetworkEntry {
    fn drop(&mut self) {
        self.peer_rpc
            .rpc_server()
            .registry()
            .unregister_by_domain(&self.network.network_name);

        tracing::debug!(self.my_peer_id, ?self.network, "drop foreign network entry");
    }
}

struct ForeignNetworkManagerData {
    network_peer_maps: DashMap<String, Arc<ForeignNetworkEntry>>,
    peer_network_map: DashMap<PeerId, DashSet<String>>,
    network_peer_last_update: DashMap<String, SystemTime>,
    accessor: Arc<Box<dyn GlobalForeignNetworkAccessor>>,
    lock: std::sync::Mutex<()>,
}

impl ForeignNetworkManagerData {
    fn get_peer_network(&self, peer_id: PeerId) -> Option<DashSet<String>> {
        self.peer_network_map.get(&peer_id).map(|v| v.clone())
    }

    fn get_network_entry(&self, network_name: &str) -> Option<Arc<ForeignNetworkEntry>> {
        self.network_peer_maps.get(network_name).map(|v| v.clone())
    }

    fn remove_peer(&self, peer_id: PeerId, network_name: &String) {
        let _l = self.lock.lock().unwrap();
        self.peer_network_map.remove_if(&peer_id, |_, v| {
            let _ = v.remove(network_name);
            v.is_empty()
        });
        if self
            .network_peer_maps
            .remove_if(network_name, |_, v| v.peer_map.is_empty())
            .is_some()
        {
            self.network_peer_last_update.remove(network_name);
        }
    }

    async fn clear_no_conn_peer(&self, network_name: &String) {
        let Some(peer_map) = self
            .network_peer_maps
            .get(network_name)
            .map(|v| v.peer_map.clone())
        else {
            return;
        };
        peer_map.clean_peer_without_conn().await;
    }

    fn remove_network(&self, network_name: &String) {
        let _l = self.lock.lock().unwrap();
        self.peer_network_map.iter().for_each(|v| {
            v.value().remove(network_name);
        });
        self.peer_network_map.retain(|_, v| !v.is_empty());
        self.network_peer_maps.remove(network_name);
        self.network_peer_last_update.remove(network_name);
    }

    async fn get_or_insert_entry(
        &self,
        network_identity: &NetworkIdentity,
        my_peer_id: PeerId,
        dst_peer_id: PeerId,
        relay_data: bool,
        global_ctx: &ArcGlobalCtx,
        pm_packet_sender: &PacketRecvChan,
    ) -> (Arc<ForeignNetworkEntry>, bool) {
        let mut new_added = false;

        let l = self.lock.lock().unwrap();
        let entry = self
            .network_peer_maps
            .entry(network_identity.network_name.clone())
            .or_insert_with(|| {
                new_added = true;
                Arc::new(ForeignNetworkEntry::new(
                    network_identity.clone(),
                    my_peer_id,
                    global_ctx.clone(),
                    relay_data,
                    pm_packet_sender.clone(),
                ))
            })
            .clone();

        self.peer_network_map
            .entry(dst_peer_id)
            .or_default()
            .insert(network_identity.network_name.clone());

        self.network_peer_last_update
            .insert(network_identity.network_name.clone(), SystemTime::now());

        drop(l);

        if new_added {
            entry.prepare(Box::new(self.accessor.clone())).await;
        }

        (entry, new_added)
    }
}

pub const FOREIGN_NETWORK_SERVICE_ID: u32 = 1;

pub struct ForeignNetworkManager {
    my_peer_id: PeerId,
    global_ctx: ArcGlobalCtx,
    packet_sender_to_mgr: PacketRecvChan,

    data: Arc<ForeignNetworkManagerData>,

    tasks: Arc<std::sync::Mutex<JoinSet<()>>>,
}

impl ForeignNetworkManager {
    pub fn new(
        my_peer_id: PeerId,
        global_ctx: ArcGlobalCtx,
        packet_sender_to_mgr: PacketRecvChan,
        accessor: Box<dyn GlobalForeignNetworkAccessor>,
    ) -> Self {
        let data = Arc::new(ForeignNetworkManagerData {
            network_peer_maps: DashMap::new(),
            peer_network_map: DashMap::new(),
            network_peer_last_update: DashMap::new(),
            accessor: Arc::new(accessor),
            lock: std::sync::Mutex::new(()),
        });

        let tasks = Arc::new(std::sync::Mutex::new(JoinSet::new()));
        join_joinset_background(tasks.clone(), "ForeignNetworkManager".to_string());

        Self {
            my_peer_id,
            global_ctx,
            packet_sender_to_mgr,

            data,

            tasks,
        }
    }

    pub fn get_network_peer_id(&self, network_name: &str) -> Option<PeerId> {
        self.data
            .network_peer_maps
            .get(network_name)
            .map(|v| v.my_peer_id)
    }

    pub async fn add_peer_conn(&self, peer_conn: PeerConn) -> Result<(), Error> {
        tracing::info!(peer_conn = ?peer_conn.get_conn_info(), network = ?peer_conn.get_network_identity(), "add new peer conn in foreign network manager");

        let relay_peer_rpc = self.global_ctx.get_flags().relay_all_peer_rpc;
        let ret = self
            .global_ctx
            .check_network_in_whitelist(&peer_conn.get_network_identity().network_name)
            .map_err(Into::into);
        if ret.is_err() && !relay_peer_rpc {
            return ret;
        }

        let (entry, new_added) = self
            .data
            .get_or_insert_entry(
                &peer_conn.get_network_identity(),
                peer_conn.get_my_peer_id(),
                peer_conn.get_peer_id(),
                ret.is_ok(),
                &self.global_ctx,
                &self.packet_sender_to_mgr,
            )
            .await;

        let _g = entry.lock.lock().await;

        if entry.network != peer_conn.get_network_identity()
            || entry.my_peer_id != peer_conn.get_my_peer_id()
        {
            if new_added {
                self.data
                    .remove_network(&entry.network.network_name.clone());
            }
            let err = if entry.my_peer_id != peer_conn.get_my_peer_id() {
                anyhow::anyhow!(
                    "my peer id not match. exp: {:?} real: {:?}, need retry connect",
                    entry.my_peer_id,
                    peer_conn.get_my_peer_id()
                )
            } else {
                anyhow::anyhow!(
                    "network secret not match. exp: {:?} real: {:?}",
                    entry.network,
                    peer_conn.get_network_identity()
                )
            };
            tracing::error!(?err, "foreign network entry not match, disconnect peer");
            return Err(err.into());
        }

        if new_added {
            self.start_event_handler(&entry).await;
        } else if let Some(peer) = entry.peer_map.get_peer_by_id(peer_conn.get_peer_id()) {
            let direct_conns_len = peer.get_directly_connections().len();
            let max_count = use_global_var!(MAX_DIRECT_CONNS_PER_PEER_IN_FOREIGN_NETWORK);
            if direct_conns_len >= max_count as usize {
                return Err(anyhow::anyhow!(
                    "too many direct conns, cur: {}, max: {}",
                    direct_conns_len,
                    max_count
                )
                .into());
            }
        }

        entry.peer_map.add_new_peer_conn(peer_conn).await;
        Ok(())
    }

    async fn start_event_handler(&self, entry: &ForeignNetworkEntry) {
        let data = self.data.clone();
        let network_name = entry.network.network_name.clone();
        let mut s = entry.global_ctx.subscribe();
        self.tasks.lock().unwrap().spawn(async move {
            while let Ok(e) = s.recv().await {
                match &e {
                    GlobalCtxEvent::PeerRemoved(peer_id) => {
                        tracing::info!(?e, "remove peer from foreign network manager");
                        data.remove_peer(*peer_id, &network_name);
                        data.network_peer_last_update
                            .insert(network_name.clone(), SystemTime::now());
                    }
                    GlobalCtxEvent::PeerConnRemoved(..) => {
                        tracing::info!(?e, "clear no conn peer from foreign network manager");
                        data.clear_no_conn_peer(&network_name).await;
                    }
                    GlobalCtxEvent::PeerAdded(_) => {
                        tracing::info!(?e, "add peer to foreign network manager");
                        data.network_peer_last_update
                            .insert(network_name.clone(), SystemTime::now());
                    }
                    _ => continue,
                }
            }
            // if lagged or recv done just remove the network
            tracing::error!("global event handler at foreign network manager exit");
            data.remove_network(&network_name);
        });
    }

    pub async fn list_foreign_networks(&self) -> ListForeignNetworkResponse {
        let mut ret = ListForeignNetworkResponse::default();
        let networks = self
            .data
            .network_peer_maps
            .iter()
            .map(|v| v.key().clone())
            .collect::<Vec<_>>();

        for network_name in networks {
            let Some(item) = self
                .data
                .network_peer_maps
                .get(&network_name)
                .map(|v| v.clone())
            else {
                continue;
            };

            let mut entry = ForeignNetworkEntryPb {
                network_secret_digest: item
                    .network
                    .network_secret_digest
                    .unwrap_or_default()
                    .to_vec(),
                my_peer_id_for_this_network: item.my_peer_id,
                peers: Default::default(),
            };
            for peer in item.peer_map.list_peers().await {
                let peer_info = PeerInfo {
                    peer_id: peer,
                    conns: item.peer_map.list_peer_conns(peer).await.unwrap_or(vec![]),
                    ..Default::default()
                };
                entry.peers.push(peer_info);
            }

            ret.foreign_networks.insert(network_name, entry);
        }
        ret
    }

    pub fn get_foreign_network_last_update(&self, network_name: &str) -> Option<SystemTime> {
        self.data
            .network_peer_last_update
            .get(network_name)
            .map(|v| *v)
    }

    pub async fn send_msg_to_peer(
        &self,
        network_name: &str,
        dst_peer_id: PeerId,
        msg: ZCPacket,
    ) -> Result<(), Error> {
        if let Some(entry) = self.data.get_network_entry(network_name) {
            entry
                .peer_map
                .send_msg(msg, dst_peer_id, NextHopPolicy::LeastHop)
                .await
        } else {
            Err(Error::RouteError(Some("network not found".to_string())))
        }
    }

    pub async fn close_peer_conn(
        &self,
        peer_id: PeerId,
        conn_id: &super::peer_conn::PeerConnId,
    ) -> Result<(), Error> {
        let network_names = self.data.get_peer_network(peer_id).unwrap_or_default();
        for network_name in network_names {
            if let Some(entry) = self.data.get_network_entry(&network_name) {
                let ret = entry.peer_map.close_peer_conn(peer_id, conn_id).await;
                if ret.is_ok() || !matches!(ret.as_ref().unwrap_err(), Error::NotFound) {
                    return ret;
                }
            }
        }
        Err(Error::NotFound)
    }
}

impl Drop for ForeignNetworkManager {
    fn drop(&mut self) {
        self.data.peer_network_map.clear();
        self.data.network_peer_maps.clear();
    }
}
