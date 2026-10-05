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
