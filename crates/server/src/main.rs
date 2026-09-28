//! The engine as a server, and the interface as a browser tab.
//!
//! Two things over one port: a handful of static files, and a WebSocket carrying
//! commands one way and telemetry the other. There is no HTTP framework and no
//! async runtime, because there is exactly one client and nothing here has to
//! scale — a thread per connection is the whole of it.
//!
//! **Why a separate process rather than an embedded webview.** The audio thread
//! has a deadline measured in milliseconds. Keeping the interface in its own
//! process means nothing it does — a slow layout, a garbage collection, a
//! devtools pause, a crash — can starve or take down the thread making the
//! sound. It also means the page reloads without restarting the engine, which is
//! most of the reason for choosing the web at all: the loop is a save and a
//! refresh rather than a rebuild and a relaunch.
//!
//! Note what is *not* shared with the audio thread. `Link` holds this side of
//! both ring buffers and nothing else; the audio thread owns the other ends
//! inside its callback. So putting the `Link` behind a mutex is free — the audio
//! thread can never contend on it, and could not be made to wait even by a
//! connection thread holding it forever.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use engine::telemetry::{Command, Description, Telemetry};
use host::Link;
use serde::Serialize;
use tungstenite::{Message, accept};

/// Where the browser goes.
const ADDRESS: &str = "127.0.0.1:8730";

/// What the server sends, tagged the same way commands are — so a message in
/// devtools reads as the thing it does.
#[derive(Serialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum Outgoing<'a> {
    /// Once, on connect: what this is and everything about it that will not
    /// change while it plays.
    Hello {
        set: &'a str,
        device: &'a str,
        buffer_frames: u32,
        #[serde(flatten)]
        description: &'a Description,
    },
    /// About sixty times a second. By reference: this enum exists for one line
    /// of serialisation and is never stored, and a telemetry frame is 448 bytes
    /// against the hello's handful of pointers.
    Telemetry(&'a Telemetry),
}

fn main() {
    let set_name = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "intro".to_string());

    // A piece on disk wins over the built-in of the same name. That ordering is
    // the point: once a piece has been written down, editing the file is editing
    // the music, and the Rust that first generated it is only its provenance.
    let path = engine::document::directory().join(format!("{set_name}.json"));
    let set = match engine::document::load(&path) {
        Ok(piece) => {
            println!("{} · {}", piece.name, path.display());
            piece.set
        }
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            match engine::sets::by_name(&set_name) {
                Some(set) => {
                    println!("{set_name} · built in, nothing at {}", path.display());
                    set
                }
                None => {
                    eprintln!(
                        "no piece named {set_name:?}; have {:?}",
                        engine::sets::NAMES
                    );
                    std::process::exit(1);
                }
            }
        }
        Err(err) => {
            eprintln!("could not read {}: {err}", path.display());
            std::process::exit(1);
        }
    };

    let link = match host::start(set) {
        Ok(link) => link,
        Err(err) => {
            eprintln!("could not open the audio device: {err}");
            std::process::exit(1);
        }
    };

    // Built once: it cannot change, since the description was read before the
    // engine was handed over.
    let hello = serde_json::to_string(&Outgoing::Hello {
        set: &set_name,
        device: &link.device,
        buffer_frames: link.buffer_frames,
        description: &link.description,
    })
    .expect("the description serialises");

    println!(
        "{set_name} · {} · {} frames",
        link.device, link.buffer_frames
    );

    let listener = match TcpListener::bind(ADDRESS) {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("could not listen on {ADDRESS}: {err}");
            std::process::exit(1);
        }
    };
    println!("open http://{ADDRESS}");

    let link = Arc::new(Mutex::new(link));
    let hello = Arc::new(hello);
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let (link, hello) = (Arc::clone(&link), Arc::clone(&hello));
        thread::spawn(move || serve(&stream, &link, &hello));
    }
}

