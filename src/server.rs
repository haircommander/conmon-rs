use anyhow::{Context, Error, Result};
use async_stream::stream;
use clap::crate_version;
use conmon::{
    conmon_server::{Conmon, ConmonServer},
    VersionRequest, VersionResponse,
};
use futures::TryFutureExt;
use getset::{Getters, MutGetters};
use log::{debug, info};
use nix::{
    libc::_exit,
    unistd::{fork, ForkResult},
};
use std::{env, path::PathBuf};
use stream::Stream;
use tokio::{
    fs,
    net::UnixListener,
    runtime::Builder,
    signal::unix::{signal, SignalKind},
    sync::oneshot,
};
use tonic::{transport::Server, Request, Response, Status};

mod config;
mod init;
mod stream;

const VERSION: &str = crate_version!();

pub mod conmon {
    tonic::include_proto!("conmon");
}

#[derive(Debug, Default, Getters, MutGetters)]
pub struct ConmonServerImpl {
    #[doc = "The main conmon configuration."]
    #[getset(get, get_mut)]
    config: config::Config,
}

impl ConmonServerImpl {
    /// Create a new ConmonServerImpl instance.
    pub fn new() -> Result<Self> {
        let server = Self::default();
        server.init_logging().context("set log verbosity")?;
        server.config().validate().context("validate config")?;

        server.init_self()?;
        Ok(server)
    }

    fn init_self(&self) -> Result<(), Error> {
        init::unset_locale();
        // While we could configure this, standard practice has it as -1000,
        // so it may be YAGNI to add configuration.
        init::set_oom("-1000")?;
        Ok(())
    }

    fn init_logging(&self) -> Result<()> {
        if let Some(level) = self.config().log_level().to_level() {
            simple_logger::init_with_level(level).context("init logger")?;
            info!("Set log level to {}", level);
        }
        Ok(())
    }
}

#[tonic::async_trait]
impl Conmon for ConmonServerImpl {
    async fn version(
        &self,
        request: Request<VersionRequest>,
    ) -> Result<Response<VersionResponse>, Status> {
        info!("Got a request: {:?}", request);

        let res = VersionResponse {
            version: String::from(VERSION),
        };

        Ok(Response::new(res))
    }
}

fn main() -> Result<(), Error> {
    // First, initialize the server so we have access to the config pre-fork.
    let server = ConmonServerImpl::new()?;

    // We need to fork as early as possible, especially before setting up tokio.
    // If we don't, the child will have a strange thread space and we're at risk of deadlocking.
    // We also have to treat the parent as the child (as described in [1]) to ensure we don't
    // interrupt the child's execution.
    // 1: https://docs.rs/nix/0.23.0/nix/unistd/fn.fork.html#safety
    match unsafe { fork() } {
        Ok(ForkResult::Parent { child: _, .. }) => {
            unsafe { _exit(0) };
        }
        Ok(ForkResult::Child) => (),
        Err(e) => panic!("Fork failed {}", e),
    }
    // Use the single threaded runtime to save rss memory.
    let rt = Builder::new_current_thread().enable_io().build()?;
    rt.block_on(start_server(server))?;
    Ok(())
}

async fn start_server(server: ConmonServerImpl) -> Result<(), Error> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();

    let socket = server.config().socket().clone();
    let sigterm_handler = tokio::spawn(start_sigterm_handler(socket, shutdown_tx));
    let grpc_backend = tokio::spawn(start_grpc_backend(server, shutdown_rx));

    let _ = tokio::join!(sigterm_handler, grpc_backend);
    Ok(())
}

async fn start_sigterm_handler(socket: PathBuf, shutdown_tx: oneshot::Sender<()>) -> Result<()> {
    let mut sigterm = signal(SignalKind::terminate())?;
    let mut sigint = signal(SignalKind::interrupt())?;

    tokio::select! {
        _ = sigterm.recv() => {
            info!("Received SIGTERM");
        }
        _ = sigint.recv() => {
            info!("Received SIGINT");
        }
    };

    let _ = shutdown_tx.send(());
    debug!("Removing socket file {}", socket.display());
    fs::remove_file(socket)
        .await
        .context("remove existing socket file")?;
    Ok(())
}

async fn start_grpc_backend(
    server: ConmonServerImpl,
    shutdown_rx: oneshot::Receiver<()>,
) -> Result<(), Error> {
    let incoming = {
        let uds = UnixListener::bind(server.config().socket()).context("bind server socket")?;
        stream! {
            loop {
                let item = uds.accept().map_ok(|(st, _)| Stream(st)).await;
                yield item;
            }
        }
    };

    Server::builder()
        .add_service(ConmonServer::new(server))
        .serve_with_incoming_shutdown(incoming, async move {
            let _ = shutdown_rx.await.ok();
            info!("Gracefully shutting down grpc backend")
        })
        .await?;
    Ok(())
}
