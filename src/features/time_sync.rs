//! WiFi time synchronization using the NTP protocol.

use embassy_futures::select::{select, Either};
use embassy_net::{
    udp::{PacketMetadata, UdpMetadata, UdpSocket},
    IpAddress, IpEndpoint, IpListenEndpoint, Stack,
};
use embassy_time::{Duration, Timer};

use crate::features::config;

const NTP_SERVERS: &[&str] = &[
    "ntp.aliyun.com",
    "ntp1.aliyun.com",
    "pool.ntp.org",
    "time.cloudflare.com",
    "time.google.com",
    "ntp.ntsc.ac.cn",
    "s1a.time.edu.cn",
    "s1b.time.edu.cn",
    "s1c.time.edu.cn",
    "s1d.time.edu.cn",
    "s1e.time.edu.cn",
    "s2a.time.edu.cn",
    "s2b.time.edu.cn",
    "s2c.time.edu.cn",
    "s2d.time.edu.cn",
    "s2e.time.edu.cn",
    "s2f.time.edu.cn",
    "s2g.time.edu.cn",
    "s2h.time.edu.cn",
    "s2j.time.edu.cn",
    "s2k.time.edu.cn",
    "s2m.time.edu.cn",
];

const DNS_TIMEOUT: Duration = Duration::from_secs(5);
const NTP_PORT: u16 = 123;
const NTP_PACKET_LEN: usize = 48;
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;
const SYNC_INTERVAL: Duration = Duration::from_secs(3_600);
const RETRY_INTERVAL: Duration = Duration::from_secs(30);
const RESPONSE_TIMEOUT: Duration = Duration::from_secs(8);

#[embassy_executor::task]
pub async fn run(stack: Stack<'static>) {
    crate::esp_info!("TIME: NTP sync task started");

    loop {
        if !stack.is_link_up() || !stack.is_config_up() {
            crate::esp_debug!(
                "TIME: network not ready, link_up={}, config_up={}",
                stack.is_link_up(),
                stack.is_config_up()
            );
            Timer::after(RETRY_INTERVAL).await;
            continue;
        }

        crate::esp_info!(
            "TIME: station network ready, link_up={}, config={:?}",
            stack.is_link_up(),
            stack.config_v4()
        );

        match sync_once(stack).await {
            Some(timestamp) => {
                crate::esp_info!("TIME: NTP response received, unix={}", timestamp);
                config::publish_time_sync(timestamp);
                Timer::after(SYNC_INTERVAL).await;
            }
            None => {
                crate::esp_warn!("TIME: NTP synchronization failed after trying all servers");
                Timer::after(RETRY_INTERVAL).await;
            }
        }
    }
}

async fn sync_once(stack: Stack<'static>) -> Option<u64> {
    for server in NTP_SERVERS {
        crate::esp_debug!("TIME: trying NTP server {}", server);
        if let Some(timestamp) = sync_server(stack, server).await {
            crate::esp_info!("TIME: synchronized from {}", server);
            return Some(timestamp);
        }

        crate::esp_warn!("TIME: NTP server {} did not respond", server);
    }

    None
}

