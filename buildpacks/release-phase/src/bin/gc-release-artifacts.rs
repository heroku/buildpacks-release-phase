// Required due to: https://github.com/rust-lang/rust/issues/95513
#![allow(unused_crate_dependencies)]

use std::path::Path;

use release_artifacts::{capture_env, gc};

#[tokio::main]
async fn main() {

    let env = capture_env(Path::new("/etc/heroku"));

    match gc(&env).await {
        Ok(()) => {
            eprintln!("gc-release-artifacts complete.");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("gc-release-artifacts failed: {error:#?}");
            std::process::exit(1);
        }
    }
}
