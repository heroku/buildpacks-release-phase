// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::{collections::HashMap, env, path::Path};

use libcnb::data::exec_d::ExecDProgramOutputKey;
use libcnb::data::exec_d_program_output_key;
use libcnb::exec_d::write_exec_d_program_output;

use release_artifacts::load;

#[tokio::main]
async fn main() {
    let source_dir = Path::new("static-artifacts");

    let mut env = HashMap::new();
    for (key, value) in env::vars() {
        if key.starts_with("STATIC_ARTIFACTS_") || key == "RELEASE_ID" {
            env.insert(key, value);
        }
    }

    match load(&env, source_dir).await {
        Ok(loaded_key) => {
            eprintln!("load-release-artifacts complete.");
            let output_env: HashMap<ExecDProgramOutputKey, String> = HashMap::from([(
                exec_d_program_output_key!("STATIC_ARTIFACTS_LOADED_FROM_KEY"),
                loaded_key,
            )]);
            write_exec_d_program_output(output_env);
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("load-release-artifacts failed: {error:#?}");
            std::process::exit(1);
        }
    }
}
