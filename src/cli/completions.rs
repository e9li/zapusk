use anyhow::Result;
use clap::Command;
use clap_complete::{Shell, generate};
use std::io;

pub fn run(shell: Shell, mut cmd: Command) -> Result<()> {
    generate(shell, &mut cmd, "zapusk", &mut io::stdout());
    Ok(())
}
