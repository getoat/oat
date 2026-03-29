use std::{io::Read, path::Path};

use anyhow::{Context, Result};
use portable_pty::{Child, ChildKiller, CommandBuilder, PtySize, native_pty_system};

pub(crate) struct SpawnedPty {
    pub(crate) child: Box<dyn Child + Send + Sync>,
    pub(crate) killer: Box<dyn ChildKiller + Send + Sync>,
    pub(crate) reader: Box<dyn Read + Send>,
    pub(crate) pid: Option<u32>,
}

pub(crate) fn spawn_pty(script: &str, cwd: &Path) -> Result<SpawnedPty> {
    let pty_system = native_pty_system();
    let pair = pty_system
        .openpty(PtySize {
            rows: 24,
            cols: 80,
            pixel_width: 0,
            pixel_height: 0,
        })
        .context("failed to allocate PTY")?;

    let mut command = CommandBuilder::new("bash");
    command.arg("-lc");
    command.arg(script);
    command.cwd(cwd);

    let child = pair
        .slave
        .spawn_command(command)
        .context("failed to spawn background terminal")?;
    let killer = child.clone_killer();
    let pid = child.process_id();
    drop(pair.slave);
    let reader = pair
        .master
        .try_clone_reader()
        .context("failed to clone PTY reader")?;

    Ok(SpawnedPty {
        child,
        killer,
        reader,
        pid,
    })
}
