use std::{collections::BTreeMap, sync::Arc};
use std::{io, marker::PhantomData, pin::Pin};

use anyhow::Result;
use bytes::{BufMut as _, Bytes, BytesMut};
use futures_lite::stream::{Stream, StreamExt};
use futures_util::sink::SinkExt;
use iroh::{
    blobs::Hash,
    net::{
        endpoint::{get_remote_node_id, RecvStream, SendStream},
        NodeId,
    },
    node::ProtocolHandler,
};
use serde::{Deserialize, Serialize};
use tauri::async_runtime::RwLock;
use tokio::sync::mpsc;
use tokio_serde::{Deserializer, Serializer};

pub const ALPN: &[u8] = b"iroh-drop/0";

#[derive(Debug)]
pub struct Protocol {
    name: String,
    known_nodes: RwLock<BTreeMap<NodeId, RemoteNode>>,
    client: iroh::client::Iroh,
    endpoint: iroh::net::Endpoint,
    s: mpsc::Sender<LocalProtocolMessage>,
}

#[derive(Debug, Clone)]
struct RemoteNode {
    /// Name of the remote node
    name: String,
}

impl ProtocolHandler for Protocol {
    fn accept(
        self: Arc<Self>,
        connecting: iroh::net::endpoint::Connecting,
    ) -> futures_lite::future::Boxed<Result<()>> {
        Box::pin(async move {
            // Wait for the connection to be fully established.
            let connection = connecting.await?;
            // We can get the remote's node id from the connection.
            let node_id = get_remote_node_id(&connection)?;
            println!("accepted connection from {node_id}");

            // Our protocol is a simple request-response protocol, so we expect the
            // connecting peer to open a single bi-directional stream.
            let (send_stream, recv_stream) = connection.accept_bi().await?;
            let (mut reader, mut writer) = wrap_streams(send_stream, recv_stream);

            let this = self.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(message) = reader.next().await {
                    match message {
                        Ok(message) => {
                            match message {
                                ProtocolMessage::IntroRequest { name } => {
                                    this.known_nodes
                                        .write()
                                        .await
                                        .insert(node_id, RemoteNode { name });

                                    if let Err(err) = writer
                                        .send(ProtocolMessage::IntroResponse {
                                            name: self.name.clone(),
                                        })
                                        .await
                                    {
                                        eprintln!("failed to send: {:?}", err);
                                    }
                                }
                                ProtocolMessage::IntroResponse { name } => {
                                    this.known_nodes
                                        .write()
                                        .await
                                        .insert(node_id, RemoteNode { name });
                                }
                                ProtocolMessage::SendRequest { name, hash, size } => {
                                    if let Some(info) = self.known_nodes.read().await.get(&node_id)
                                    {
                                        // TODO: ask for accepting
                                        println!("incoming request for {name}: {hash}: {size}bytes from {}", info.name);
                                        // TODO: spawn?
                                        match self
                                            .client
                                            .blobs()
                                            .download(hash, node_id.into())
                                            .await
                                        {
                                            Ok(res) => match res.await {
                                                Ok(res) => {
                                                    println!("{:?}", res);
                                                    this.s.send(
                                                        LocalProtocolMessage::FileDownloaded {
                                                            name,
                                                            hash,
                                                            size,
                                                        },
                                                    ).await.ok();
                                                }
                                                Err(err) => {
                                                    eprintln!("failed to download {:?}", err);
                                                }
                                            },
                                            Err(err) => {
                                                eprintln!("failed to download {:?}", err);
                                            }
                                        }
                                    } else {
                                        println!("ignoring request for unknown node");
                                    }
                                }
                                ProtocolMessage::Finish => {
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            eprintln!("error: {:?}", err);
                        }
                    }
                }

                let mut writer = writer.into_inner().into_inner();
                writer.finish().ok();
                writer.stopped().await.ok();
            });

            Ok(())
        })
    }

    fn shutdown(self: Arc<Self>) -> futures_lite::future::Boxed<()> {
        Box::pin(async move {})
    }
}

pub enum LocalProtocolMessage {
    FileDownloaded { name: String, hash: Hash, size: u64 },
}

impl Protocol {
    pub fn new(
        name: String,
        client: iroh::client::Iroh,
        endpoint: iroh::net::Endpoint,
        s: mpsc::Sender<LocalProtocolMessage>,
    ) -> Arc<Self> {
        Arc::new(Self {
            name,
            client,
            endpoint,
            known_nodes: Default::default(),
            s,
        })
    }

    pub async fn is_known_node(&self, node_id: &NodeId) -> bool {
        self.known_nodes.read().await.contains_key(node_id)
    }

