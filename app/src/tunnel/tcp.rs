use std::net::SocketAddr;

use async_trait::async_trait;
use futures::stream::FuturesUnordered;
use tokio::net::{TcpListener, TcpSocket, TcpStream};

use super::TunnelInfo;
use crate::tunnel::common::setup_sokcet2;

use super::{
    check_scheme_and_get_socket_addr,
    common::{wait_for_connect_futures, FramedReader, FramedWriter, TunnelWrapper},
    IpVersion, Tunnel, TunnelError, TunnelListener,
};

const TCP_MTU_BYTES: usize = 2000;

#[derive(Debug)]
pub struct TcpTunnelListener {
    addr: url::Url,
    listener: Option<TcpListener>,
}

impl TcpTunnelListener {
    pub fn new(addr: url::Url) -> Self {
        TcpTunnelListener {
            addr,
            listener: None,
        }
    }

    async fn do_accept(&mut self) -> Result<Box<dyn Tunnel>, std::io::Error> {
        let listener = self.listener.as_ref().unwrap();
        let (stream, _) = listener.accept().await?;

        if let Err(e) = stream.set_nodelay(true) {
            tracing::warn!(?e, "set_nodelay fail in accept");
        }

        let info = TunnelInfo {
            tunnel_type: "tcp".to_owned(),
            local_addr: Some(self.local_url().into()),
            remote_addr: Some(
                super::build_url_from_socket_addr(&stream.peer_addr()?.to_string(), "tcp").into(),
            ),
        };

        let (r, w) = stream.into_split();
        Ok(Box::new(TunnelWrapper::new(
            FramedReader::new(r, TCP_MTU_BYTES),
            FramedWriter::new(w),
            Some(info),
        )))
    }
}

#[async_trait]
impl TunnelListener for TcpTunnelListener {
    async fn listen(&mut self) -> Result<(), TunnelError> {
        self.listener = None;
        let addr =
            check_scheme_and_get_socket_addr::<SocketAddr>(&self.addr, "tcp", IpVersion::Both)
                .await?;

        let socket2_socket = socket2::Socket::new(
            socket2::Domain::for_address(addr),
            socket2::Type::STREAM,
            Some(socket2::Protocol::TCP),
        )?;
        setup_sokcet2(&socket2_socket, &addr)?;
        let socket = TcpSocket::from_std_stream(socket2_socket.into());

        if let Err(e) = socket.set_nodelay(true) {
            tracing::warn!(?e, "set_nodelay fail in listen");
        }

        self.addr
            .set_port(Some(socket.local_addr()?.port()))
            .unwrap();

        self.listener = Some(socket.listen(1024)?);
        Ok(())
    }

    async fn accept(&mut self) -> Result<Box<dyn Tunnel>, super::TunnelError> {
        loop {
            match self.do_accept().await {
                Ok(ret) => return Ok(ret),
                Err(e) => {
                    use std::io::ErrorKind::*;
                    if matches!(
                        e.kind(),
                        NotConnected | ConnectionAborted | ConnectionRefused | ConnectionReset
                    ) {
                        tracing::warn!(?e, "accept fail with retryable error: {:?}", e);
                        continue;
                    }
                    tracing::warn!(?e, "accept fail");
                    return Err(e.into());
                }
            }
        }
    }

    fn local_url(&self) -> url::Url {
        self.addr.clone()
    }
}

fn get_tunnel_with_tcp_stream(
    stream: TcpStream,
    remote_url: url::Url,
) -> Result<Box<dyn Tunnel>, super::TunnelError> {
    if let Err(e) = stream.set_nodelay(true) {
        tracing::warn!(?e, "set_nodelay fail in get_tunnel_with_tcp_stream");
    }

    let info = TunnelInfo {
        tunnel_type: "tcp".to_owned(),
        local_addr: Some(
            super::build_url_from_socket_addr(&stream.local_addr()?.to_string(), "tcp").into(),
        ),
        remote_addr: Some(remote_url.into()),
    };

    let (r, w) = stream.into_split();
    Ok(Box::new(TunnelWrapper::new(
        FramedReader::new(r, TCP_MTU_BYTES),
        FramedWriter::new(w),
        Some(info),
    )))
}

#[derive(Debug)]
pub struct TcpTunnelConnector {
    addr: url::Url,

    bind_addrs: Vec<SocketAddr>,
    ip_version: IpVersion,
}

impl TcpTunnelConnector {
    pub fn new(addr: url::Url) -> Self {
        TcpTunnelConnector {
            addr,
            bind_addrs: vec![],
            ip_version: IpVersion::Both,
        }
    }

    async fn connect_with_default_bind(
        &mut self,
        addr: SocketAddr,
    ) -> Result<Box<dyn Tunnel>, super::TunnelError> {
        tracing::info!(url = ?self.addr, ?addr, "connect tcp start, bind addrs: {:?}", self.bind_addrs);
        let stream = TcpStream::connect(addr).await?;
        tracing::info!(url = ?self.addr, ?addr, "connect tcp succ");
        get_tunnel_with_tcp_stream(stream, self.addr.clone())
    }

    async fn connect_with_custom_bind(
        &mut self,
        addr: SocketAddr,
    ) -> Result<Box<dyn Tunnel>, super::TunnelError> {
        let futures = FuturesUnordered::new();

        for bind_addr in self.bind_addrs.iter() {
            tracing::info!(bind_addr = ?bind_addr, ?addr, "bind addr");

            let socket2_socket = socket2::Socket::new(
                socket2::Domain::for_address(addr),
                socket2::Type::STREAM,
                Some(socket2::Protocol::TCP),
            )?;

            if let Err(e) = setup_sokcet2(&socket2_socket, bind_addr) {
                tracing::error!(bind_addr = ?bind_addr, ?addr, "bind addr fail: {:?}", e);
                continue;
            }

            let socket = TcpSocket::from_std_stream(socket2_socket.into());
            futures.push(socket.connect(addr));
        }

        let ret = wait_for_connect_futures(futures).await;
        get_tunnel_with_tcp_stream(ret?, self.addr.clone())
    }
}

#[async_trait]
impl super::TunnelConnector for TcpTunnelConnector {
    async fn connect(&mut self) -> Result<Box<dyn Tunnel>, super::TunnelError> {
        let addr =
            check_scheme_and_get_socket_addr::<SocketAddr>(&self.addr, "tcp", self.ip_version)
                .await?;
        if self.bind_addrs.is_empty() {
            self.connect_with_default_bind(addr).await
        } else {
            self.connect_with_custom_bind(addr).await
        }
    }

    fn remote_url(&self) -> url::Url {
        self.addr.clone()
    }

    fn set_bind_addrs(&mut self, addrs: Vec<SocketAddr>) {
        self.bind_addrs = addrs;
    }

    fn set_ip_version(&mut self, ip_version: IpVersion) {
        self.ip_version = ip_version;
    }
}
