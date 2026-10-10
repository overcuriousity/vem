//! vem: Vestigia Ex Machina. Forensic analysis of agentic AI harness traces.

mod commands;

use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(
    name = "vem",
    version,
    about = "Forensic analysis of agentic AI harness traces"
)]
struct Cli {
    /// Print reports as JSON instead of text.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Case management.
    Case {
        #[command(subcommand)]
        cmd: CaseCmd,
    },
    /// Evidence roots (collected harness directories).
    Evidence {
        #[command(subcommand)]
        cmd: EvidenceCmd,
    },
    /// Parse every unparsed file of every attached root.
    Ingest {
        case: PathBuf,
        #[arg(long)]
        root: Option<i64>,
    },
    /// Re-hash evidence and retained copies against the manifest.
    Verify { case: PathBuf },
    /// Roots, stores, absent stores, files, anomalies and audit log.
    Inventory { case: PathBuf },
    /// List sessions.
    Sessions {
        case: PathBuf,
        #[arg(long)]
        root: Option<i64>,
        #[arg(long)]
        kind: Option<String>,
    },
    /// Export a timeline.
    Export {
        case: PathBuf,
        #[arg(long, value_enum)]
        format: ExportFormat,
        #[arg(long)]
        root: Option<i64>,
        #[arg(long)]
        session: Option<i64>,
        #[arg(short, long)]
        output: PathBuf,
    },
}

#[derive(Subcommand)]
enum CaseCmd {
    /// Create a new case directory.
    New {
        dir: PathBuf,
        #[arg(long)]
        name: String,
        #[arg(long)]
        examiner: Option<String>,
    },
}

#[derive(Subcommand)]
enum EvidenceCmd {
    /// Attach a collected harness directory.
    Add {
        case: PathBuf,
        path: PathBuf,
        #[arg(long)]
        label: String,
        #[arg(long)]
        host: Option<String>,
        #[arg(long)]
        user: Option<String>,
        #[arg(long)]
        os: Option<String>,
        #[arg(long, value_enum)]
        harness: Option<HarnessArg>,
        #[arg(long)]
        no_retain: bool,
    },
    /// List attached roots.
    List { case: PathBuf },
}

#[derive(Clone, Copy, ValueEnum)]
enum HarnessArg {
    ClaudeCode,
    Codex,
    Cursor,
    CursorIde,
}

impl From<HarnessArg> for vem_core::model::Harness {
    fn from(h: HarnessArg) -> Self {
        match h {
            HarnessArg::ClaudeCode => Self::ClaudeCode,
            HarnessArg::Codex => Self::Codex,
            HarnessArg::Cursor => Self::Cursor,
            HarnessArg::CursorIde => Self::CursorIde,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum ExportFormat {
    TimesketchJsonl,
    TimesketchCsv,
    VestigoParquet,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Case {
            cmd:
                CaseCmd::New {
                    dir,
                    name,
                    examiner,
                },
        } => commands::case_new(&dir, &name, examiner.as_deref(), cli.json),
        Command::Evidence {
            cmd:
                EvidenceCmd::Add {
                    case,
                    path,
                    label,
                    host,
                    user,
                    os,
                    harness,
                    no_retain,
                },
        } => commands::evidence_add(
            &case,
            &path,
            label,
            host,
            user,
            os,
            harness.map(Into::into),
            !no_retain,
            cli.json,
        ),
        Command::Evidence {
            cmd: EvidenceCmd::List { case },
        } => commands::evidence_list(&case, cli.json),
        Command::Ingest { case, root } => commands::ingest(&case, root, cli.json),
        Command::Verify { case } => commands::verify(&case, cli.json),
        Command::Inventory { case } => commands::inventory(&case, cli.json),
        Command::Sessions { case, root, kind } => commands::sessions(&case, root, kind, cli.json),
        Command::Export {
            case,
            format,
            root,
            session,
            output,
        } => commands::export(&case, format, root, session, &output, cli.json),
    };
    match result {
        Ok(()) => {}
        Err(vem_case::CaseError::Unrecognized { path, hint }) => {
            eprintln!(
                "error: {} is not recognized as a harness directory{}",
                path.display(),
                hint
            );
            std::process::exit(2);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