async fn sync_server(stack: Stack<'static>, server: &str) -> Option<u64> {
    crate::esp_debug!("TIME: resolving NTP server {}", server);
    let server_ip = resolve_ipv4(stack, server).await?;
    crate::esp_debug!("TIME: {} resolved to {:?}", server, server_ip);

    let mut rx_metadata = [PacketMetadata::EMPTY; 1];
    let mut tx_metadata = [PacketMetadata::EMPTY; 1];
    let mut rx_buffer = [0u8; 128];
    let mut tx_buffer = [0u8; NTP_PACKET_LEN];
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_metadata,
        &mut rx_buffer,
        &mut tx_metadata,
        &mut tx_buffer,
    );

    // Use a wildcard listener; binding 0.0.0.0 as a concrete address rejects replies.
    if let Err(error) = socket.bind(IpListenEndpoint {
        addr: None,
        port: 0,
    }) {
        crate::esp_warn!("TIME: NTP UDP bind failed for {}: {:?}", server, error);
        return None;
    }
    let Some(local_address) = stack
        .config_v4()
        .map(|config| IpAddress::Ipv4(config.address.address()))
    else {
        crate::esp_warn!("TIME: NTP station IPv4 address unavailable for {}", server);
        return None;
    };
    let remote_endpoint = UdpMetadata {
        endpoint: IpEndpoint::new(server_ip, NTP_PORT),
        local_address: Some(local_address),
        meta: Default::default(),
    };
    crate::esp_debug!(
        "TIME: NTP socket bound local={:?}, source={:?}, destination={:?}",
        socket.endpoint(),
        local_address,
        remote_endpoint.endpoint
    );

    let mut request = [0u8; NTP_PACKET_LEN];
    request[0] = 0x23; // LI=0, VN=4, Mode=3 (client)
    crate::esp_debug!("TIME: sending NTP request to {} ({:?})", server, server_ip);
    if let Err(error) = socket.send_to(&request, remote_endpoint).await {
        crate::esp_warn!(
            "TIME: NTP request failed for {} ({:?}): {:?}",
            server,
            server_ip,
            error
        );
        return None;
    }
    match select(socket.flush(), Timer::after(RESPONSE_TIMEOUT)).await {
        Either::First(()) => {
            crate::esp_debug!(
                "TIME: NTP request flushed from local socket {:?}",
                socket.endpoint()
            );
        }
        Either::Second(_) => {
            crate::esp_warn!("TIME: NTP request flush timed out for {}", server);
            return None;
        }
    }

    let mut response = [0u8; 128];
    match select(
        socket.recv_from(&mut response),
        Timer::after(RESPONSE_TIMEOUT),
    )
    .await
    {
        Either::First(Ok((length, metadata))) => {
            let response = &response[..length.min(response.len())];
            let timestamp = parse_response(response);
            crate::esp_debug!(
                "TIME: NTP response from {} source={:?}, local={:?}, length={}, valid={}",
                server,
                metadata.endpoint,
                metadata.local_address,
                length,
                timestamp.is_some()
            );
            timestamp
        }
        Either::First(Err(error)) => {
            crate::esp_warn!(
                "TIME: NTP receive failed for {} ({:?}): {:?}",
                server,
                server_ip,
                error
            );
            None
        }
        Either::Second(_) => {
            crate::esp_warn!(
                "TIME: NTP response timed out for {} ({:?})",
                server,
                server_ip
            );
            None
        }
    }
}

async fn resolve_ipv4(stack: Stack<'static>, name: &str) -> Option<IpAddress> {
    crate::esp_debug!("TIME: resolving DNS name {}", name);
    match select(
        stack.dns_query(name, embassy_net::dns::DnsQueryType::A),
        Timer::after(DNS_TIMEOUT),
    )
    .await
    {
        Either::First(Ok(addresses)) => {
            let address = addresses.into_iter().next();
            crate::esp_debug!("TIME: DNS response for {}: {:?}", name, address);
            if address.is_none() {
                crate::esp_warn!("TIME: DNS response for {} contained no IPv4 address", name);
            }
            address
        }
        Either::First(Err(error)) => {
            crate::esp_warn!("TIME: DNS query failed for {}: {:?}", name, error);
            None
        }
        Either::Second(_) => {
            crate::esp_warn!("TIME: DNS query timed out for {}", name);
            None
        }
    }
}

fn parse_response(response: &[u8]) -> Option<u64> {
    if response.len() < NTP_PACKET_LEN {
        return None;
    }

    let mode = response[0] & 0x07;
    if mode != 4 && mode != 5 {
        return None;
    }
    if response[1] == 0 {
        return None;
    }

    let ntp_seconds =
        u32::from_be_bytes([response[40], response[41], response[42], response[43]]) as u64;
    ntp_seconds.checked_sub(NTP_UNIX_OFFSET)
}
