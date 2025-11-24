Small project to stream a local camera device as mjpeg frames (JPEG images) via Websocket.

The goal is to stream the frames in a simple and efficient way, without creating a bunch of CPU overhead or latency.

Uses Video4Linux API, so this only works on Linux.