#![allow(dead_code)]
use anyhow::Result;
use std::{collections::HashMap, future::Future, pin::Pin};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt, Layer,
};

/// Tower的基础概念和由来
/// 逐步完善这个Trait的定义和实现

/// 模拟Request
#[derive(Debug)]
struct MockRequest {
    url: String,
}
/// 模拟Response
#[derive(Debug)]
struct MockResponse {
    url: String,
    headers: HashMap<String, String>,
    body: String,
}

#[derive(Debug)]
struct Server;

impl Server {
    async fn run<H>(self, mut handler: H) -> Result<()>
    where
        H: EvoHandler<MockRequest, Response = MockResponse, Error = anyhow::Error>,
    {
        // Mock Request
        let request = MockRequest {
            url: "http://www.mockapi.com".to_string(),
        };

        // 交给Handler
        let response = handler.call(request).await;

        println!("Response: {:?}", response);

        Ok(())
    }
}

// 第一版 阻塞实现 Fn(MockRequest)->MockResponse
fn say_hello_handler(req: MockRequest) -> MockResponse {
    MockResponse {
        url: req.url,
        headers: HashMap::new(),
        body: "Hello,world".to_string(),
    }
}

// 第二版 非阻塞实现 Fn(MockRequest)->impl Future<Output=MockResponse>
fn say_hello_handler_async(req: MockRequest) -> impl Future<Output = MockResponse> {
    let response = MockResponse {
        url: req.url,
        headers: HashMap::new(),
        body: "Hello,world".to_string(),
    };

    async move { response }
}

/// 第三版 定义Handler Trait 实现服务的组合
trait BasicHandler {
    /// 响应类型 实现 Future<Output = Result<MockResponse,Error>>
    type Future: Future<Output = Result<MockResponse>>;

    // 调用函数
    fn call(&mut self, request: MockRequest) -> Self::Future;
}

#[derive(Debug, Clone)]
struct BasicSayHelloHandler;
impl BasicHandler for BasicSayHelloHandler {
    type Future = Pin<Box<dyn Future<Output = Result<MockResponse>>>>;

    fn call(&mut self, request: MockRequest) -> Self::Future {
        Box::pin(async move {
            let response = MockResponse {
                url: request.url,
                headers: HashMap::new(),
                body: "Hello,world".to_string(),
            };

            tokio::time::sleep(std::time::Duration::from_millis(1000)).await;

            Ok(response)
        })
    }
}

#[derive(Debug, Clone)]
struct BasicTimeoutHandler<T> {
    inner_handler: T,
    duration: std::time::Duration,
}
impl<T> BasicHandler for BasicTimeoutHandler<T>
where
    // Clone 解决了所有权问题 'static生命周期避免传入引用
    T: BasicHandler + Clone + 'static,
{
    type Future = Pin<Box<dyn Future<Output = Result<MockResponse>>>>;

    fn call(&mut self, request: MockRequest) -> Self::Future {
        // 获取拥有所有权的 &mut self
        // 等于Clone::clone(&self)  Clone::clone(self.inner_handler) Clone::clone(self.duration)
        // 同理 传入的Handler也需要Clone
        let mut this = self.clone();

        Box::pin(async move {
            let result =
                tokio::time::timeout(this.duration, this.inner_handler.call(request)).await;

            match result {
                Ok(response) => response,
                Err(_) => Err(anyhow::anyhow!("Timeout")),
            }
        })
    }
}

impl<T> BasicTimeoutHandler<T> {
    fn new(handler: T, duration: std::time::Duration) -> Self {
        Self {
            inner_handler: handler,
            duration,
        }
    }
}

/// 第四版 定义更加灵活的 Handler Trait
/// 参数值使用 泛型
trait EvoHandler<Request> {
    /// 返回值使用关联类型
    type Response;
    type Error;

    type Future: Future<Output = Result<Self::Response, Self::Error>>;
    // 调用方法
    fn call(&mut self, request: Request) -> Self::Future;
}

#[derive(Debug, Clone, Default)]
struct EvoSayHelloHandler {
    request_duration: std::time::Duration,
}
impl EvoHandler<MockRequest> for EvoSayHelloHandler {
    type Error = anyhow::Error;
    type Response = MockResponse;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn call(&mut self, request: MockRequest) -> Self::Future {
        let this = self.clone();

        Box::pin(async move {
            let response = MockResponse {
                url: request.url,
                headers: HashMap::new(),
                body: "Evo Hello World!".to_string(),
            };
            // 模拟请求延迟
            tokio::time::sleep(this.request_duration).await;
            Ok(response)
        })
    }
}

#[derive(Debug, Clone)]
struct EvoTimeoutHandler<T> {
    inner_handler: T,
    duration: std::time::Duration,
}

impl<Request, T> EvoHandler<Request> for EvoTimeoutHandler<T>
where
    Request: 'static,
    T: EvoHandler<Request> + Clone + 'static,
    T::Error: From<tokio::time::error::Elapsed>,
{
    type Response = T::Response;
    type Error = T::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn call(&mut self, request: Request) -> Self::Future {
        // clone self
        let mut this = self.clone();

        Box::pin(async move {
            let result =
                tokio::time::timeout(this.duration, this.inner_handler.call(request)).await;

            match result {
                Ok(response) => response,
                Err(elapsed) => Err(T::Error::from(elapsed)),
            }
        })
    }
}

impl<T> EvoTimeoutHandler<T> {
    fn new(handler: T, duration: std::time::Duration) -> Self {
        Self {
            inner_handler: handler,
            duration,
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let console_layer = tracing_subscriber::fmt::Layer::new()
        .with_span_events(FmtSpan::CLOSE)
        .pretty()
        .with_filter(LevelFilter::INFO);

    tracing_subscriber::registry().with(console_layer).init();

    let server: Server = Server;

    let say_hello_handler = EvoSayHelloHandler {
        request_duration: std::time::Duration::from_secs(1),
    };

    let timeout_handler =
        EvoTimeoutHandler::new(say_hello_handler, std::time::Duration::from_millis(500));

    server.run(timeout_handler).await?;

    Ok(())
}
