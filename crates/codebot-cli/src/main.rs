use clap::Parser;
use codebot_core::{execute, Config, Rules};
use std::{
    io::{self, Read},
    path::PathBuf,
    process::ExitCode,
};
#[derive(Parser)]
#[command(version, about = "OSS CodeBot: offline log diagnostics")]
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
    let output = execute(&c, &Rules, &input)?;
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
