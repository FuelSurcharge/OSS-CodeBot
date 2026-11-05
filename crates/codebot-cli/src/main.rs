use clap::Parser;
use codebot_core::{execute_backend, Backend, Config};
use std::{
    io::{self, Read},
    path::PathBuf,
    process::ExitCode,
};
#[derive(Parser)]
#[command(version, about = "OSS CodeBot: local diagnostics and AI APIs")]
struct Args {
    /// Input text; reads stdin when omitted
    input: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
}
fn run() -> Result<(), String> {
    let args = Args::parse();
    let c = Config::load(args.config.as_deref())?;
    let input = match args.input {
        Some(s) => s,
        None => {
            let mut s = String::new();
            io::stdin()
                .take((c.max_input_chars as u64 * 4) + 1)
                .read_to_string(&mut s)
                .map_err(|_| "標準入力を読み込めません")?;
            s
        }
    };
    let backend = Backend::from_id(&c.provider).ok_or("対応していないプロバイダーです")?;
    let output = execute_backend(&c, backend, &c.api, None, &input)?;
    println!("{output}");
    eprintln!("INFO request completed");
    Ok(())
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("ERROR {e}");
            ExitCode::FAILURE
        }
    }
}