    pub async fn send_intro(&self, node_id: NodeId) -> Result<String> {
        let conn = self.endpoint.connect_by_node_id(node_id, ALPN).await?;
        let (send, recv) = conn.open_bi().await?;

        let (mut reader, mut writer) = wrap_streams(send, recv);

        writer
            .send(ProtocolMessage::IntroRequest {
                name: self.name.clone(),
            })
            .await?;

        let name = match reader.next().await {
            Some(Ok(ProtocolMessage::IntroResponse { name })) => name,
            Some(Ok(msg)) => {
                anyhow::bail!("unexpected response: {:?}", msg);
            }
            Some(Err(err)) => return Err(err.into()),
            None => anyhow::bail!("remote aborted"),
        };

        self.known_nodes
            .write()
            .await
            .insert(node_id, RemoteNode { name: name.clone() });

        writer.send(ProtocolMessage::Finish).await?;
        let mut writer = writer.into_inner().into_inner();
        writer.finish()?;
        writer.stopped().await?;

        Ok(name)
    }

    pub async fn send_file(
        &self,
        node_id: NodeId,
        file_name: String,
        file_data: Vec<u8>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.known_nodes.read().await.get(&node_id).is_some(),
            "unknown node"
        );

        let add_res = self.client.blobs().add_bytes(file_data).await?;

        let conn = self.endpoint.connect_by_node_id(node_id, ALPN).await?;
        let (send, recv) = conn.open_bi().await?;

        let (_reader, mut writer) = wrap_streams(send, recv);

        writer
            .send(ProtocolMessage::SendRequest {
                name: file_name,
                hash: add_res.hash,
                size: add_res.size,
            })
            .await?;

        writer.send(ProtocolMessage::Finish).await?;
        let mut writer = writer.into_inner().into_inner();
        writer.finish()?;
        writer.stopped().await?;

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ProtocolMessage {
    IntroRequest {
        /// The name of the node sending the request
        name: String,
    },
    IntroResponse {
        /// The name of the node answering
        name: String,
    },
    SendRequest {
        name: String,
        hash: Hash,
        size: u64,
    },
    Finish,
}

type RpcRead<R> = tokio_serde::SymmetricallyFramed<
    tokio_util::codec::FramedRead<R, tokio_util::codec::LengthDelimitedCodec>,
    ProtocolMessage,
    SymmetricalPostcard<ProtocolMessage>,
>;
type RpcWrite<W> = tokio_serde::SymmetricallyFramed<
    tokio_util::codec::FramedWrite<W, tokio_util::codec::LengthDelimitedCodec>,
    ProtocolMessage,
    SymmetricalPostcard<ProtocolMessage>,
>;

static_assertions::assert_impl_all!(RpcRead<RecvStream>: Stream<Item = std::io::Result<ProtocolMessage>>);

fn wrap_streams<R, W>(send_stream: W, recv_stream: R) -> (RpcRead<R>, RpcWrite<W>)
where
    W: tokio::io::AsyncWrite,
    R: tokio::io::AsyncRead,
{
    let transport = tokio_util::codec::FramedRead::new(
        recv_stream,
        tokio_util::codec::LengthDelimitedCodec::default(),
    );
    let framed_read = tokio_serde::SymmetricallyFramed::<_, ProtocolMessage, _>::new(
        transport,
        SymmetricalPostcard::<ProtocolMessage>::default(),
    );

    let transport = tokio_util::codec::FramedWrite::new(
        send_stream,
        tokio_util::codec::LengthDelimitedCodec::default(),
    );
    let framed_write = tokio_serde::SymmetricallyFramed::<_, ProtocolMessage, _>::new(
        transport,
        SymmetricalPostcard::<ProtocolMessage>::default(),
    );

    (framed_read, framed_write)
}

pub struct Postcard<Item, SinkItem> {
    _marker: PhantomData<(Item, SinkItem)>,
}

impl<Item, SinkItem> Default for Postcard<Item, SinkItem> {
    fn default() -> Self {
        Self::new()
    }
}

impl<Item, SinkItem> Postcard<Item, SinkItem> {
    pub fn new() -> Self {
        Self {
            _marker: PhantomData,
        }
    }
}

pub type SymmetricalPostcard<T> = Postcard<T, T>;

impl<Item, SinkItem> Deserializer<Item> for Postcard<Item, SinkItem>
where
    for<'a> Item: Deserialize<'a>,
{
    type Error = io::Error;

    fn deserialize(self: Pin<&mut Self>, src: &BytesMut) -> Result<Item, Self::Error> {
        postcard::from_bytes(&src).map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}

impl<Item, SinkItem> Serializer<SinkItem> for Postcard<Item, SinkItem>
where
    SinkItem: Serialize,
{
    type Error = io::Error;

    fn serialize(self: Pin<&mut Self>, data: &SinkItem) -> Result<Bytes, Self::Error> {
        postcard::experimental::serialized_size(data)
            .and_then(|size| postcard::to_io(data, BytesMut::with_capacity(size).writer()))
            .map(|writer| writer.into_inner().freeze())
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
    }
}
