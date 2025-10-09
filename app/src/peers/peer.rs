use std::sync::Arc;

use crossbeam::atomic::AtomicCell;
use dashmap::{DashMap, DashSet};

use tokio::{select, sync::mpsc};

use tracing::Instrument;

use super::{
    peer_conn::{PeerConn, PeerConnId},
    PacketRecvChan,
};
use crate::{common::scoped_task::ScopedTask, proto::cli::PeerConnInfo};
use crate::{
    common::{
        error::Error,
        global_ctx::{ArcGlobalCtx, GlobalCtxEvent},
        PeerId,
    },
    tunnel::packet_def::ZCPacket,
};

type ArcPeerConn = Arc<PeerConn>;
type ConnMap = Arc<DashMap<PeerConnId, ArcPeerConn>>;

pub struct Peer {
    pub peer_node_id: PeerId,
    conns: ConnMap,
    global_ctx: ArcGlobalCtx,

    packet_recv_chan: PacketRecvChan,

    close_event_sender: mpsc::Sender<PeerConnId>,
    close_event_listener: ScopedTask<()>,

    shutdown_notifier: Arc<tokio::sync::Notify>,

    default_conn_id: Arc<AtomicCell<PeerConnId>>,
    default_conn_id_clear_task: ScopedTask<()>,
}

impl Peer {
    pub fn new(
        peer_node_id: PeerId,
        packet_recv_chan: PacketRecvChan,
        global_ctx: ArcGlobalCtx,
    ) -> Self {
        let conns: ConnMap = Arc::new(DashMap::new());
        let (close_event_sender, mut close_event_receiver) = mpsc::channel(10);
        let shutdown_notifier = Arc::new(tokio::sync::Notify::new());

        let conns_copy = conns.clone();
        let shutdown_notifier_copy = shutdown_notifier.clone();
        let global_ctx_copy = global_ctx.clone();
        let close_event_listener = tokio::spawn(
            async move {
                loop {
                    select! {
                        ret = close_event_receiver.recv() => {
                            if ret.is_none() {
                                break;
                            }
                            let ret = ret.unwrap();
                            tracing::warn!(
                                ?peer_node_id,
                                ?ret,
                                "notified that peer conn is closed",
                            );

                            if let Some((_, conn)) = conns_copy.remove(&ret) {
                                global_ctx_copy.issue_event(GlobalCtxEvent::PeerConnRemoved(
                                    conn.get_conn_info(),
                                ));
                            }
                        }

                        _ = shutdown_notifier_copy.notified() => {
                            close_event_receiver.close();
                            tracing::warn!(?peer_node_id, "peer close event listener notified");
                        }
                    }
                }
                tracing::info!("peer {} close event listener exit", peer_node_id);
            }
            .instrument(tracing::info_span!(
                "peer_close_event_listener",
                ?peer_node_id,
            )),
        )
        .into();

        let default_conn_id = Arc::new(AtomicCell::new(PeerConnId::default()));

        let conns_copy = conns.clone();
        let default_conn_id_copy = default_conn_id.clone();
        let default_conn_id_clear_task = ScopedTask::from(tokio::spawn(async move {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                if conns_copy.len() > 1 {
                    default_conn_id_copy.store(PeerConnId::default());
                }
            }
        }));

        Peer {
            peer_node_id,
            conns: conns.clone(),
            packet_recv_chan,
            global_ctx,

            close_event_sender,
            close_event_listener,

            shutdown_notifier,
            default_conn_id,
            default_conn_id_clear_task,
        }
    }

    pub async fn add_peer_conn(&self, mut conn: PeerConn) {
        let close_notifier = conn.get_close_notifier();
        let conn_info = conn.get_conn_info();

        conn.start_recv_loop(self.packet_recv_chan.clone()).await;
        conn.start_pingpong();
        self.conns.insert(conn.get_conn_id(), Arc::new(conn));

        let close_event_sender = self.close_event_sender.clone();
        tokio::spawn(async move {
            let conn_id = close_notifier.get_conn_id();
            if let Some(mut waiter) = close_notifier.get_waiter().await {
                let _ = waiter.recv().await;
            }
            if let Err(e) = close_event_sender.send(conn_id).await {
                tracing::warn!(?conn_id, "failed to send close event: {}", e);
            }
        });

        self.global_ctx
            .issue_event(GlobalCtxEvent::PeerConnAdded(conn_info));
    }

    async fn select_conn(&self) -> Option<ArcPeerConn> {
        let default_conn_id = self.default_conn_id.load();
        if let Some(conn) = self.conns.get(&default_conn_id) {
            return Some(conn.clone());
        }

        // find a conn with the smallest latency
        let mut min_latency = u64::MAX;
        for conn in self.conns.iter() {
            let latency = conn.value().get_stats().latency_us;
            if latency < min_latency {
                min_latency = latency;
                self.default_conn_id.store(conn.get_conn_id());
            }
        }

        self.conns
            .get(&self.default_conn_id.load())
            .map(|conn| conn.clone())
    }

    pub async fn send_msg(&self, msg: ZCPacket) -> Result<(), Error> {
        let Some(conn) = self.select_conn().await else {
            return Err(Error::PeerNoConnectionError(self.peer_node_id));
        };
        conn.send_msg(msg).await?;

        Ok(())
    }

    pub async fn close_peer_conn(&self, conn_id: &PeerConnId) -> Result<(), Error> {
        let has_key = self.conns.contains_key(conn_id);
        if !has_key {
            return Err(Error::NotFound);
        }
        self.close_event_sender.send(*conn_id).await.unwrap();
        Ok(())
    }

    pub async fn list_peer_conns(&self) -> Vec<PeerConnInfo> {
        let mut conns = vec![];
        for conn in self.conns.iter() {
            // do not lock here, otherwise it will cause dashmap deadlock
            conns.push(conn.clone());
        }

        let mut ret = Vec::new();
        for conn in conns {
            let info = conn.get_conn_info();
            if !info.is_closed {
                ret.push(info);
            } else {
                let conn_id = info.conn_id.parse().unwrap();
                let _ = self.close_peer_conn(&conn_id).await;
            }
        }
        ret
    }

    pub fn has_directly_connected_conn(&self) -> bool {
        self.conns
            .iter()
            .any(|entry| !(entry.value()).is_hole_punched())
    }

    pub fn get_directly_connections(&self) -> DashSet<uuid::Uuid> {
        self.conns
            .iter()
            .filter(|entry| !(entry.value()).is_hole_punched())
            .map(|entry| (entry.value()).get_conn_id())
            .collect()
    }

    pub fn get_default_conn_id(&self) -> PeerConnId {
        self.default_conn_id.load()
    }
}

// pritn on drop
impl Drop for Peer {
    fn drop(&mut self) {
        self.shutdown_notifier.notify_one();
        tracing::info!("peer {} drop", self.peer_node_id);
    }
}
