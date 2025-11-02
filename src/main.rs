use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use log::{info, LevelFilter};
use std::time::Instant;
use std::{env, io::Error};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use v4l::buffer::Type;
use v4l::io::traits::CaptureStream;
use v4l::prelude::MmapStream;
use v4l::video::Capture;
use v4l::{Device, FourCC};

use env_logger::{Builder, Target, WriteStyle};
use v4l::context::{enum_devices, Node};

const MJPEG_FORMAT_FOURCC: FourCC = FourCC { repr: *b"MJPG" };

#[tokio::main]
async fn main() -> Result<(), Error> {
    let mut builder = Builder::from_default_env();
    builder
        .filter_level(LevelFilter::Debug)
        .write_style(WriteStyle::Always)
        .target(Target::Stdout);
    let _ = builder.try_init();
    let addr = env::args()
        .nth(1)
        .unwrap_or_else(|| "127.0.0.1:8080".to_string());

    // Create the event loop and TCP listener we'll accept connections on.
    let try_socket = TcpListener::bind(&addr).await;
    let listener = try_socket.expect("Failed to bind");
    info!("Listening on: {}", addr);

    while let Ok((stream, _)) = listener.accept().await {
        tokio::spawn(handle_connection(stream));
    }

    Ok(())
}

async fn handle_connection(stream: TcpStream) {
    let addr = stream
        .peer_addr()
        .expect("connected streams should have a peer address");
    info!("Peer address: {}", addr);

    let ws_stream = tokio_tungstenite::accept_async(stream)
        .await
        .expect("Error during the websocket handshake occurred");

    info!("New WebSocket connection: {}", addr);

    let (write, mut read) = ws_stream.split();

    let stream_frames = stream_frames(write);
    let read_for_pings = async move {
        loop {
            read.next().await;
        }
    };

    tokio::join!(stream_frames, read_for_pings);
}

fn get_usable_devices() -> Vec<Node> {
    let mut device_nodes = enum_devices();
    device_nodes.sort_by_key(|node_one| node_one.index());
    device_nodes.retain(|node| {
        let device = Device::new(node.index()).unwrap();
        let formats = device.enum_formats().unwrap();
        let has_mjpeg_format: bool = formats
            .iter()
            .any(|format| format.fourcc == MJPEG_FORMAT_FOURCC);

        has_mjpeg_format
    });

    device_nodes
}

async fn stream_frames(
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
) -> Result<(), Error> {
    let devices = get_usable_devices();

    let dev_path = devices.first().unwrap().path();
    info!("Using device: {}\n", dev_path.display());
    let dev = Device::with_path(dev_path).expect("Failed to open device");
    {
        let mut desired_format = dev.format().expect("Failed to read format");
        desired_format.width = 1280;
        desired_format.height = 720;
        desired_format.fourcc = MJPEG_FORMAT_FOURCC;
        let actual_format = dev
            .set_format(&desired_format)
            .expect("Failed to write format");

        let params = dev.params()?;
        info!("Active format:\n{}", actual_format);
        info!("Active parameters:\n{}", params);
    }

    // Set up a buffer stream and grab a frame, then print its data
    const BUFFER_COUNT: u32 = 30;
    let mut stream = MmapStream::with_buffers(&dev, Type::VideoCapture, BUFFER_COUNT)?;

    // warmup
    stream.next()?;

    let start = Instant::now();
    let mut i: i32 = 0;
    let mut megabytes_per_sec: f64 = 0.0;
    loop {
        let t0 = Instant::now();
        let (buf, meta) = stream.next()?;
        let duration_secs = t0.elapsed().as_secs_f64();

        const BYTES_PER_MB: f64 = 1_048_576.0;
        let megabytes_in_buf: f64 = buf.len() as f64 / BYTES_PER_MB;
        let current_megabytes_per_sec = megabytes_in_buf / duration_secs;
        if i == 0 {
            megabytes_per_sec = current_megabytes_per_sec;
        } else {
            // ignore the first measurement
            let prev = megabytes_per_sec * (i as f64 / (i + 1) as f64);
            let now = current_megabytes_per_sec * (1.0 / (i + 1) as f64);
            megabytes_per_sec = prev + now;
        }

        let new_buf = buf.to_vec();
        let msg = Message::binary(new_buf);

        websocket.send(msg).await.unwrap();

        // let result = websocket.send(msg);
        // match result {
        //     Ok(v) => v,
        //     Err(e) => {
        //         println!("Error sending message: {:?}", e);
        //         break Ok(()); // FIXME: pass back this error?
        //     }
        // }

        println!("Buffer");
        println!("  sequence  : {}", meta.sequence);
        println!("  timestamp : {}", meta.timestamp);
        println!("  flags     : {}", meta.flags);
        println!("  length    : {}", buf.len());
        println!("FPS: {}", i as f64 / start.elapsed().as_secs_f64());
        println!("MB/s: {}", megabytes_per_sec);
        println!();

        i += 1;
    }
}
