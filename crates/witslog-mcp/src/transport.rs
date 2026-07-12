use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};

pub struct JsonRpcTransport {
    reader: BufReader<std::io::Stdin>,
    writer: std::io::Stdout,
}

impl JsonRpcTransport {
    pub fn new() -> Self {
        JsonRpcTransport {
            reader: BufReader::new(std::io::stdin()),
            writer: std::io::stdout(),
        }
    }

    /// Read a JSON-RPC 2.0 request from stdin (line-delimited JSON).
    /// Returns `Ok(None)` at EOF (stdin closed). Blank lines are skipped.
    pub fn read_request(&mut self) -> std::io::Result<Option<Value>> {
        loop {
            let mut line = String::new();
            let n = self.reader.read_line(&mut line)?;
            if n == 0 {
                return Ok(None);
            }
            if line.trim().is_empty() {
                continue;
            }
            return Ok(Some(serde_json::from_str(&line)?));
        }
    }

    /// Write a JSON-RPC 2.0 response to stdout.
    pub fn write_response(&mut self, response: Value) -> std::io::Result<()> {
        self.writer.write_all(serde_json::to_string(&response)?.as_bytes())?;
        self.writer.write_all(b"\n")?;
        self.writer.flush()?;
        Ok(())
    }

    /// Helper: build a JSON-RPC error response.
    pub fn error_response(id: Option<Value>, code: i32, message: &str, data: Option<Value>) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {
                "code": code,
                "message": message,
                "data": data
            }
        })
    }

    /// Helper: build a JSON-RPC success response.
    pub fn success_response(id: Option<Value>, result: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })
    }
}
