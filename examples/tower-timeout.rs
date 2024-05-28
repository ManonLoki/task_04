use core::fmt;
use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};

use anyhow::Result;
use pin_project::pin_project;
use tokio::time::Sleep;
use tower::{BoxError, Service};

/// 创建一个Timeout Service
#[derive(Debug, Clone)]
struct Timeout<S> {
    // 内部的Service
    inner: S,
    timeout: Duration,
}
impl<S> Timeout<S> {
    pub fn new(inner: S, timeout: Duration) -> Self {
        Self { inner, timeout }
    }
}
/// 实现ServiceTrait
impl<S, Request> Service<Request> for Timeout<S>
where
    S: Service<Request>,
    // 内部Service的Error能转换为BoxError
    S::Error: Into<BoxError>,
{
    type Response = S::Response;
    // 最终对外的Error是BoxError
    type Error = BoxError;
    /// 包装一个我们自己的Future 而不是通用的Future
    type Future = ResponseFuture<S::Future>;

    // 这里可以做背压
    // 这里会等待内部的Service Ready之后，这里会等内部的PollReady之后再调用Call
    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx).map_err(|err| err.into())
    }

    // 实际Service的逻辑
    fn call(&mut self, req: Request) -> Self::Future {
        // 创建一个内部调用的Future
        let response_future = self.inner.call(req);
        // 创建一个Sleep的Future
        let sleep = tokio::time::sleep(self.timeout);

        // 封装成我们自己的Future
        // 推动逻辑在这里实现
        ResponseFuture {
            response_future,
            sleep,
        }
    }
}

/// 创建响应Future结构，包含了 内部的响应Future和一个Sleep Future
/// Pin还不熟悉，但是异步要保证引用的数据是Pin的
#[pin_project]
pub struct ResponseFuture<F> {
    #[pin]
    response_future: F,
    #[pin]
    sleep: Sleep,
}

/// 为Response Future实现Future
impl<F, Response, Error> Future for ResponseFuture<F>
where
    F: Future<Output = Result<Response, Error>>,
    // 支持Into到 Box<dyn std::error::Error + Send + Sync>
    Error: Into<BoxError>,
{
    // 输出Result<Response,Error>
    type Output = Result<Response, BoxError>;

    // 轮询获取当前Future状态
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // 产生一个Pin Self
        let this = self.project();
        // 如果内部响应完成，直接响应完成
        match this.response_future.poll(cx) {
            Poll::Ready(result) => return Poll::Ready(result).map_err(|e| e.into()),
            Poll::Pending => {}
        }

        // 如果Sleep先完成 返回一个超时错误
        match this.sleep.poll(cx) {
            Poll::Ready(()) => {
                let error = Box::new(TimeoutError(()));
                return Poll::Ready(Err(error));
            }
            Poll::Pending => {}
        }
        // 否则Pending
        Poll::Pending
    }
}

/// 统一错误
#[derive(Debug, Default)]
pub struct TimeoutError(());
/// 实现Display
impl fmt::Display for TimeoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.pad("Request Timeout")
    }
}
/// 实现std::error:Error =  Display+Debug
impl std::error::Error for TimeoutError {}

/// 创建一个RootService作为Timeout的逻辑
struct RootService {
    is_timeout: bool,
}
impl RootService {
    fn new(is_timeout: bool) -> Self {
        Self { is_timeout }
    }
}

impl<Request> Service<Request> for RootService {
    type Error = BoxError;

    type Response = String;

    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>>>>;

    fn poll_ready(
        &mut self,
        cx: &mut Context<'_>,
    ) -> Poll<std::prelude::v1::Result<(), Self::Error>> {
        println!("Root Poll Ready Call");

        if self.is_timeout {
            Poll::Pending
        } else {
            cx.waker().wake_by_ref();
            Poll::Ready(Ok(()))
        }
    }

    fn call(&mut self, _req: Request) -> Self::Future {
        let is_timeout = self.is_timeout;
        Box::pin(async move {
            if is_timeout {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }

            Ok("Hello World".to_string())
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // True则触发超时
    let root_service = RootService::new(true);

    let mut timeout_service = Timeout::new(root_service, Duration::from_secs(1));

    // 这里不知道为何没有调用背压
    let result = timeout_service.call(()).await;

    match result {
        Ok(data) => println!("Response:{}", data),
        Err(e) => println!("Err:{}", e),
    }

    Ok(())
}
