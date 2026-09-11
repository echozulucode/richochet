//! Richochet's debug CLI.
//!
//! Two jobs: convert on the command line (`just conv`), and interrogate the Windows clipboard
//! (`just clipdump`, `just capture`) during the Phase 1 spike.

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use mdcore::{Format, RenderProfile};

mod clipboard;
mod fixtures;

#[derive(Parser)]
#[command(name = "mdcli", version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Convert a file (or stdin) between formats.
    Convert {
        /// Input format.
        #[arg(long, value_enum)]
        from: CliFormat,
        /// Output format.
        #[arg(long, value_enum)]
        to: CliFormat,
        /// Input file; reads stdin when omitted.
        file: Option<PathBuf>,
    },
    /// Shorthand for `convert --from markdown --to html`.
    Md2teams {
        /// Input file; reads stdin when omitted.
        file: Option<PathBuf>,
    },
    /// Shorthand for `convert --from html --to markdown`.
    Teams2md {
        /// Input file; reads stdin when omitted.
        file: Option<PathBuf>,
    },
    /// Print every format currently on the clipboard, with a decoded preview.
    DumpClipboard,
    /// Save the current clipboard contents as a new fixture directory.
    Capture {
        /// Fixture name, used as the directory name under tests/fixtures/.
        #[arg(long)]
        name: String,
    },
    /// Export the fixture corpus as JSON, for the frontend's E2E conversion oracle.
    ExportFixtures {
        /// Where to write the JSON.
        #[arg(long, default_value = "src/test-support/oracle.json")]
        out: PathBuf,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum CliFormat {
    Markdown,
    Html,
    Text,
}

impl From<CliFormat> for Format {
    fn from(f: CliFormat) -> Self {
        match f {
            CliFormat::Markdown => Format::Markdown,
            CliFormat::Html => Format::Html,
            CliFormat::Text => Format::Text,
        }
    }
}

fn read_input(file: Option<PathBuf>) -> Result<String> {
    match file {
        Some(p) => std::fs::read_to_string(&p).with_context(|| format!("reading {}", p.display())),
        None => {
            use std::io::Read;
            let mut s = String::new();
            std::io::stdin().read_to_string(&mut s)?;
            Ok(s)
        }
    }
}

fn convert(input: &str, from: Format, to: Format) -> Result<String> {
    let profile = RenderProfile::teams().pretty();
    Ok(mdcore::convert_with(input, from, to, &profile)?)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Convert { from, to, file } => {
            let input = read_input(file)?;
            println!("{}", convert(&input, from.into(), to.into())?);
        }
        Command::Md2teams { file } => {
            let input = read_input(file)?;
            println!("{}", convert(&input, Format::Markdown, Format::Html)?);
        }
        Command::Teams2md { file } => {
            let input = read_input(file)?;
            println!("{}", convert(&input, Format::Html, Format::Markdown)?);
        }
        Command::DumpClipboard => clipboard::dump()?,
        Command::Capture { name } => clipboard::capture(&name)?,
        Command::ExportFixtures { out } => fixtures::export(&out)?,
    }
    Ok(())
}
