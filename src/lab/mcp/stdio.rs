//! JSON-RPC MCP stdio transport (newline-delimited).
//! Used by: `mint lab mcp-stdio`. Logs go to stderr; messages on stdout.

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use crate::lab::error::LabResult;
use crate::lab::mcp::{handle_rpc, JsonRpcRequest, McpCallContext, McpState};

pub async fn serve_stdio(state: McpState) -> LabResult<()> {
    let stdin = BufReader::new(tokio::io::stdin());
    let mut lines = stdin.lines();
    let mut stdout = tokio::io::stdout();
    let ctx = McpCallContext { authorized: true };
    eprintln!(
        "mint-lab-pa-bv stdio MCP run_id={} task_id={} apply={}",
        state.run_id, state.task_id, state.apply
    );
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let req: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(err) => {
                let resp = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": { "code": -32700, "message": format!("parse error: {err}") }
                });
                stdout
                    .write_all(format!("{resp}\n").as_bytes())
                    .await
                    .map_err(|e| crate::lab::error::LabError::Io(e.to_string()))?;
                stdout.flush().await?;
                continue;
            }
        };
        let resp = handle_rpc(&state, req, &ctx);
        let encoded = serde_json::to_string(&resp)?;
        stdout.write_all(encoded.as_bytes()).await?;
        stdout.write_all(b"\n").await?;
        stdout.flush().await?;
    }
    Ok(())
}
