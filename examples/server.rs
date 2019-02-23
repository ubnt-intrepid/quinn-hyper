use slog::Drain;
use tokio::prelude::*;
use tokio::runtime::current_thread::Runtime;

fn main() -> failure::Fallible<()> {
    let logger = {
        let decorator = slog_term::PlainSyncDecorator::new(std::io::stderr());
        let drain = slog_term::FullFormat::new(decorator)
            .use_original_order()
            .build()
            .fuse();
        slog::Logger::root(drain, slog::o!())
    };

    let server_config = {
        let mut server_config = quinn::ServerConfigBuilder::default();
        server_config.set_protocols(&[quinn::ALPN_QUIC_HTTP]);
        server_config.build()
    };

    let mut endpoint = quinn::Endpoint::new();
    endpoint.listen(server_config);
    let (_, driver, incoming) = endpoint.bind("127.0.0.1:4000")?;

    let mut runtime = Runtime::new()?;
    runtime.spawn(incoming.for_each(move |conn| {
        handle_connection(&logger, conn);
        Ok(())
    }));
    runtime.block_on(driver)?;
    Ok(())
}

fn handle_connection(logger: &slog::Logger, conn: quinn::NewConnection) {
    let quinn::NewConnection {
        incoming,
        connection,
    } = conn;

    slog::info!(logger, "got connection";
        "remote_id" => %connection.remote_id(),
        "address"   => &connection.remote_address(),
        "protocol"  => connection.protocol().map_or_else(
            || "<none>".into(),
            |x| String::from_utf8_lossy(&x).into_owned()
        )
    );

    tokio::runtime::current_thread::spawn(
        incoming
            .map_err({
                let logger = logger.clone();
                move |e| slog::error!(logger, "connection terminated, reason: {}", e)
            })
            .for_each({
                let logger = logger.clone();
                move |stream| {
                    handle_stream(&logger, stream);
                    Ok(())
                }
            }),
    );
}

fn handle_stream(logger: &slog::Logger, stream: quinn::NewStream) {
    let stream = match stream {
        quinn::NewStream::Bi(stream) => stream,
        quinn::NewStream::Uni(..) => {
            unreachable!("config.max_remote_uni_streams is defaulted to 0")
        }
    };

    tokio::runtime::current_thread::spawn(
        hyper::server::conn::Http::new()
            .with_executor(tokio::runtime::current_thread::TaskExecutor::current())
            .serve_connection(
                stream,
                hyper::service::service_fn_ok(|_req| {
                    hyper::Response::builder()
                        .header("content-type", "text/plain")
                        .body(hyper::Body::from("Hello"))
                        .unwrap()
                }),
            )
            .map_err({
                let logger = logger.clone();
                move |e| slog::error!(logger, "connection error: {}", e)
            }),
    );
}
