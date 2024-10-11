// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::{collections::HashMap, env, path::Path};

use release_artifacts::upload;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("upload-release-artifacts requires argument: the source directory");
        std::process::exit(1);
    }
    let source_dir = Path::new(&args[1]);

    let mut env = HashMap::new();
    for (key, value) in env::vars() {
        if key.starts_with("STATIC_ARTIFACTS_") || key == "RELEASE_ID" {
            env.insert(key, value);
        }
    }

    match upload(&env, source_dir).await {
        Ok(()) => {
            eprintln!("upload-release-artifacts complete.");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("upload-release-artifacts failed: {error:#?}");
            std::process::exit(1);
        }
    }
}
