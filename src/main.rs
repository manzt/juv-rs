use anyhow::Result;
use clap::builder::styling::{AnsiColor, Effects};
use clap::builder::Styles;
use clap::{Parser, Subcommand, ValueEnum};
use std::io::Write as _;

mod commands;
mod printer;

// Configures Clap v3-style help menu colors
const STYLES: Styles = Styles::styled()
    .header(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .usage(AnsiColor::Green.on_default().effects(Effects::BOLD))
    .literal(AnsiColor::Cyan.on_default().effects(Effects::BOLD))
    .placeholder(AnsiColor::Cyan.on_default());

fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[derive(Parser)]
#[command(name = "juv", author, long_version = version())]
#[command(about = "A fast toolkit for reproducible Jupyter notebooks")]
#[command(styles=STYLES)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
    /// Increase verbosity
    #[arg(short, long, action, conflicts_with = "quiet", global = true)]
    verbose: bool,
    /// Suppress all output
    #[arg(short, long, action, conflicts_with = "verbose", global = true)]
    quiet: bool,
}

#[derive(ValueEnum, Debug, Clone)]
#[clap(rename_all = "kebab_case")]
enum VersionOutputFormat {
    Text,
    Json,
}

#[derive(Subcommand)]
enum Commands {
    /// Preview the contents of a notebook
    Cat {
        /// The file to display
        file: std::path::PathBuf,
        /// Display the file as python script
        #[arg(long, action)]
        script: bool,
        /// A pager to use for displaying the contents
        #[arg(long, env = "JUV_PAGER")]
        pager: Option<String>,
    },
    /// Initialize a new notebook
    Init {
        /// The name of the project
        file: Option<std::path::PathBuf>,
        /// The interpreter version specifier
        #[arg(short, long)]
        python: Option<String>,
    },
    /// Launch a notebook or script in a Jupyter front end
    Run,
    /// Execute a notebook as a script
    Exec,
    /// Add dependencies to a notebook
    Add,
    /// Clear notebook cell outputs
    Clear,
    /// Display juv's version
    Version {
        #[arg(long, default_value = "text", value_enum)]
        output_format: VersionOutputFormat,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let printer = match (cli.verbose, cli.quiet) {
        (true, false) => printer::Printer::Verbose,
        (false, true) => printer::Printer::Quiet,
        _ => printer::Printer::Default,
    };
    match Cli::parse().command {
        Commands::Version { output_format } => {
            match output_format {
                VersionOutputFormat::Text => {
                    std::io::stdout().write_all(format!("juv {}", version()).as_bytes())?;
                }
                VersionOutputFormat::Json => {
                    let json = serde_json::json!({ "version": version() });
                    std::io::stdout().write_all(serde_json::to_string(&json)?.as_bytes())?;
                }
            };
            std::io::stdout().write_all(b"\n")?;
            Ok(())
        }
        Commands::Init { file, python } => {
            commands::init(&printer, file.as_deref(), python.as_deref())
        }
        Commands::Cat {
            file,
            script,
            pager,
        } => commands::cat(&printer, &file, script, pager.as_deref()),
        _ => unimplemented!(),
    }
}
