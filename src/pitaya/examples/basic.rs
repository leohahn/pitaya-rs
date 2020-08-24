use pitaya::{Context, EtcdLazy, NatsRpcClient, NatsRpcServer};
use slog::{error, info, o, Drain};
use tokio::sync::watch;

async fn send_rpc(
    mut pitaya_server: pitaya::Pitaya<EtcdLazy, NatsRpcServer, NatsRpcClient>,
    msg: Vec<u8>,
    rx: watch::Receiver<bool>,
) {
    loop {
        if *rx.borrow() {
            // Received signal to quit.
            break;
        }

        let msg = msg.clone();

        if let Err(e) = pitaya_server
            .send_rpc(
                Context::new(),
                // "csharp.testRemote.remote",
                "SuperKind.testRemote.remote",
                msg,
            )
            .await
        {
            println!("RPC FAILED: {}", e);
        }
    }
}

fn init_logger() -> slog::Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_async::Async::new(drain)
        .chan_size(1000)
        .build()
        .filter_level(slog::Level::Info)
        .fuse();
    slog::Logger::root(drain, o!())
}

#[derive(Clone, PartialEq, ::prost::Message)]
pub struct RpcMsg {
    #[prost(string, tag = "1")]
    pub route: std::string::String,
    #[prost(string, tag = "2")]
    pub msg: std::string::String,
}

#[tokio::main]
async fn main() {
    let root_logger = init_logger();
    let logger = root_logger.clone();

    let (pitaya_server, shutdown_receiver) = pitaya::PitayaBuilder::new()
        .with_env_prefix("MY_ENV")
        .with_config_file("examples/config/production.yaml")
        .with_logger(root_logger)
        .with_rpc_handler({
            let logger = logger.clone();
            Box::new(move |_ctx, rpc| {
                let res = pitaya::protos::Response {
                    data: "HEY, THIS IS THE SERVER".as_bytes().to_owned(),
                    error: None,
                };
                if !rpc.respond(res) {
                    error!(logger, "failed to respond to the server");
                }
            })
        })
        .with_cluster_subscriber({
            let logger = logger.clone();
            move |notification| match notification {
                pitaya::cluster::Notification::ServerAdded(server) => {
                    info!(logger, "[subscriber] server added"; "server" => ?server);
                }
                pitaya::cluster::Notification::ServerRemoved(server_id) => {
                    info!(logger, "[subscriber] server removed"; "server_id" => ?server_id);
                }
            }
        })
        .build()
        .await
        .expect("failed to start pitaya server");

    info!(logger, "sending rpc");

    let (tx, rx) = watch::channel(false);

    let msg = RpcMsg::default();
    let msg_data = pitaya::utils::encode_proto(&msg);

    const NUM_CONCURRENT_TASKS: usize = 50;

    let mut tasks = Vec::with_capacity(NUM_CONCURRENT_TASKS);
    println!("spawning tasks...");
    for _ in 0..NUM_CONCURRENT_TASKS {
        let task = tokio::spawn({
            let pitaya_server = pitaya_server.clone();
            let msg_data = msg_data.clone();
            let rx = rx.clone();
            async move {
                send_rpc(pitaya_server.clone(), msg_data.clone(), rx).await;
            }
        });
        tasks.push(task);
    }

    println!("done spawning tasks.");

    println!("all requests finished!");

    // info!(
    //     logger,
    //     "received response: {:?}",
    //     String::from_utf8_lossy(&res.data)
    // );

    println!("waiting");
    shutdown_receiver
        .await
        .expect("failed to wait for shutdown receiver");
    println!("done waiting");

    println!("broadcasting");
    tx.broadcast(true).unwrap();
    println!("done broadcasting");

    futures::future::join_all(tasks).await;

    std::thread::sleep(std::time::Duration::from_secs(1));

    if let Err(e) = pitaya_server.shutdown().await {
        error!(logger, "failed to shutdown pitaya: {}", e);
    }
}
