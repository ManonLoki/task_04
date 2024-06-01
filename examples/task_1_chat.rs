use core::fmt;
use std::{net::SocketAddr, sync::Arc};

use anyhow::Result;
use dashmap::DashMap;
use futures_util::{stream::SplitStream, SinkExt, StreamExt};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::mpsc::Sender,
};

use tokio_util::codec::{Framed, LinesCodec};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    fmt::{format::FmtSpan, Layer},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
    Layer as _,
};

/// 思路
/// 1 监听端口
/// 2 处理每一个链接 将Addr+Sender 保存到全局 并且将自身的信息和Receiver封装为一个Peer返回
/// 3 当Peer进入，离开，以及收到消息时，广播给所有的Sender

/// 问题
/// 1. 处理整条消息链路时容易混乱
/// 2. 使用了 block_send 阻塞了整个线程 导致panic

/// 用时
/// 40分钟左右 其中查询Sink 和 SplitStream 的资料花了点时间

/// 通道内最大消息数量
const MAX_MESSAGE_COUNT: usize = 10;

#[derive(Debug, Default)]
pub struct State {
    map: DashMap<SocketAddr, Sender<String>>,
}

impl State {
    /// 加入
    pub fn join(
        &self,
        addr: SocketAddr,
        username: String,
        stream: Framed<TcpStream, LinesCodec>,
    ) -> Peer {
        // 创建Channel 并插入到Map中
        let (tx, mut rx) = tokio::sync::mpsc::channel(MAX_MESSAGE_COUNT);
        self.map.insert(addr, tx);

        // 拆分Steam
        let (mut sender, receiver) = stream.split();

        // 监听收到的消息
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if let Err(err) = sender.send(msg).await {
                    tracing::warn!("Send Message Error: {:?}", err);
                }
            }
        });

        // 创建并返回Peer
        Peer {
            username,
            stream: receiver,
        }
    }

    /// 离开
    pub fn leave(&self, addr: SocketAddr) {
        self.map.remove(&addr);
    }

    /// 广播
    pub async fn broadcast(&self, addr: SocketAddr, msg: Arc<Message>) {
        for sender in self.map.iter() {
            if sender.key() == &addr {
                continue;
            }
            if let Err(err) = sender.value().send(msg.to_string()).await {
                tracing::warn!("Broadcast Message Error: {:?}", err);
                // 发送失败时，将Peer移除
                self.leave(addr);
            }
        }
    }
}

#[derive(Debug)]
pub struct Peer {
    username: String,
    stream: SplitStream<Framed<TcpStream, LinesCodec>>,
}

#[derive(Debug)]
pub enum Message {
    Join(String),
    Leave(String),
    Broadcast { username: String, content: String },
}

impl fmt::Display for Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Message::Join(username) => write!(f, "{} join the chat", username),
            Message::Leave(username) => write!(f, "{} leave the chat", username),
            Message::Broadcast {
                username,
                content: message,
            } => write!(f, "{}: {}", username, message),
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    let console_layer = Layer::new()
        .with_span_events(FmtSpan::CLOSE)
        .pretty()
        .with_filter(LevelFilter::INFO);

    tracing_subscriber::registry().with(console_layer).init();
    // 监听端口
    let addr = "0.0.0.0:3000";
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Listening on: {}", addr);

    // 创建全局状态
    let state = Arc::new(State::default());

    loop {
        let (socket, addr) = listener.accept().await?;
        let state = state.clone();
        tokio::spawn(async move {
            tracing::info!("Accept Connection: {:?}", addr);

            if let Err(err) = handle_connection(socket, addr, state).await {
                tracing::warn!("Handle Connection Error: {:?}", err);
            }

            #[allow(dead_code)]
            Ok::<(), anyhow::Error>(())
        });
    }
}

async fn handle_connection(
    socket: tokio::net::TcpStream,
    addr: SocketAddr,
    state: Arc<State>,
) -> Result<()> {
    // 将socket包装为Framed 每一帧通过\n来分割
    let mut stream = Framed::new(socket, LinesCodec::new());

    stream.send("Please input your username:").await?;

    let username = match stream.next().await {
        Some(Ok(username)) => username,
        Some(Err(err)) => return Err(err.into()),
        None => anyhow::bail!("No username received"),
    };

    // 发送加入消息
    let msg = Message::Join(username.clone());
    state.broadcast(addr, Arc::new(msg)).await;

    let mut peer = state.join(addr, username, stream);

    // 接收消息
    while let Some(msg) = peer.stream.next().await {
        let msg = match msg {
            Ok(msg) => msg,
            Err(err) => {
                tracing::warn!("Receive Message Error: {:?}", err);
                break;
            }
        };

        tracing::info!("Receive Message: {}", msg);

        // 广播消息
        let msg = Message::Broadcast {
            username: peer.username.clone(),
            content: msg,
        };
        state.broadcast(addr, Arc::new(msg)).await;
    }

    // 当无法接受消息时 表示Peer已经离开
    state.leave(addr);
    let msg = Message::Leave(peer.username.clone());
    state.broadcast(addr, Arc::new(msg)).await;

    Ok(())
}
