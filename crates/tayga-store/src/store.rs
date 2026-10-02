use crate::migrate::ClickHouseSettings;
use crate::rows::{LogRow, SpanRow};
use clickhouse::Client;

#[derive(Clone)]
pub struct Store {
    client: Client,
}

impl Store {
    pub fn new(s: &ClickHouseSettings) -> Self {
        Self {
            client: Client::default()
                .with_url(&s.url)
                .with_database(&s.database),
        }
    }

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn insert_spans(&self, rows: &[SpanRow]) -> clickhouse::error::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert::<SpanRow>("spans").await?;
        for row in rows {
            insert.write(row).await?;
        }
        insert.end().await
    }

    pub async fn insert_logs(&self, rows: &[LogRow]) -> clickhouse::error::Result<()> {
        if rows.is_empty() {
            return Ok(());
        }
        let mut insert = self.client.insert::<LogRow>("logs").await?;
        for row in rows {
            insert.write(row).await?;
        }
        insert.end().await
    }
}
