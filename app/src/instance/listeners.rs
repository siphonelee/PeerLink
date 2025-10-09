use std::{
    fmt::Debug,
    net::IpAddr,
    str::FromStr,
    sync::{Arc, Weak},
};

use anyhow::Context;
use async_trait::async_trait;
use tokio::task::JoinSet;

#[cfg(feature = "quic")]
use crate::tunnel::quic::QUICTunnelListener;
#[cfg(feature = "wireguard")]
use crate::tunnel::wireguard::{WgConfig, WgTunnelListener};
use crate::{
    common::{
        error::Error,
        global_ctx::{ArcGlobalCtx, GlobalCtxEvent},
        netns::NetNS,
    },
    peers::peer_manager::PeerManager,
    tunnel::{
        ring::RingTunnelListener, tcp::TcpTunnelListener, udp::UdpTunnelListener, Tunnel,
        TunnelListener,
    },
};

pub fn get_listener_by_url(
    l: &url::Url,
    _ctx: ArcGlobalCtx,
) -> Result<Box<dyn TunnelListener>, Error> {
    Ok(match l.scheme() {
        "tcp" => Box::new(TcpTunnelListener::new(l.clone())),
        "udp" => Box::new(UdpTunnelListener::new(l.clone())),
        #[cfg(feature = "wireguard")]
        "wg" => {
            let nid = _ctx.get_network_identity();
            let wg_config = WgConfig::new_from_network_identity(
                &nid.network_name,
                &nid.network_secret.unwrap_or_default(),
            );
            Box::new(WgTunnelListener::new(l.clone(), wg_config))
        }
        #[cfg(feature = "quic")]
        "quic" => Box::new(QUICTunnelListener::new(l.clone())),
        #[cfg(feature = "websocket")]
        "ws" | "wss" => {
            use crate::tunnel::websocket::WSTunnelListener;
            Box::new(WSTunnelListener::new(l.clone()))
        }
        _ => {
            return Err(Error::InvalidUrl(l.to_string()));
        }
    })
}

pub fn is_url_host_ipv6(l: &url::Url) -> bool {
    l.host_str().is_some_and(|h| h.contains(':'))
}

pub fn is_url_host_unspecified(l: &url::Url) -> bool {
    if let Ok(ip) = IpAddr::from_str(l.host_str().unwrap_or_default()) {
        ip.is_unspecified()
    } else {
        false
    }
}

#[async_trait]
pub trait TunnelHandlerForListener {
    async fn handle_tunnel(&self, tunnel: Box<dyn Tunnel>) -> Result<(), Error>;
}

#[async_trait]
impl TunnelHandlerForListener for PeerManager {
    #[tracing::instrument]
    async fn handle_tunnel(&self, tunnel: Box<dyn Tunnel>) -> Result<(), Error> {
        self.add_tunnel_as_server(tunnel, true).await
    }
}

pub trait ListenerCreatorTrait: Fn() -> Box<dyn TunnelListener> + Send + Sync {}
impl<T: Send + Sync> ListenerCreatorTrait for T where T: Fn() -> Box<dyn TunnelListener> + Send {}
pub type ListenerCreator = Box<dyn ListenerCreatorTrait>;

#[derive(Clone)]
struct ListenerFactory {
    creator_fn: Arc<ListenerCreator>,
    must_succ: bool,
}

pub struct ListenerManager<H> {
    global_ctx: ArcGlobalCtx,
    net_ns: NetNS,
    listeners: Vec<ListenerFactory>,
    peer_manager: Weak<H>,

    tasks: JoinSet<()>,
}

