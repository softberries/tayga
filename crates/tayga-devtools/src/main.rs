use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tayga-devtools", about = "Developer tooling for the Tayga demo environment")]
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
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Flag { name, variant, file } => {
            tayga_devtools::flags::set_flag(&file, &name, &variant)?;
            println!("{name} -> {variant}");
        }
        Cmd::VerifyRaw { samples, clickhouse, jaeger } => {
            let (checked, mismatches) = tayga_devtools::verify::verify_raw(&clickhouse, &jaeger, samples).await?;
            for (trace, ours, theirs) in &mismatches {
                println!("MISMATCH {trace}: tayga={ours} jaeger={theirs}");
            }
            println!("checked {checked} traces, {} mismatches", mismatches.len());
            anyhow::ensure!(checked > 0, "no traces sampled; is the stack running?");
            anyhow::ensure!(mismatches.is_empty(), "span counts differ");
        }
    }
    Ok(())
}
