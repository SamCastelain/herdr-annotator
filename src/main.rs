use herdr_annotator::{Engine, Request};
use serde_json::json;
use std::io::{Read, Write};

fn main() {
    let result = (|| -> Result<serde_json::Value, String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if args.first().is_some_and(|s| s == "--version") {
            return Ok(json!({"version": env!("CARGO_PKG_VERSION")}));
        }
        let input = match args.as_slice() {
            [command, input] if command == "request" => input.clone(),
            [command] if command == "request" => {
                let mut text = String::new();
                std::io::stdin()
                    .take(262_145)
                    .read_to_string(&mut text)
                    .map_err(|e| e.to_string())?;
                text
            }
            _ => {
                return Err(
                    "Usage: herdr-annotator request '<JSON>' (or JSON on stdin); --version".into(),
                );
            }
        };
        if input.len() > 262_144 {
            return Err("Request exceeds 256 KiB".into());
        }
        let request: Request = serde_json::from_str(&input).map_err(|e| e.to_string())?;
        Engine::new(Engine::default_state_dir()?).handle(request)
    })();
    let (value, code) = match result {
        Ok(data) => (json!({"ok": true, "data": data}), 0),
        Err(error) => (json!({"ok": false, "error": error}), 1),
    };
    // A closed stdout pipe is a caller going away, not a panic.
    let _ = writeln!(std::io::stdout(), "{value}");
    std::process::exit(code);
}