impl<H: TunnelHandlerForListener + Send + Sync + 'static + Debug> ListenerManager<H> {
    pub fn new(global_ctx: ArcGlobalCtx, peer_manager: Arc<H>) -> Self {
        Self {
            global_ctx: global_ctx.clone(),
            net_ns: global_ctx.net_ns.clone(),
            listeners: Vec::new(),
            peer_manager: Arc::downgrade(&peer_manager),
            tasks: JoinSet::new(),
        }
    }

    pub async fn prepare_listeners(&mut self) -> Result<(), Error> {
        let self_id = self.global_ctx.get_id();
        self.add_listener(
            move || {
                Box::new(RingTunnelListener::new(
                    format!("ring://{}", self_id).parse().unwrap(),
                ))
            },
            true,
        )
        .await?;

        for l in self.global_ctx.config.get_listener_uris().iter() {
            let l = l.clone();
            let Ok(_) = get_listener_by_url(&l, self.global_ctx.clone()) else {
                let msg = format!("failed to get listener by url: {}, maybe not supported", l);
                self.global_ctx
                    .issue_event(GlobalCtxEvent::ListenerAddFailed(l.clone(), msg));
                continue;
            };
            let ctx = self.global_ctx.clone();

            let listener = l.clone();
            self.add_listener(
                move || get_listener_by_url(&listener, ctx.clone()).unwrap(),
                true,
            )
            .await?;

            if self.global_ctx.config.get_flags().enable_ipv6
                && !is_url_host_ipv6(&l)
                && is_url_host_unspecified(&l)
                // quic enables dual-stack by default, may conflict with v4 listener
                && l.scheme() != "quic"
            {
                let mut ipv6_listener = l.clone();
                ipv6_listener
                    .set_host(Some("[::]".to_string().as_str()))
                    .with_context(|| format!("failed to set ipv6 host for listener: {}", l))?;
                let ctx = self.global_ctx.clone();
                self.add_listener(
                    move || get_listener_by_url(&ipv6_listener, ctx.clone()).unwrap(),
                    false,
                )
                .await?;
            }
        }

        Ok(())
    }

    pub async fn add_listener<C: ListenerCreatorTrait + 'static>(
        &mut self,
        creator: C,
        must_succ: bool,
    ) -> Result<(), Error> {
        self.listeners.push(ListenerFactory {
            creator_fn: Arc::new(Box::new(creator)),
            must_succ,
        });
        Ok(())
    }

    #[tracing::instrument(skip(creator))]
    async fn run_listener(
        creator: Arc<ListenerCreator>,
        peer_manager: Weak<H>,
        global_ctx: ArcGlobalCtx,
    ) {
        let mut err_count = 0;
        loop {
            let mut l = (creator)();
            let _g = global_ctx.net_ns.guard();
            match l.listen().await {
                Ok(_) => {
                    err_count = 0;
                    global_ctx.add_running_listener(l.local_url());
                    global_ctx.issue_event(GlobalCtxEvent::ListenerAdded(l.local_url()));
                }
                Err(e) => {
                    tracing::error!(?e, ?l, "listener listen error");
                    global_ctx.issue_event(GlobalCtxEvent::ListenerAddFailed(
                        l.local_url(),
                        format!("error: {:?}, retry listen later...", e),
                    ));
                    err_count += 1;
                    if err_count > 5 {
                        return;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
            loop {
                let ret = match l.accept().await {
                    Ok(ret) => ret,
                    Err(e) => {
                        global_ctx.issue_event(GlobalCtxEvent::ListenerAcceptFailed(
                            l.local_url(),
                            format!("error: {:?}, retry listen later...", e),
                        ));
                        tracing::error!(?e, ?l, "listener accept error");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                        break;
                    }
                };

                let tunnel_info = ret.info().unwrap();
                global_ctx.issue_event(GlobalCtxEvent::ConnectionAccepted(
                    tunnel_info
                        .local_addr
                        .clone()
                        .unwrap_or_default()
                        .to_string(),
                    tunnel_info
                        .remote_addr
                        .clone()
                        .unwrap_or_default()
                        .to_string(),
                ));
                tracing::info!(ret = ?ret, "conn accepted");
                let peer_manager = peer_manager.clone();
                let global_ctx = global_ctx.clone();
                tokio::spawn(async move {
                    let Some(peer_manager) = peer_manager.upgrade() else {
                        tracing::error!("peer manager is gone, cannot handle tunnel");
                        return;
                    };
                    let server_ret = peer_manager.handle_tunnel(ret).await;
                    if let Err(e) = &server_ret {
                        global_ctx.issue_event(GlobalCtxEvent::ConnectionError(
                            tunnel_info.local_addr.unwrap_or_default().to_string(),
                            tunnel_info.remote_addr.unwrap_or_default().to_string(),
                            e.to_string(),
                        ));
                        tracing::error!(error = ?e, "handle conn error");
                    }
                });
            }
        }
    }

    pub async fn run(&mut self) -> Result<(), Error> {
        for listener in &self.listeners {
            if listener.must_succ {
                // try listen once
                let mut l = (listener.creator_fn)();
                let _g = self.net_ns.guard();
                l.listen()
                    .await
                    .with_context(|| format!("failed to listen on {}", l.local_url()))?;
            }

            self.tasks.spawn(Self::run_listener(
                listener.creator_fn.clone(),
                self.peer_manager.clone(),
                self.global_ctx.clone(),
            ));
        }

        Ok(())
    }
}
