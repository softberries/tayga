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
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().cmd {
        Cmd::Flag { name, variant, file } => {
            tayga_devtools::flags::set_flag(&file, &name, &variant)?;
            println!("{name} -> {variant}");
        }
    }
    Ok(())
}
