use futures_util::stream::SplitSink;
use futures_util::SinkExt as _;
use log::info;
use std::io::{Error, ErrorKind};
use std::time::Instant;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::WebSocketStream;
use v4l::buffer::Type;
use v4l::context::{enum_devices, Node};
use v4l::io::mmap::Stream as MmapStream;
use v4l::io::traits::CaptureStream as _;
use v4l::video::Capture as _;
use v4l::{Device, FourCC};

const MJPEG_FORMAT_FOURCC: FourCC = FourCC { repr: *b"MJPG" };

fn get_usable_devices() -> Vec<Node> {
    let mut device_nodes = enum_devices();
    device_nodes.sort_by_key(Node::index);
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

pub async fn stream_frames(
    mut websocket: SplitSink<WebSocketStream<TcpStream>, Message>,
) -> Result<(), Error> {
    const BUFFER_COUNT: u32 = 30;

    let devices = get_usable_devices();
    if devices.is_empty() {
        return Err(Error::new(
            ErrorKind::NotFound,
            "No valid camera devices found",
        ));
    }

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
        info!("Active format:\n{actual_format}");
        info!("Active parameters:\n{params}");
    }

    // Set up a buffer stream and grab a frame, then print its data
    let mut stream = MmapStream::with_buffers(&dev, Type::VideoCapture, BUFFER_COUNT)?;

    // warmup
    stream.next()?;

    let start = Instant::now();
    let mut i: i32 = 0;
    let mut megabytes_per_sec: f64 = 0.0;
    loop {
        const BYTES_PER_MB: f64 = 1_048_576.0;

        let t0 = Instant::now();
        let (buf, meta) = stream.next()?;
        let duration_secs = t0.elapsed().as_secs_f64();

        let megabytes_in_buf: f64 = f64::from(meta.bytesused) / BYTES_PER_MB;
        let current_megabytes_per_sec = megabytes_in_buf / duration_secs;
        if i == 0 {
            megabytes_per_sec = current_megabytes_per_sec;
        } else {
            // ignore the first measurement
            let prev = megabytes_per_sec * (f64::from(i) / f64::from(i + 1));
            let now = current_megabytes_per_sec * (1.0 / f64::from(i + 1));
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
        println!("  length    : {}", meta.bytesused);
        println!("FPS: {}", f64::from(i) / start.elapsed().as_secs_f64());
        println!("MB/s: {megabytes_per_sec}");
        println!();

        i += 1;
    }
}
