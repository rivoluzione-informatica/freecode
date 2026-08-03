use tonic::transport::Server;

pub mod freecode_pb {
    tonic::include_proto!("freecode");
}

pub mod git;
pub mod scanner;
pub mod llm;
pub mod core;
pub mod memory_search;
pub mod safety_gate;
pub mod api_surface;
pub mod analyzers;
pub mod escalation;
pub mod run_policy;
pub mod t1;

use freecode_pb::freecode_service_server::FreecodeServiceServer;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let addr = "127.0.0.1:50051".parse()?;
    let daemon = core::FreecodeCore::default();

    println!("FreeCode Daemon listening on {}", addr);

    Server::builder()
        .add_service(FreecodeServiceServer::new(daemon))
        .serve(addr)
        .await?;

    Ok(())
}
