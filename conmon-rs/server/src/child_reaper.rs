//! Child process reaping and management.

use anyhow::Result;
use log::{debug, error};
use std::path::PathBuf;
use std::{fs::File, io::Write};
use tokio::process::Child;

impl ChildReaper {
    pub fn wait_child(&self, id: String, mut c: Child, exit_paths: Vec<PathBuf>) {
        tokio::spawn(async move {
            match c.wait().await {
                Ok(status) => {
                    debug!("status for container ID {} is {}", id, status);
                    let code = match status.code() {
                        None => {
                            error!("failed to find code for exited container {}", id);
                            return;
                        }
                        Some(code) => code,
                    };
                    if let Err(e) = write_to_exit_paths(code, exit_paths) {
                        error!("failed to write to exit paths process {}", e);
                    }
                }
                Err(e) => debug!("failed to spawn runtime process {}", e),
            }
        });
    }
}

#[derive(Debug, Default)]
pub struct ChildReaper {}

fn write_to_exit_paths(code: i32, paths: Vec<PathBuf>) -> Result<()> {
    let code_str = format!("{}", code);
    for path in paths {
        File::create(path)?.write_all(code_str.as_bytes())?;
    }
    Ok(())
}
