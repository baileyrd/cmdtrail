mod db;
mod duration;
mod git;
mod ignore;
mod picker;
mod rank;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Parser)]
#[command(name = "cmdtrail", about = "Per-directory command history and suggestions")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Record a command that was run (called by the shell hook).
    Log {
        command: String,
        #[arg(long)]
        cwd: String,
        #[arg(long)]
        shell: String,
        #[arg(long = "exit-code")]
        exit_code: Option<i32>,
    },
    /// Print ranked command suggestions for a directory.
    Suggest {
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long, default_value_t = 10)]
        limit: usize,
        /// Only consider commands starting with this text.
        #[arg(long)]
        query: Option<String>,
        /// Launch the interactive picker and print only the chosen command.
        #[arg(long)]
        pick: bool,
    },
    /// Export all history as JSON Lines (one JSON object per line),
    /// oldest first, to stdout or a file.
    Export {
        #[arg(long)]
        out: Option<String>,
    },
    /// Delete history entries older than a given age (e.g. "90d", "6m").
    Prune {
        /// Duration suffix: h(ours), d(ays), w(eeks), m(onths, ~30d), y(ears, ~365d).
        #[arg(long = "older-than", value_parser = duration::parse)]
        older_than: i64,
        /// Report how many entries would be deleted without deleting them.
        #[arg(long)]
        dry_run: bool,
        /// Reclaim disk space after deleting (rewrites the whole DB file).
        #[arg(long)]
        vacuum: bool,
    },
    /// Print the shell hook script for the given shell.
    Init { shell: ShellKind },
}

#[derive(Clone, ValueEnum)]
enum ShellKind {
    Bash,
    Zsh,
    Pwsh,
}

fn now_ts() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let db_path = db::default_db_path()?;
    let database = db::Db::open(&db_path)?;

    match cli.command {
        Command::Log {
            command,
            cwd,
            shell,
            exit_code,
        } => {
            let patterns = ignore::load_patterns();
            if !ignore::is_ignored(&patterns, &command) {
                let cwd = git::normalize_path(&cwd);
                let git_root = git::find_git_root(std::path::Path::new(&cwd))
                    .map(|p| git::normalize_path(&p.to_string_lossy()));
                database.log(&command, &cwd, git_root.as_deref(), &shell, exit_code, now_ts())?;
            }
        }
        Command::Suggest {
            cwd,
            limit,
            query,
            pick,
        } => {
            let cwd = match cwd {
                Some(c) => c,
                None => std::env::current_dir()
                    .context("could not determine current directory")?
                    .to_string_lossy()
                    .to_string(),
            };
            let cwd = git::normalize_path(&cwd);
            let git_root = git::find_git_root(std::path::Path::new(&cwd))
                .map(|p| git::normalize_path(&p.to_string_lossy()));

            let entries = database.candidates(&cwd, git_root.as_deref())?;
            let ranked_limit = if pick { 200 } else { limit };
            let ranked = rank::rank(&entries, &cwd, git_root.as_deref(), now_ts(), query.as_deref(), ranked_limit);

            if pick {
                let items: Vec<String> = ranked.into_iter().map(|r| r.command).collect();
                if let Some(choice) = picker::pick(&items)? {
                    println!("{}", choice);
                }
            } else {
                for r in ranked.into_iter().take(limit) {
                    println!("{}", r.command);
                }
            }
        }
        Command::Export { out } => {
            let count = match out {
                Some(path) => {
                    let file = std::fs::File::create(&path)
                        .with_context(|| format!("could not create {path}"))?;
                    let mut writer = std::io::BufWriter::new(file);
                    database.export_all(&mut writer)?
                }
                None => {
                    let stdout = std::io::stdout();
                    let mut writer = std::io::BufWriter::new(stdout.lock());
                    database.export_all(&mut writer)?
                }
            };
            // Status goes to stderr: stdout carries the JSON Lines data
            // itself when --out isn't given, and must stay clean for
            // piping (e.g. `cmdtrail export > history.jsonl`).
            eprintln!("exported {count} entries");
        }
        Command::Prune {
            older_than,
            dry_run,
            vacuum,
        } => {
            let cutoff = now_ts() - older_than;
            if dry_run {
                let count = database.count_older_than(cutoff)?;
                println!("{count} entries older than the cutoff would be deleted (dry run, nothing changed)");
            } else {
                let deleted = database.prune_older_than(cutoff)?;
                println!("deleted {deleted} entries");
                if vacuum {
                    database.vacuum()?;
                    println!("vacuumed database");
                }
            }
        }
        Command::Init { shell } => {
            let script = match shell {
                ShellKind::Bash => include_str!("../hooks/cmdtrail.bash"),
                ShellKind::Zsh => include_str!("../hooks/cmdtrail.zsh"),
                ShellKind::Pwsh => include_str!("../hooks/cmdtrail.ps1"),
            };
            print!("{}", script);
        }
    }

    Ok(())
}
