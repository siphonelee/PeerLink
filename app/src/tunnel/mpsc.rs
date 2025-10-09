// this mod wrap tunnel to a mpsc tunnel, based on crossbeam_channel

use std::{pin::Pin, time::Duration};

use anyhow::Context;
use tokio::time::timeout;

use crate::{common::scoped_task::ScopedTask, proto::common::TunnelInfo};

use super::{packet_def::ZCPacket, Tunnel, TunnelError, ZCPacketSink, ZCPacketStream};

// use tokio::sync::mpsc::{channel, error::TrySendError, Receiver, Sender};
use tachyonix::{channel, Receiver, Sender, TrySendError};

use futures::SinkExt;

#[derive(Clone)]
pub struct MpscTunnelSender(Sender<ZCPacket>);

impl MpscTunnelSender {
    pub async fn send(&self, item: ZCPacket) -> Result<(), TunnelError> {
        self.0.send(item).await.with_context(|| "send error")?;
        Ok(())
    }

    pub fn try_send(&self, item: ZCPacket) -> Result<(), TunnelError> {
        self.0.try_send(item).map_err(|e| match e {
            TrySendError::Full(_) => TunnelError::BufferFull,
            TrySendError::Closed(_) => TunnelError::Shutdown,
        })
    }
}

pub struct MpscTunnel<T> {
    tx: Option<Sender<ZCPacket>>,

    tunnel: T,
    stream: Option<Pin<Box<dyn ZCPacketStream>>>,

    task: ScopedTask<()>,
}

impl<T: Tunnel> MpscTunnel<T> {
    pub fn new(tunnel: T, send_timeout: Option<Duration>) -> Self {
        let (tx, mut rx) = channel(32);
        let (stream, mut sink) = tunnel.split();

        let task = tokio::spawn(async move {
            loop {
                if let Err(e) = Self::forward_one_round(&mut rx, &mut sink, send_timeout).await {
                    tracing::error!(?e, "forward error");
                    break;
                }
            }
            rx.close();
            let close_ret = timeout(Duration::from_secs(5), sink.close()).await;
            tracing::warn!(?close_ret, "mpsc close sink");
        });

        Self {
            tx: Some(tx),
            tunnel,
            stream: Some(stream),
            task: task.into(),
        }
    }

    async fn forward_one_round(
        rx: &mut Receiver<ZCPacket>,
        sink: &mut Pin<Box<dyn ZCPacketSink>>,
        send_timeout_ms: Option<Duration>,
    ) -> Result<(), TunnelError> {
        let item = rx.recv().await.with_context(|| "recv error")?;
        if let Some(timeout_ms) = send_timeout_ms {
            Self::forward_one_round_with_timeout(rx, sink, item, timeout_ms).await
        } else {
            Self::forward_one_round_no_timeout(rx, sink, item).await
        }
    }

    async fn forward_one_round_no_timeout(
        rx: &mut Receiver<ZCPacket>,
        sink: &mut Pin<Box<dyn ZCPacketSink>>,
        initial_item: ZCPacket,
    ) -> Result<(), TunnelError> {
        sink.feed(initial_item).await?;

        while let Ok(item) = rx.try_recv() {
            if let Err(e) = sink.feed(item).await {
                tracing::error!(?e, "feed error");
                return Err(e);
            }
        }

        sink.flush().await
    }

    async fn forward_one_round_with_timeout(
        rx: &mut Receiver<ZCPacket>,
        sink: &mut Pin<Box<dyn ZCPacketSink>>,
        initial_item: ZCPacket,
        timeout_ms: Duration,
    ) -> Result<(), TunnelError> {
        match timeout(timeout_ms, async move {
            Self::forward_one_round_no_timeout(rx, sink, initial_item).await
        })
        .await
        {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => {
                tracing::error!(?e, "forward error");
                Err(e)
            }
            Err(e) => {
                tracing::error!(?e, "forward timeout");
                Err(e.into())
            }
        }
    }

    pub fn get_stream(&mut self) -> Pin<Box<dyn ZCPacketStream>> {
        self.stream.take().unwrap()
    }

    pub fn get_sink(&self) -> MpscTunnelSender {
        MpscTunnelSender(self.tx.as_ref().unwrap().clone())
    }

    pub fn close(&mut self) {
        self.tx.take();
        self.task.abort();
    }

    pub fn tunnel_info(&self) -> Option<TunnelInfo> {
        self.tunnel.info()
    }
}
