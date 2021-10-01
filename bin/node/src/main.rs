mod chain_spec;
#[macro_use]
pub(crate) mod service;
mod cli;
mod command;
mod commands;
mod rpc;

fn main() -> sc_cli::Result<()> {
    command::run()
}
