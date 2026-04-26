//! agent command handlers.
#![allow(unused_imports)]

use crate::*;


pub(crate) async fn cmd_agent(cli: &Cli, cmd: AgentCmd) -> Result<i32> {
    let workdir = resolve_workdir(cli);
    prepare_runtime_hooks(&workdir, cli.quiet);
    agent_serve::run(cmd).await?;
    Ok(EXIT_SUCCESS)
}

