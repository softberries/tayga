use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "tayga-devtools",
    about = "Developer tooling for the Tayga demo environment"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Set a demo feature flag's defaultVariant (e.g. `flag paymentFailure 100%`).
    Flag {
        name: String,
        variant: String,
        #[arg(long, default_value = "deploy/flagd/demo.flagd.json")]
        file: PathBuf,
    },
    /// Compare span counts per trace between ClickHouse and Jaeger.
    VerifyRaw {
        #[arg(long, default_value_t = 20)]
        samples: u32,
        #[arg(long, default_value = "http://localhost:18123")]
        clickhouse: String,
        #[arg(long, default_value = "http://localhost:8080/jaeger/ui")]
        jaeger: String,
    },
    /// Capture complete traces from tayga.signals into a gzip fixture file.
    Capture {
        #[arg(long)]
        out: PathBuf,
        #[arg(long, default_value_t = 60)]
        seconds: u64,
        #[arg(long, default_value_t = 200)]
        max_traces: usize,
        #[arg(long)]
        only_errors: bool,
        /// Keep only traces with at least one span from this service.
        #[arg(long)]
        require_service: Option<String>,
        #[arg(long, default_value = "localhost:19092")]
        brokers: String,
        #[arg(long, default_value = "tayga.signals")]
        topic: String,
    },
    /// Send one OTLP/HTTP log record to an ingest endpoint and print its trace id.
    EmitLog {
        #[arg(long, default_value = "http://localhost:14318")]
        endpoint: String,
        #[arg(long)]
        service: String,
        #[arg(long)]
        body: String,
        /// 32 hex chars; random when omitted.
        #[arg(long)]
        trace_id: Option<String>,
        #[arg(long, default_value_t = 9)]
        severity: i32,
    },
    /// Print the Argon2id PHC string for `auth.password_hash`. Prompts twice without echo, or
    /// reads the first stdin line when piped.
    HashPassword,
    /// Rebuild log templates from the stored logs of the last 3 days with the logminer's current
    /// `[logminer]` config (same `TAYGA_CONFIG` file and `TAYGA__` env). Truncates
    /// `log_templates`, `log_template_hits` and `log_template_minutes`, then stores the
    /// watermark, a fresh masking epoch and the masking version. A real run refuses while any
    /// logminer replica's heartbeat is under 3 minutes old; `--dry-run` is read-only and always
    /// allowed. Stop every logminer replica first (`docker compose ... stop tayga-logminer`) and
    /// start them again afterwards.
    Remine {
        /// Mine and print the summary; write nothing.
        #[arg(long)]
        dry_run: bool,
        /// Run a real re-mine although a logminer heartbeat is fresh.
        #[arg(long)]
        force: bool,
        /// ClickHouse HTTP URL; default `clickhouse.url` of the config, else http://localhost:18123.
        #[arg(long)]
        clickhouse: Option<String>,
        /// Database name; default `clickhouse.database` of the config, else `tayga`.
        #[arg(long)]
        database: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Flag {
            name,
            variant,
            file,
        } => {
            tayga_devtools::flags::set_flag(&file, &name, &variant)?;
            println!("{name} -> {variant}");
        }
        Cmd::VerifyRaw {
            samples,
            clickhouse,
            jaeger,
        } => {
            let (checked, mismatches) =
                tayga_devtools::verify::verify_raw(&clickhouse, &jaeger, samples).await?;
            for (trace, ours, theirs) in &mismatches {
                println!("MISMATCH {trace}: tayga={ours} jaeger={theirs}");
            }
            println!("checked {checked} traces, {} mismatches", mismatches.len());
            anyhow::ensure!(checked > 0, "no traces sampled; is the stack running?");
            anyhow::ensure!(mismatches.is_empty(), "span counts differ");
        }
        Cmd::Capture {
            out,
            seconds,
            max_traces,
            only_errors,
            require_service,
            brokers,
            topic,
        } => {
            let envelopes = tayga_devtools::capture::capture(
                &brokers,
                &topic,
                seconds,
                max_traces,
                only_errors,
                require_service.as_deref(),
            )
            .await?;
            let file = std::fs::File::create(&out)?;
            let mut gz = flate2::write::GzEncoder::new(file, flate2::Compression::best());
            tayga_model::envelope::write_framed(&mut gz, &envelopes)?;
            gz.finish()?;
            println!("wrote {} envelopes to {}", envelopes.len(), out.display());
        }
        Cmd::EmitLog {
            endpoint,
            service,
            body,
            trace_id,
            severity,
        } => {
            let id = match trace_id {
                Some(hex) => parse_trace_id(&hex)?,
                None => tayga_devtools::emit::random_trace_id(),
            };
            tayga_devtools::emit::emit_log(&endpoint, &service, &body, &id, severity).await?;
            println!(
                "{}",
                id.iter().map(|b| format!("{b:02x}")).collect::<String>()
            );
        }
        Cmd::HashPassword => {
            let password = tayga_devtools::password::read_password()?;
            println!("{}", tayga_devtools::password::hash_password(&password)?);
        }
        Cmd::Remine {
            dry_run,
            force,
            clickhouse,
            database,
        } => remine(dry_run, force, clickhouse, database).await?,
    }
    Ok(())
}

fn parse_trace_id(hex: &str) -> anyhow::Result<[u8; 16]> {
    anyhow::ensure!(
        hex.len() == 32 && hex.is_ascii(),
        "trace id must be 32 hex chars"
    );
    let mut id = [0u8; 16];
    for (i, b) in id.iter_mut().enumerate() {
        *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16)?;
    }
    Ok(id)
}

/// The `[clickhouse]` keys a flag can override.
#[derive(serde::Deserialize, Default)]
struct ClickHouseSection {
    url: Option<String>,
    database: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct RemineConfig {
    #[serde(default)]
    clickhouse: ClickHouseSection,
}

async fn remine(
    dry_run: bool,
    force: bool,
    clickhouse: Option<String>,
    database: Option<String>,
) -> anyhow::Result<()> {
    let drain = tayga_logminer::config::DrainSettings::load()?.drain();
    let cfg: RemineConfig = tayga_common::load_settings()?;
    let store = tayga_store::store::Store::new(&tayga_store::ClickHouseSettings {
        url: clickhouse
            .or(cfg.clickhouse.url)
            .unwrap_or_else(|| "http://localhost:18123".to_string()),
        database: database
            .or(cfg.clickhouse.database)
            .unwrap_or_else(|| "tayga".to_string()),
    });
    let now_ns = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let summary = tayga_devtools::remine::remine(
        &store,
        &drain,
        tayga_devtools::remine::Options { dry_run, force },
        i64::try_from(now_ns)?,
    )
    .await?;
    print!("{}", summary.render(dry_run));
    Ok(())
}
