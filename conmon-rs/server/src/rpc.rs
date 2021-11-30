use crate::Server;
use capnp::capability::Promise;
use capnp::Error;
use capnp_rpc::pry;
use clap::crate_version;
use conmon_common::conmon_capnp::conmon;
use log::debug;
use std::path::PathBuf;
use tokio::process::Command;

const VERSION: &str = crate_version!();

impl conmon::Server for Server {
    fn version(
        &mut self,
        _: conmon::VersionParams,
        mut results: conmon::VersionResults,
    ) -> Promise<(), capnp::Error> {
        debug!("Got a request");
        results.get().init_response().set_version(VERSION);
        Promise::ok(())
    }

    fn create_container(
        &mut self,
        params: conmon::CreateContainerParams,
        mut results: conmon::CreateContainerResults,
    ) -> Promise<(), capnp::Error> {
        let req = pry!(pry!(params.get()).get_request());
        debug!(
            "Got a create container request for id {}",
            pry!(req.get_id())
        );
        let args = pry!(self.generate_runtime_args(&params));

        let child = pry!(Command::new(self.config().runtime()).args(args).spawn());

        let pid = pry!(child
            .id()
            .ok_or_else(|| Error::failed("Child of container creation had none id".into())));

        let id = pry!(req.get_id()).to_string();
        let exit_paths = pry!(path_vec_from_text_list(pry!(req.get_exit_paths())));

        if let Err(e) = self.reaper().wait_child(id, child, exit_paths) {
            return Promise::err(Error::failed(e.to_string()));
        }

        results.get().init_response().set_container_pid(pid);
        Promise::ok(())
    }
}

fn path_vec_from_text_list(tl: capnp::text_list::Reader) -> Result<Vec<PathBuf>, capnp::Error> {
    let mut v: Vec<PathBuf> = vec![];
    for t in tl {
        let t_str = t?.to_string();
        v.push(PathBuf::from(t_str));
    }
    Ok(v)
}
