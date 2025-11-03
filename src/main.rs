mod camera;

use futures_util::StreamExt as _;
use log::{info, log_enabled, Level, LevelFilter};
use std::{env, io::Error};
use tokio::net::{TcpListener, TcpStream};

use env_logger::{Builder, Target, WriteStyle};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::http::StatusCode;

const VIDEO_ENDPOINT: &str = "/video";
const ADDR: &str = "127.0.0.1:8080";

#[tokio::main]
async fn main() -> Result<(), Error> {
    _ = Builder::from_default_env()
        .filter_level(LevelFilter::Debug)
        .write_style(WriteStyle::Always)
        .target(Target::Stdout)
        .try_init();

    let addr = env::args().nth(1).unwrap_or_else(|| ADDR.to_owned());

    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind(&addr).await;
    let listener = try_socket.expect("Failed to bind");
    info!("Listening on: {addr}");

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_connection(stream));
    }

    Ok(())
}

//noinspection RsUnreachableCode
async fn handle_connection(stream: TcpStream) {
    if log_enabled!(Level::Info) {
        let addr = stream
            .peer_addr()
            .expect("connected streams should have a peer address");
        info!("Peer address: {addr}");
    }

    let header_callback = |request: &Request, response: Response| {
        let uri = request.uri();
        info!("URI: {uri}");

        if !uri.path().starts_with(VIDEO_ENDPOINT) {
            let mut error_response = ErrorResponse::default();
            *error_response.status_mut() = StatusCode::NOT_FOUND;
            return Err(error_response);
        }

        Ok(response)
    };
    let ws_stream = tokio_tungstenite::accept_hdr_async(stream, header_callback)
        .await
        .expect("Error during the websocket handshake occurred");

    info!("Peer has initiated a new WebSocket connection");

    let (write, mut read) = ws_stream.split();

    let stream_frames = camera::stream_frames(write);
    let read_for_pings = async move {
        loop {
            read.next().await;
        }

        #[expect(
            unreachable_code,
            reason = "Returning `Result` here to make it infer the type as `Result` so it can be `try_join`'d"
        )]
        Ok(())
    };

    tokio::try_join!(stream_frames, read_for_pings).unwrap();
}
