use std::{
    fmt,
    future::Future,
    pin::Pin,
    task::{ready, Poll},
};

use anyhow::Result;
use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
use pin_project::pin_project;
use tokio::net::TcpListener;
use tower::{Layer as TowerLayer, Service};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    fmt::{format::FmtSpan, Layer as ConsoleLayer},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
    Layer as _,
};

/// 存在问题
/// 1.为何Response了Err 但是依旧走的是Ok分支

#[derive(Debug, Clone)]
pub struct MyLogService<S> {
    inner: S,
}

impl<S> MyLogService<S> {
    pub fn new(inner: S) -> Self {
        Self { inner }
    }
}

impl<S, ReqBody, ResBody> Service<axum::http::Request<ReqBody>> for MyLogService<S>
where
    S: Service<axum::http::Request<ReqBody>, Response = axum::response::Response<ResBody>>,
    ResBody: Default + fmt::Debug,
    ReqBody: fmt::Debug,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = ResponseFuture<S::Future>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        tracing::info!("Call Log Service Poll Ready");
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: axum::http::Request<ReqBody>) -> Self::Future {
        tracing::info!("Request: {:?}", req);

        // 封装一个Future，在这里处理Response
        ResponseFuture {
            response_future: self.inner.call(req),
        }
    }
}

#[pin_project]
#[derive(Debug)]
pub struct ResponseFuture<F> {
    #[pin]
    response_future: F,
}

impl<F, B, E> Future for ResponseFuture<F>
where
    F: Future<Output = Result<axum::response::Response<B>, E>>,
    B: Default + fmt::Debug,
{
    type Output = F::Output;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // 拿到pin-project的self
        let this = self.project();
        // 运行Future
        let response = ready!(this.response_future.poll(cx))?;
        tracing::info!("Response: {:?}", response);
        Poll::Ready(Ok(response))
    }
}

// 包装成Layer
#[derive(Debug, Clone)]
pub struct MyLogLayer;
impl<S> TowerLayer<S> for MyLogLayer {
    type Service = MyLogService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MyLogService::new(inner)
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Tracing
    let console_layer = ConsoleLayer::new()
        .with_span_events(FmtSpan::CLOSE)
        .pretty()
        .with_filter(LevelFilter::INFO);
    tracing_subscriber::registry().with(console_layer).init();

    let addr = "0.0.0.0:3000";

    let tower_log_layer = MyLogLayer;
    let app = Router::new()
        .route("/", get(index_handler))
        .layer(tower_log_layer);

    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Listening on: {}", addr);

    axum::serve(listener, app.into_make_service()).await?;

    Ok(())
}

async fn index_handler() -> Result<&'static str, impl IntoResponse> {
    Err((StatusCode::INTERNAL_SERVER_ERROR).into_response())
}
