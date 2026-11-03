// SPDX-License-Identifier: MIT
//! Bounded two-pointer transport for an admitted, inspected input-preview test.
//! This grants no custody and performs no ADB discovery. The caller owns the
//! exact scrcpy 5.0 server/loopback forward and current focus/geometry checks.
//! Protocol: Genymobile/scrcpy v5.0 ControlMessageReader.parseInjectTouchEvent.
//! https://github.com/Genymobile/scrcpy/blob/v5.0/server/src/main/java/com/genymobile/scrcpy/control/ControlMessageReader.java
use std::{
    io::{self, BufRead, Read, Write},
    net::{Ipv4Addr, Shutdown, SocketAddrV4, TcpStream},
    sync::mpsc,
    time::Duration,
};

#[derive(Clone, Copy)]
struct Point {
    x: u32,
    y: u32,
}
fn message(action: u8, pointer: u64, p: Point, w: u16, h: u16) -> io::Result<[u8; 32]> {
    if !matches!(action, 0..=3)
        || !(2..=8192).contains(&w)
        || !(2..=8192).contains(&h)
        || p.x >= u32::from(w)
        || p.y >= u32::from(h)
    {
        return Err(io::Error::other("Pointer outside the admitted frame"));
    }
    let mut b = [0; 32];
    b[0] = 2;
    b[1] = action;
    b[2..10].copy_from_slice(&pointer.to_be_bytes());
    b[10..14].copy_from_slice(&p.x.to_be_bytes());
    b[14..18].copy_from_slice(&p.y.to_be_bytes());
    b[18..20].copy_from_slice(&w.to_be_bytes());
    b[20..22].copy_from_slice(&h.to_be_bytes());
    b[22..24].copy_from_slice(
        &(if action == 1 || action == 3 {
            0_u16
        } else {
            u16::MAX
        })
        .to_be_bytes(),
    );
    Ok(b)
}
struct Contacts {
    socket: TcpStream,
    points: [Point; 2],
    size: (u16, u16),
    active: [bool; 2],
    healthy: bool,
}
impl Contacts {
    fn send(&mut self, index: usize, action: u8) -> io::Result<()> {
        let bytes = message(
            action,
            10 + index as u64,
            self.points[index],
            self.size.0,
            self.size.1,
        )?;
        if !self.healthy {
            return Err(io::Error::other("Uncertain stream; never replay input"));
        }
        if let Err(error) = self.socket.write_all(&bytes) {
            // A partial frame cannot safely be followed by another message.
            self.healthy = false;
            let _ = self.socket.shutdown(Shutdown::Both);
            return Err(error);
        }
        Ok(())
    }
    fn down(&mut self) -> io::Result<()> {
        if self.active.iter().any(|p| *p) {
            return Err(io::Error::other("Contacts already held"));
        }
        self.send(0, 0)?;
        self.active[0] = true;
        self.send(1, 0)?;
        self.active[1] = true;
        Ok(())
    }
    fn release_second(&mut self) -> io::Result<()> {
        if self.active != [true, true] {
            return Err(io::Error::other("Expected both contacts"));
        }
        self.send(1, 1)?;
        self.active[1] = false;
        Ok(())
    }
    fn cancel(&mut self) -> io::Result<()> {
        if !self.active.iter().any(|p| *p) {
            return Ok(());
        }
        self.send(0, 3)?;
        // scrcpy's pointer table removes entries on UP, not CANCEL. Retire its
        // bookkeeping too; no new DOWN is sent after cancellation.
        for i in [1, 0] {
            if self.active[i] {
                self.send(i, 1)?;
                self.active[i] = false;
            }
        }
        Ok(())
    }
}
impl Drop for Contacts {
    fn drop(&mut self) {
        if self.healthy {
            let _ = self.cancel();
        }
        let _ = self.socket.shutdown(Shutdown::Both);
    }
}
fn connect(port: u16) -> io::Result<TcpStream> {
    let mut socket = TcpStream::connect_timeout(
        &SocketAddrV4::new(Ipv4Addr::LOCALHOST, port).into(),
        Duration::from_secs(2),
    )?;
    socket.set_write_timeout(Some(Duration::from_secs(1)))?;
    socket.set_read_timeout(Some(Duration::from_secs(2)))?;
    socket.set_nodelay(true)?;
    let mut hello = [0_u8; 1];
    socket.read_exact(&mut hello)?;
    if hello != [0] {
        return Err(io::Error::other(
            "Expected pinned server dummy-byte handshake; no input sent",
        ));
    }
    Ok(socket)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() != 7 {
        return Err("usage: ntx-motion-probe PORT WIDTH HEIGHT X1 Y1 X2 Y2; commands: down, release, cancel, close".into());
    }
    let fd = std::env::var("CODEX_ANDROID_LEASE_FD")?.parse::<u32>()?;
    if !std::fs::metadata(format!("/proc/self/fd/{fd}"))?.is_file() {
        return Err("Actual inherited device lease FD required".into());
    }
    let port = args[0].parse::<u16>()?;
    if port == 0 {
        return Err("Owned loopback port required".into());
    }
    let size = (args[1].parse::<u16>()?, args[2].parse::<u16>()?);
    let points = [
        Point {
            x: args[3].parse()?,
            y: args[4].parse()?,
        },
        Point {
            x: args[5].parse()?,
            y: args[6].parse()?,
        },
    ];
    for (i, p) in points.iter().enumerate() {
        message(0, 10 + i as u64, *p, size.0, size.1)?;
    }
    if points[0].x == points[1].x && points[0].y == points[1].y {
        return Err("Two distinct inspected targets required".into());
    }
    let socket = connect(port)?;
    let mut contacts = Contacts {
        socket,
        points,
        size,
        active: [false; 2],
        healthy: true,
    };
    let (tx, rx) = mpsc::sync_channel(8);
    std::thread::spawn(move || {
        for line in io::stdin().lock().lines().take(8) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    println!("ready; no input sent");
    io::stdout().flush()?;
    for _ in 0..8 {
        let line = rx.recv_timeout(Duration::from_secs(8))??;
        match line.trim() {
            "down" => {
                contacts.down()?;
                println!("sent both; observation required");
            }
            "release" => {
                contacts.release_second()?;
                println!("sent second release; observation required");
            }
            "cancel" => {
                contacts.cancel()?;
                println!("sent cancel; observation required");
            }
            "close" => {
                contacts.cancel()?;
                println!("closed");
                return Ok(());
            }
            _ => return Err("Unknown input command; contacts cancelled when possible".into()),
        }
        io::stdout().flush()?;
    }
    Err("Finite command limit reached".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn second_release_keeps_the_first_contact_and_drop_cancels_the_remainder() {
        use std::{io::Read, net::TcpListener};
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let capture = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).unwrap();
            bytes
        });
        let socket = TcpStream::connect(address).unwrap();
        let mut contacts = Contacts {
            socket,
            points: [Point { x: 10, y: 10 }, Point { x: 80, y: 80 }],
            size: (100, 100),
            active: [false; 2],
            healthy: true,
        };
        contacts.down().unwrap();
        assert!(contacts.down().is_err());
        contacts.release_second().unwrap();
        assert_eq!(contacts.active, [true, false]);
        drop(contacts);
        let bytes = capture.join().unwrap();
        assert_eq!(bytes.len(), 5 * 32);
        let sequence = bytes
            .chunks_exact(32)
            .map(|frame| {
                (
                    frame[1],
                    u64::from_be_bytes(frame[2..10].try_into().unwrap()),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(sequence, [(0, 10), (0, 11), (1, 11), (3, 10), (1, 10)]);
    }
    #[test]
    fn wrong_server_handshake_sends_no_input() {
        use std::net::TcpListener;
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let capture = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            socket.write_all(&[1]).unwrap();
            let mut bytes = Vec::new();
            socket.read_to_end(&mut bytes).unwrap();
            bytes
        });
        assert!(connect(port).is_err());
        assert!(capture.join().unwrap().is_empty());
    }
    #[test]
    fn pinned_touch_wire_shape_and_bounds() {
        let b = message(0, 10, Point { x: 111, y: 222 }, 2280, 1080).unwrap();
        assert_eq!(&b[..10], &[2, 0, 0, 0, 0, 0, 0, 0, 0, 10]);
        assert_eq!(u32::from_be_bytes(b[10..14].try_into().unwrap()), 111);
        assert_eq!(u32::from_be_bytes(b[14..18].try_into().unwrap()), 222);
        assert_eq!(u16::from_be_bytes(b[18..20].try_into().unwrap()), 2280);
        assert_eq!(u16::from_be_bytes(b[20..22].try_into().unwrap()), 1080);
        assert_eq!(&b[22..], &[255, 255, 0, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            &message(3, 10, Point { x: 1, y: 1 }, 100, 100).unwrap()[22..24],
            &[0, 0]
        );
        assert!(message(0, 10, Point { x: 100, y: 1 }, 100, 100).is_err());
        assert!(message(5, 10, Point { x: 1, y: 1 }, 100, 100).is_err());
    }
}
