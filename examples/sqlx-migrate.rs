use anyhow::Result;
use sqlx::migrate;

#[tokio::main]
async fn main() -> Result<()> {
    let conn_str = "postgres://postgres:880914@localhost:5432/rust_study_demo";

    let pool = sqlx::PgPool::connect(conn_str).await?;

    migrate!("./migrations/").run(&pool).await?;

    Ok(())
}
