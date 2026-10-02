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
    }
    Ok(())
}
