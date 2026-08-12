mod db;
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
