use std::{io::Read, path::Path};
fn main() {
    if loramesh::version_requested("loramesh-mesh-sim") {
        return;
    }
    if let Err(e) = run() {
        eprintln!("mesh simulation: {}", e);
        std::process::exit(1);
    }
}
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.first().map(|a| a == "--help").unwrap_or(false) {
        println!("Usage: loramesh-mesh-sim CASE.json [--replay] [--output FILE]");
        return Ok(());
    }
    if args.is_empty() {
        return Err("Usage: loramesh-mesh-sim CASE.json [--replay] [--output FILE]".into());
    }
    let mut data = Vec::new();
    std::fs::File::open(&args[0])?
        .take(4 * 1024 * 1024 + 1)
        .read_to_end(&mut data)?;
    if data.len() > 4 * 1024 * 1024 {
        return Err("case too large".into());
    }
    let case: loramesh::mesh::simulation::Case = if args.iter().any(|a| a == "--replay") {
        let report: serde_json::Value = serde_json::from_slice(&data)?;
        serde_json::from_value(
            report
                .get("configuration")
                .ok_or("missing configuration")?
                .clone(),
        )?
    } else {
        serde_json::from_slice(&data)?
    };
    let report = loramesh::mesh::simulation::run(&case);
    let failed = report
        .as_ref()
        .map(|r| !r.failures.is_empty())
        .unwrap_or(true);
    let value = match report {
        Ok(r) => serde_json::to_value(r)?,
        Err(e) => serde_json::json!({"configuration":case,"error":e.to_string()}),
    };
    let output = serde_json::to_vec_pretty(&value)?;
    if let Some(i) = args.iter().position(|a| a == "--output") {
        std::fs::write(
            Path::new(args.get(i + 1).ok_or("missing output path")?),
            output,
        )?;
    } else {
        println!("{}", String::from_utf8(output)?);
    }
    if failed {
        return Err("scenario expectations failed; replay configuration is in the report".into());
    }
    Ok(())
}
