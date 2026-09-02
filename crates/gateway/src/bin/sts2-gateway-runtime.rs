// SPDX-License-Identifier: MIT

#[path = "runtime_support/mod.rs"]
mod runtime_support;

fn main() {
    match runtime_support::RuntimeService::from_environment().and_then(|service| service.run()) {
        Ok(()) => {}
        Err(error) => {
            eprintln!("sts2-gateway runtime failed: {error}");
            std::process::exit(2);
        }
    }
}
