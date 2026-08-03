use clap::{Parser, Subcommand};
use freecode_pb::freecode_service_client::FreecodeServiceClient;
use freecode_pb::{IntentRequest, PingRequest};

const DAEMON_ADDR: &str = "http://127.0.0.1:50051";

pub mod freecode_pb {
    tonic::include_proto!("freecode");
}

#[derive(Parser)]
#[command(name = "freecode")]
// `version` wires up `--version` from the crate manifest — it was missing, so
// `freecode --version` failed with an "unexpected argument" error.
#[command(version, about = "Strict CLI client for the Freecode Daemon", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Ping the daemon to check if it's running
    Ping,
    /// Send an intent to the AI
    Ask {
        /// The instruction for the AI
        prompt: String,
        /// Mode of operation (chat, hitl, auto)
        #[arg(short, long, default_value = "hitl")]
        mode: String,
        /// Workspace path the daemon should operate on
        #[arg(short, long, default_value = ".")]
        workspace: String,
        /// Override LLM endpoint (empty = daemon default)
        #[arg(long, default_value = "")]
        endpoint: String,
        /// Override LLM model name (empty = daemon default)
        #[arg(long, default_value = "")]
        model: String,
        /// Session id (empty = daemon default). Use a fresh id per bench task.
        #[arg(long, default_value = "")]
        session: String,
    },
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // A bare transport error ("tcp connect error") tells the operator nothing actionable.
    let mut client = match FreecodeServiceClient::connect(DAEMON_ADDR).await {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "error: cannot reach the FreeCode daemon at {DAEMON_ADDR} ({e}).\n\
                 Start it with `cargo run --release -p freecode-daemon` or the launchd service, then retry."
            );
            std::process::exit(1);
        }
    };

    match &cli.command {
        Commands::Ping => {
            let request = tonic::Request::new(PingRequest {});
            let response = client.ping(request).await?;
            let resp = response.into_inner();
            println!("Daemon Version: {}", resp.version);
            println!("Daemon Status: {}", resp.status);
        }
        Commands::Ask { prompt, mode, workspace, endpoint, model, session } => {
            // Resolve workspace to an absolute path so the daemon (a separate
            // process) operates on the intended directory regardless of its cwd.
            let workspace_abs = std::fs::canonicalize(workspace)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| workspace.clone());
            let request = tonic::Request::new(IntentRequest {
                prompt: prompt.clone(),
                workspace_path: workspace_abs,
                mode: mode.clone(),
                session_id: session.clone(),
                llm_endpoint: endpoint.clone(),
                llm_model: model.clone(),
                approved_command: String::new(),
                selection: String::new(), // T1 fast-path is IDE-only; CLI never sets it
                file: String::new(),
            });
            let mut response_stream = client.dispatch_intent(request).await?.into_inner();
            while let Some(resp) = response_stream.message().await? {
                // stdout carries the model's answer and nothing else, so `freecode ask ... > out.md`
                // yields clean data; progress/diagnostics go to stderr.
                if resp.status == "token" {
                    print!("{}", resp.message);
                    use std::io::Write;
                    let _ = std::io::stdout().flush();
                } else if resp.status == "step" {
                    eprintln!("[step] {}", resp.message);
                } else {
                    eprintln!("[{}] {}", resp.status, resp.message);
                }
            }
            println!();
        }
    }

    Ok(())
}
