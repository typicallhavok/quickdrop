use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;
use tokio::time::{interval, Duration, Instant};

use crate::protocol::UDP_DISCOVERY_PORT;
const QUICKDROP_PREFIX: &str = "QUICKDROP_DISCOVER:";

#[derive(Clone, Debug)]
pub struct UdpDevice {
    pub id: String,
    pub name: String,
    pub ip: String,
    pub last_seen: Instant,
}

pub struct UdpDiscovery {
    devices: Arc<Mutex<HashMap<String, UdpDevice>>>,
}

impl UdpDiscovery {
    pub fn new() -> Self {
        Self {
            devices: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn start(&self, _my_id: String, my_name: Arc<Mutex<String>>) {
        let devices = self.devices.clone();
        
        tokio::spawn(async move {
            let socket = UdpSocket::bind(SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, UDP_DISCOVERY_PORT))
                .await
                .expect("Failed to bind UDP discovery port");
            socket.set_broadcast(true).expect("Failed to set broadcast");

            let socket = Arc::new(socket);

            // Listener loop
            let listener_socket = socket.clone();
            let devices_listen = devices.clone();
            let my_name_listen = my_name.clone();
            
            tokio::spawn(async move {
                let mut buf = [0u8; 1024];
                loop {
                    if let Ok((len, addr)) = listener_socket.recv_from(&mut buf).await {
                        if let Ok(msg) = std::str::from_utf8(&buf[..len]) {
                            let ip_str = addr.ip().to_string();

                            if let Some(payload) = msg.strip_prefix(QUICKDROP_PREFIX) {
                                let name = if let Some(colon_pos) = payload.rfind(':') {
                                    let maybe_port = &payload[colon_pos + 1..];
                                    if maybe_port.parse::<u16>().is_ok() {
                                        &payload[..colon_pos]
                                    } else {
                                        payload
                                    }
                                } else {
                                    payload
                                };

                                let my_current_name = my_name_listen.lock().await.clone();
                                let mut is_own_ip = false;
                                if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
                                    for (_, local_ip) in interfaces {
                                        if local_ip.to_string() == ip_str {
                                            is_own_ip = true;
                                            break;
                                        }
                                    }
                                }
                                
                                if name == my_current_name || ip_str == "127.0.0.1" || ip_str == "0.0.0.0" || is_own_ip {
                                    continue;
                                }

                                // Send unicast reply
                                let reply = format!("{}{}", QUICKDROP_PREFIX, my_current_name);
                                let _ = listener_socket.send_to(reply.as_bytes(), addr).await;

                                let mut devs = devices_listen.lock().await;
                                devs.insert(
                                    ip_str.clone(),
                                    UdpDevice {
                                        id: ip_str.clone(),
                                        name: name.to_string(),
                                        ip: ip_str,
                                        last_seen: Instant::now(),
                                    },
                                );
                            }
                        }
                    }
                }
            });

            // Broadcast loop
            let broadcast_socket = socket.clone();
            tokio::spawn(async move {
                let mut ticker = interval(Duration::from_secs(2));
                let broadcast_addr = SocketAddrV4::new(Ipv4Addr::BROADCAST, UDP_DISCOVERY_PORT);

                loop {
                    ticker.tick().await;

                    {
                        let mut devs = devices.lock().await;
                        devs.retain(|_, d| d.last_seen.elapsed() < Duration::from_secs(10));
                    }

                    let my_current_name = my_name.lock().await.clone();
                    let msg = format!("{}{}", QUICKDROP_PREFIX, my_current_name);

                    let mut broadcasted = false;
                    if let Ok(interfaces) = local_ip_address::list_afinet_netifas() {
                        for (_, ip) in interfaces {
                            if let std::net::IpAddr::V4(ipv4) = ip {
                                if ipv4.is_loopback() { continue; }
                                if let Ok(sock) = std::net::UdpSocket::bind(std::net::SocketAddrV4::new(ipv4, 0)) {
                                    let _ = sock.set_broadcast(true);
                                    
                                    if sock.send_to(msg.as_bytes(), broadcast_addr).is_ok() {
                                        broadcasted = true;
                                    }
                                    
                                    let octets = ipv4.octets();
                                    let subnet_bcast = std::net::SocketAddrV4::new(
                                        std::net::Ipv4Addr::new(octets[0], octets[1], octets[2], 255),
                                        UDP_DISCOVERY_PORT
                                    );
                                    let _ = sock.send_to(msg.as_bytes(), subnet_bcast);
                                }
                            }
                        }
                    }
                    
                    if !broadcasted {
                        let _ = broadcast_socket.send_to(msg.as_bytes(), broadcast_addr).await;
                    }
                }
            });
        });
    }

    pub async fn get_devices(&self) -> Vec<UdpDevice> {
        let devs = self.devices.lock().await;
        devs.values().cloned().collect()
    }
}
