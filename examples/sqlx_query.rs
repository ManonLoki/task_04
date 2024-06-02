use anyhow::Result;
use sqlx::{prelude::FromRow, PgPool};

#[derive(FromRow, Debug)]
pub struct DemoData {
    pub id: i32,
    pub data: Option<String>,
}

#[tokio::main]
pub async fn main() -> Result<()> {
    let pool = PgPool::connect(&dotenvy::var("DATABASE_URL")?).await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    println!("Migrations run successfully");

    let demo_data = sqlx::query_file_as!(DemoData, "queries/get_all_demo_records.sql")
        .fetch_all(&pool)
        .await?;

    tracing::info!("Demo data: {:?}", demo_data);

    Ok(())
}
