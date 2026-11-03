mod client;
mod config;
mod control;
mod overlay_geom;
mod paths;
mod protocol;
mod server;

use std::process::ExitCode;

const USAGE: &str = "usage:
  koshi                  attach to the server, starting it when needed
  koshi kill             stop the running server
  koshi server [--daemon]
  koshi --version";

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(code) => ExitCode::from(code as u8),
        Err(err) => {
            eprintln!("koshi: {err:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<i32> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => client::attach(config::Config::load()).await,
        Some("--version") | Some("-V") => {
            println!("koshi {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            Ok(0)
        }
        Some("kill") => {
            if let Some(extra) = args.get(1) {
                return unknown_argument(extra);
            }
            control::kill().await
        }
        Some("server") => {
            let daemon = match args.get(1).map(String::as_str) {
                None => false,
                Some("--daemon") => true,
                Some(other) => return unknown_argument(other),
            };
            if let Some(extra) = args.get(2) {
                return unknown_argument(extra);
            }
            server::run(daemon).await
        }
        Some(other) => unknown_argument(other),
    }
}

fn unknown_argument(arg: &str) -> anyhow::Result<i32> {
    eprintln!("koshi: unknown argument `{arg}`");
    eprintln!("{USAGE}");
    Ok(2)
}
