use std::sync::Arc;

use anyhow::Result;
use axum::{
    extract::{Host, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres};
use tokio::net::TcpListener;
use tower_http::cors::{self, CorsLayer};
use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    fmt::{format::FmtSpan, Layer},
    layer::SubscriberExt as _,
    util::SubscriberInitExt as _,
    Layer as _,
};

use sqlx::prelude::*;

use nanoid::nanoid;

/// 思路
/// 1. 使用Axum提供服务
/// 2. 用户提交长链接，返回短连接地址
/// 3. 用户访问短链接，重定向到原始链接
/// 4. nanoid 可能会重复，当重复时重新生成
/// 5. 使用this error 处理错误

/// 状态
pub struct AppState {
    db: PgPool,
}

/// Shortener 数据对象
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ShortenerDTO {
    url: String,
}

/// 定义Error
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("postgres sql error: {0}")]
    SqlError(#[from] sqlx::Error),
    #[error("parse header error: {0}")]
    HeaderError(#[from] axum::http::header::InvalidHeaderValue),
}
impl IntoResponse for AppError {
    fn into_response(self) -> axum::http::Response<axum::body::Body> {
        match self {
            AppError::SqlError(err) => match err {
                sqlx::Error::RowNotFound => {
                    let body = axum::body::Body::from("Data Not Found");
                    axum::http::Response::builder()
                        .status(StatusCode::NOT_FOUND)
                        .body(body)
                        .unwrap()
                }
                _ => {
                    let body = axum::body::Body::from(format!("EXECUTE SQL ERROR: {}", err));
                    axum::http::Response::builder()
                        .status(StatusCode::INTERNAL_SERVER_ERROR)
                        .body(body)
                        .unwrap()
                }
            },
            AppError::HeaderError(err) => {
                let body = axum::body::Body::from(format!("RESPONSE HEADER ERROR: {}", err));
                axum::http::Response::builder()
                    .status(StatusCode::INTERNAL_SERVER_ERROR)
                    .body(body)
                    .unwrap()
            }
        }
    }
}

#[derive(Debug, FromRow)]
pub struct Shortener {
    #[sqlx(default)]
    id: String,
    #[sqlx(default)]
    url: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    // 初始化日志
    let console_layer = Layer::new()
        .with_span_events(FmtSpan::CLOSE)
        .pretty()
        .with_filter(LevelFilter::INFO);

    tracing_subscriber::registry().with(console_layer).init();

    // 创建SQL连接
    let conn_str = "postgres://postgres:880914@localhost:5432/task_04";
    let pool = PgPool::connect(conn_str).await?;

    // 迁移数据
    sqlx::migrate!("./migrations").run(&pool).await?;

    let state = Arc::new(AppState { db: pool });

    // 构建axum路由
    let app = Router::new()
        .route("/", post(create_shorten))
        .route("/:id", get(visit_shorten))
        .layer(CorsLayer::new().allow_origin(cors::Any))
        .with_state(state);

    // 监听端口
    let addr = "0.0.0.0:3000";
    let listener = TcpListener::bind(addr).await?;
    tracing::info!("Listening on: {}", addr);
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}

async fn create_shorten(
    state: State<Arc<AppState>>,
    Host(host): Host,
    Json(payload): Json<ShortenerDTO>,
) -> Result<impl IntoResponse, AppError> {
    // 插入数据
    let sql = r#"
        INSERT INTO shortener (id,url)
        VALUES ($1,$2)
        ON CONFLICT (url)
        DO UPDATE SET url = EXCLUDED.url
        RETURNING id;
    "#;

    let id = loop {
        match sqlx::query_as::<Postgres, Shortener>(sql)
            .bind(nanoid!(6))
            .bind(&payload.url)
            .fetch_one(&state.db)
            .await
        {
            Ok(shortener) => break shortener.id,
            Err(sqlx::Error::Database(err)) => {
                // 只有违反id的唯一性约束时才会继续循环
                if err.is_foreign_key_violation() {
                    tracing::info!("Duplicated id, retrying");
                    continue;
                }

                return Err(sqlx::Error::Database(err).into());
            }
            Err(err) => return Err(err.into()),
        }
    };

    let response = ShortenerDTO {
        url: format!("http://{}/{}", host, id),
    };

    Ok(Json(response))
}

async fn visit_shorten(
    state: State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, AppError> {
    let sql = r#"
        SELECT url FROM shortener WHERE id = $1;
    "#;

    let shortener = sqlx::query_as::<Postgres, Shortener>(sql)
        .bind(id)
        .fetch_one(&state.db)
        .await?;

    let mut headers = HeaderMap::new();
    headers.insert("Location", shortener.url.parse()?);

    Ok((StatusCode::PERMANENT_REDIRECT, headers).into_response())
}
