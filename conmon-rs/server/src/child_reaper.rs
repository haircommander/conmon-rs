//! Child process reaping and management.

#![allow(dead_code)] // TODO: remove me when actually used

use anyhow::{bail, Result};
use getset::Getters;
use log::{debug, error};
use nix::{
    sys::wait::{waitpid, WaitPidFlag, WaitStatus},
    unistd::Pid,
};
use std::path::PathBuf;
use std::{collections::HashMap, fs::File, io::Write, sync::Arc, sync::RwLock};
use tokio::process::Child;

impl ChildReaper {
    pub fn start(&self) -> Result<()> {
        Ok(self.wait_for_children())
    }
    fn wait_for_children(&self) {
        let locked_children = self.children.clone();
        tokio::spawn(async move {
            loop {
                debug!("looping");
                let (pid, ec) = match waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG)) {
                    Ok(WaitStatus::StillAlive) => {
                        continue;
                    }
                    Ok(WaitStatus::Exited(pid, ec)) => (pid.as_raw(), ec),
                    Ok(status) => {
                        debug!("unexpected wait status {:?}", status);
                        continue;
                    }
                    Err(e) => {
                        panic!("Error waiting for PIDs {}", e);
                    }
                };
                let children = match locked_children.write() {
                    Ok(c) => c,
                    Err(e) => {
                        panic!("Error unlocking children {}", e);
                    }
                };
                let child = match children.get(&pid) {
                    None => {
                        continue;
                    }
                    Some(c) => c,
                };

                debug!("exit code for container ID {} is {}", child.id(), ec);
                if let Err(e) = write_to_exit_paths(ec, &child.exit_paths) {
                    error!("failed to write to exit paths process {}", e);
                }
            }
        });
    }

    pub fn wait_child(&self, id: String, c: Child, exit_paths: Vec<PathBuf>) -> Result<()> {
        let mut map = self.children.write().expect("Children map defunct");
        let pid: i32 = c
            .id()
            .ok_or_else(|| capnp::Error::failed(format!("child PID for container {} died", id)))?
            .try_into()?;

        let reapable_child = ReapableChild {
            id: id,
            child: c,
            exit_paths: exit_paths,
        };
        if let Some(old) = map.insert(pid, reapable_child) {
            bail!("Repeat PID for container {} found", old.id);
        }
        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct ChildReaper {
    children: Arc<RwLock<HashMap<i32, ReapableChild>>>,
}

#[derive(Debug, Getters)]
pub struct ReapableChild {
    #[getset(get)]
    id: String,
    #[getset(get)]
    child: Child,
    #[getset(get)]
    exit_paths: Vec<PathBuf>,
}

impl ReapableChild {
    //    fn wait(self) {
    //        tokio::spawn(async move {
    //            match self.child.wait().await {
    //                Ok(status) => {
    //                    debug!("status for container ID {} is {}", self.id, status);
    //                    let code = match status.code() {
    //                        None => {
    //                            error!("failed to find code for exited container {}", self.id);
    //                            return;
    //                        }
    //                        Some(code) => code,
    //                    };
    //                    if let Err(e) = write_to_exit_paths(code, self.exit_paths) {
    //                        error!("failed to write to exit paths process {}", e);
    //                    }
    //                }
    //                Err(e) => debug!("failed to spawn runtime process {}", e),
    //            }
    //        });
    //    }
}

fn write_to_exit_paths(code: i32, paths: &Vec<PathBuf>) -> Result<()> {
    let code_str = format!("{}", code);
    for path in paths {
        File::create(path)?.write_all(code_str.as_bytes())?;
    }
    Ok(())
}