fn serve(stream: &TcpStream, link: &Mutex<Link>, hello: &str) {
    let mut peek = [0u8; 2048];
    let Ok(read) = stream.peek(&mut peek) else {
        return;
    };
    let head = String::from_utf8_lossy(&peek[..read]).to_lowercase();

    if head.contains("upgrade: websocket") {
        socket(stream, link, hello);
    } else if let Err(err) = file(stream, &head) {
        eprintln!("could not serve a file: {err}");
    }
}

/// Serve one file. A four-line router, because there are three files.
///
/// Read from disk on every request, deliberately: editing the page needs no
/// rebuild and no restart, only a refresh.
fn file(mut stream: &TcpStream, head: &str) -> std::io::Result<()> {
    // Consume the request before answering it. Closing a socket that still has
    // unread bytes in its receive buffer sends a reset rather than a close, and
    // the client's pending read fails before it ever sees the response — which
    // looks exactly like the server crashing.
    let mut sink = [0u8; 2048];
    let mut seen = Vec::new();
    while !seen.windows(4).any(|w| w == b"\r\n\r\n") && seen.len() < 16_384 {
        let read = stream.read(&mut sink)?;
        if read == 0 {
            break;
        }
        seen.extend_from_slice(&sink[..read]);
    }

    let requested = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");
    let requested = if requested == "/" {
        "/index.html"
    } else {
        requested
    };

    // Only from `web/`, and only a bare file name: nothing to traverse with.
    let name = Path::new(requested).file_name().and_then(|n| n.to_str());
    let (status, body, kind) = match name.map(|n| (n, web_root().join(n))) {
        Some((name, file)) if file.is_file() => {
            let kind = match name.rsplit_once('.').map(|(_, extension)| extension) {
                Some("html") => "text/html; charset=utf-8",
                Some("js") => "text/javascript; charset=utf-8",
                Some("css") => "text/css; charset=utf-8",
                _ => "application/octet-stream",
            };
            ("200 OK", std::fs::read(file)?, kind)
        }
        _ => (
            "404 Not Found",
            b"not found".to_vec(),
            "text/plain; charset=utf-8",
        ),
    };

    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n\
         Cache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(&body)?;
    stream.flush()?;
    // Both halves down, so the client sees a clean close and not a reset.
    let _ = stream.shutdown(std::net::Shutdown::Both);
    Ok(())
}

/// Where the page lives: beside the workspace, so `cargo run` finds it from
/// anywhere.
fn web_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../web")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("web"))
}

fn socket(stream: &TcpStream, link: &Mutex<Link>, hello: &str) {
    let Ok(cloned) = stream.try_clone() else {
        return;
    };
    let Ok(mut socket) = accept(cloned) else {
        return;
    };
    if socket.send(Message::Text(hello.into())).is_err() {
        return;
    }

    // Non-blocking, so reading commands and writing telemetry can share one
    // thread. With one client and a frame every sixteen milliseconds there is
    // nothing to gain from two threads and a lock between them.
    if socket.get_ref().set_nonblocking(true).is_err() {
        return;
    }

    loop {
        match socket.read() {
            Ok(Message::Text(text)) => match serde_json::from_str::<Command>(&text) {
                Ok(command) => link.lock().expect("no panics hold this").send(command),
                Err(err) => eprintln!("could not read {text:?}: {err}"),
            },
            Ok(Message::Close(_)) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(err)) if err.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(_) => return,
        }

        // Only the newest frame is of interest: an old one is of no use to a
        // meter, and the queue is small enough that a slow client simply sees
        // fewer frames rather than falling behind.
        let latest = {
            let mut link = link.lock().expect("no panics hold this");
            let mut latest = None;
            while let Ok(frame) = link.telemetry.pop() {
                latest = Some(frame);
            }
            latest
        };
        match latest {
            Some(frame) => match serde_json::to_string(&Outgoing::Telemetry(&frame)) {
                Ok(json) => {
                    if socket.send(Message::Text(json.into())).is_err() {
                        return;
                    }
                }
                Err(err) => eprintln!("could not write telemetry: {err}"),
            },
            // Nothing new: sleep rather than spin a core.
            None => thread::sleep(Duration::from_millis(4)),
        }
    }
}
