// Copyright 2026 l5y
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Hardware self-test mode: writes hostname + local IPv4 to the LCD and idles.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use std::time::Duration;

use lcd_odroid::{LcdDisplay, info, write_display};

/// Renders host identification (hostname + IPv4) to the LCD and idles.
///
/// Reads the system hostname from `/etc/hostname` and the host's primary IPv4
/// address via [`local_ipv4`], writes a four-line diagnostic frame, and then
/// sleeps so the content stays on the panel. Used to confirm the LCD pipeline
/// (I²C, driver, write_display) works in isolation from any node-RPC plumbing.
pub fn run<L: LcdDisplay>(lcd: &mut L) -> Result<(), Box<dyn std::error::Error>> {
    let hostname = fs::read_to_string("/etc/hostname")?.trim().to_string();
    let ip = match local_ipv4() {
        Ok(v4) => v4.to_string(),
        Err(e) => {
            info!("local_ipv4 failed: {e}");
            "n/a".into()
        }
    };
    info!("hostname={hostname} ip={ip}");
    let lines = [
        format!("{:<20.20}", "lcd-odroid self-test"),
        format!("{:<20.20}", format!("host: {hostname}")),
        format!("{:<20.20}", format!("ip:   {ip}")),
        format!("{:<20.20}", ""),
    ];
    write_display(lcd, &lines)?;
    info!("rendered hostname to LCD; idling");
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}

/// Best-effort lookup of the host's primary IPv4 address.
///
/// Uses the canonical "connect a UDP socket to a routable destination and
/// read its source address" trick. No packets are sent — `connect` just asks
/// the kernel which interface a packet to that destination would leave from.
/// Returns an error on hosts with no IPv4 default route or where the resolved
/// source is not IPv4.
fn local_ipv4() -> Result<Ipv4Addr, Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind("0.0.0.0:0")?;
    socket.connect("1.1.1.1:80")?;
    match socket.local_addr()?.ip() {
        IpAddr::V4(v4) => Ok(v4),
        IpAddr::V6(_) => Err("local addr is not IPv4".into()),
    }
}
