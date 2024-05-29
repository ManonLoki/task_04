use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::State,
    http::StatusCode,
    response::{sse::Event, IntoResponse, Sse},
    routing::{get, post},
    Json,
};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio_stream::{wrappers::errors::BroadcastStreamRecvError, Stream, StreamExt};
use tower_http::cors::{self, CorsLayer};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    fmt::{format::FmtSpan, Layer},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
    Layer as _,
};

/// 包装广播通道
struct BroadcastWrapper {
    sender: tokio::sync::broadcast::Sender<Event>,
}

impl BroadcastWrapper {
    pub fn new() -> Self {
        let (sender, receiver) = tokio::sync::broadcast::channel(10);
        // Leak掉Receiver 否则Sender会被回收掉
        Box::leak(receiver.into());
        Self { sender }
    }
    /// 发送消息 向通道中发送消息
    pub async fn send(&self, message: String) {
        self.sender.send(Event::default().data(message)).unwrap();
    }

    /// 订阅Sender获取Receiver
    pub fn receiver(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

#[derive(Debug, Deserialize)]
pub struct SsePayload {
    pub message: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let console_layer = Layer::new()
        .with_span_events(FmtSpan::CLOSE)
        .pretty()
        .with_filter(LevelFilter::INFO);

    tracing_subscriber::registry().with(console_layer).init();

    let addr = "0.0.0.0:3000";

    let cors_layer = CorsLayer::new().allow_origin(cors::Any);

    let state = Arc::new(BroadcastWrapper::new());

    let app = axum::Router::new()
        .route("/", post(send_msg))
        .route("/sse", get(sse_handler))
        .layer(cors_layer)
        .with_state(state);

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Listening on: {}", addr);

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

/// 发送消息
async fn send_msg(
    broadcast_wrapper: State<Arc<BroadcastWrapper>>,
    Json(payload): Json<SsePayload>,
) -> impl IntoResponse {
    broadcast_wrapper.send(payload.message).await;

    StatusCode::OK
}

/// 注册SSR通道
async fn sse_handler(
    State(broadcast_wrapper): State<Arc<BroadcastWrapper>>,
) -> Sse<impl Stream<Item = Result<Event, BroadcastStreamRecvError>>> {
    // 将Broadcast Receiver转换为Stream
    let stream = tokio_stream::wrappers::BroadcastStream::new(broadcast_wrapper.receiver());
    // 过滤掉错误的消息
    let stream = stream.filter_map(|result| match result {
        Ok(item) => Some(Ok(item)),
        Err(_) => None,
    });

    // 返回Sse Stream
    Sse::new(stream)
}
