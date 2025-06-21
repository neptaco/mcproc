//! Version command implementation

use crate::common::version::VERSION;
use clap::Args;

#[derive(Debug, Args)]
pub struct VersionCommand {}

impl VersionCommand {
    pub async fn execute(self) -> Result<(), Box<dyn std::error::Error>> {
        println!("mcproc {}", VERSION);
        Ok(())
    }
}