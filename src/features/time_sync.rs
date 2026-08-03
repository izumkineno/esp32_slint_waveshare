//! WiFi time synchronization using the NTP protocol.

use embassy_futures::select::{select, Either};
use embassy_net::{
    udp::{PacketMetadata, UdpSocket},
    IpAddress, IpEndpoint, Stack,
};
use embassy_time::{Duration, Timer};

use crate::features::config;

const NTP_SERVERS: &[&str] = &[
    "ntp.aliyun.com",
    "ntp1.aliyun.com",
    "pool.ntp.org",
    "time.cloudflare.com",
    "time.google.com",
];
const DNS_SERVER: [u8; 4] = [223, 5, 5, 5];
const DNS_PORT: u16 = 53;
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
    crate::esp_debug!("TIME: {} resolved by 223.5.5.5 to {:?}", server, server_ip);

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

    if let Err(error) = socket.bind(IpEndpoint::new(IpAddress::v4(0, 0, 0, 0), 0)) {
        crate::esp_warn!("TIME: NTP UDP bind failed for {}: {:?}", server, error);
        return None;
    }

    let mut request = [0u8; NTP_PACKET_LEN];
    request[0] = 0x23; // LI=0, VN=4, Mode=3 (client)
    crate::esp_debug!("TIME: sending NTP request to {} ({:?})", server, server_ip);
    if let Err(error) = socket
        .send_to(&request, IpEndpoint::new(server_ip, NTP_PORT))
        .await
    {
        crate::esp_warn!(
            "TIME: NTP request failed for {} ({:?}): {:?}",
            server,
            server_ip,
            error
        );
        return None;
    }

    let mut response = [0u8; 128];
    match select(
        socket.recv_from(&mut response),
        Timer::after(RESPONSE_TIMEOUT),
    )
    .await
    {
        Either::First(Ok((length, _))) => {
            let response = &response[..length.min(response.len())];
            let timestamp = parse_response(response);
            crate::esp_debug!(
                "TIME: NTP response from {} length={}, valid={}",
                server,
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
    let mut rx_metadata = [PacketMetadata::EMPTY; 1];
    let mut tx_metadata = [PacketMetadata::EMPTY; 1];
    let mut rx_buffer = [0u8; 512];
    let mut tx_buffer = [0u8; 128];
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_metadata,
        &mut rx_buffer,
        &mut tx_metadata,
        &mut tx_buffer,
    );

    if let Err(error) = socket.bind(IpEndpoint::new(IpAddress::v4(0, 0, 0, 0), 0)) {
        crate::esp_warn!("TIME: DNS UDP bind failed: {:?}", error);
        return None;
    }

    let mut query = [0u8; 128];
    let query_length = encode_dns_query(name, &mut query)?;
    crate::esp_debug!(
        "TIME: sending DNS query for {} ({} bytes)",
        name,
        query_length
    );
    let dns_server = IpAddress::v4(DNS_SERVER[0], DNS_SERVER[1], DNS_SERVER[2], DNS_SERVER[3]);
    if let Err(error) = socket
        .send_to(
            &query[..query_length],
            IpEndpoint::new(dns_server, DNS_PORT),
        )
        .await
    {
        crate::esp_warn!(
            "TIME: DNS request to {:?} failed for {}: {:?}",
            DNS_SERVER,
            name,
            error
        );
        return None;
    }

    let mut response = [0u8; 512];
    match select(socket.recv_from(&mut response), Timer::after(DNS_TIMEOUT)).await {
        Either::First(Ok((length, _))) => {
            let response = &response[..length.min(response.len())];
            let address = parse_dns_response(response);
            crate::esp_debug!(
                "TIME: DNS response for {} length={}, ipv4_found={}",
                name,
                length,
                address.is_some()
            );
            if address.is_none() {
                crate::esp_warn!(
                    "TIME: DNS response from {:?} has no IPv4 answer for {}",
                    DNS_SERVER,
                    name
                );
            }
            address
        }
        Either::First(Err(error)) => {
            crate::esp_warn!("TIME: DNS receive failed for {}: {:?}", name, error);
            None
        }
        Either::Second(_) => {
            crate::esp_warn!(
                "TIME: DNS response timed out from {:?} for {}",
                DNS_SERVER,
                name
            );
            None
        }
    }
}

fn encode_dns_query(name: &str, packet: &mut [u8; 128]) -> Option<usize> {
    packet.fill(0);
    packet[0] = 0x5a;
    packet[1] = 0xc3;
    packet[2] = 0x01; // recursion desired
    packet[5] = 0x01; // one question

    let mut offset: usize = 12;
    for label in name.split('.') {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        let end = offset.checked_add(label.len() + 1)?;
        if end >= packet.len() {
            return None;
        }
        packet[offset] = label.len() as u8;
        offset += 1;
        packet[offset..offset + label.len()].copy_from_slice(label.as_bytes());
        offset += label.len();
    }

    let end = offset.checked_add(5)?;
    if end > packet.len() {
        return None;
    }
    packet[offset] = 0;
    offset += 1;
    packet[offset..offset + 2].copy_from_slice(&1u16.to_be_bytes()); // A
    packet[offset + 2..offset + 4].copy_from_slice(&1u16.to_be_bytes()); // IN
    Some(end)
}

fn parse_dns_response(packet: &[u8]) -> Option<IpAddress> {
    if packet.len() < 12
        || packet[0] != 0x5a
        || packet[1] != 0xc3
        || packet[2] & 0x80 == 0
        || packet[3] & 0x0f != 0
    {
        return None;
    }

    let question_count = u16::from_be_bytes([packet[4], packet[5]]) as usize;
    let answer_count = u16::from_be_bytes([packet[6], packet[7]]) as usize;
    let mut offset = 12;

    for _ in 0..question_count {
        offset = skip_dns_name(packet, offset)?;
        offset = offset.checked_add(4)?;
        if offset > packet.len() {
            return None;
        }
    }

    for _ in 0..answer_count {
        offset = skip_dns_name(packet, offset)?;
        if offset.checked_add(10)? > packet.len() {
            return None;
        }
        let record_type = u16::from_be_bytes([packet[offset], packet[offset + 1]]);
        let record_class = u16::from_be_bytes([packet[offset + 2], packet[offset + 3]]);
        let data_length = u16::from_be_bytes([packet[offset + 8], packet[offset + 9]]) as usize;
        offset += 10;
        let data_end = offset.checked_add(data_length)?;
        if data_end > packet.len() {
            return None;
        }
        if record_type == 1 && record_class == 1 && data_length == 4 {
            return Some(IpAddress::v4(
                packet[offset],
                packet[offset + 1],
                packet[offset + 2],
                packet[offset + 3],
            ));
        }
        offset = data_end;
    }

    None
}

fn skip_dns_name(packet: &[u8], mut offset: usize) -> Option<usize> {
    loop {
        let length = *packet.get(offset)? as usize;
        if length == 0 {
            return offset.checked_add(1);
        }
        if length & 0xc0 == 0xc0 {
            packet.get(offset + 1)?;
            return offset.checked_add(2);
        }
        if length & 0xc0 != 0 {
            return None;
        }
        offset = offset.checked_add(length + 1)?;
        if offset > packet.len() {
            return None;
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

#[cfg(all(test, not(target_arch = "xtensa")))]
mod tests {
    use super::*;

    #[test]
    fn dns_query_encodes_single_a_question() {
        let mut packet = [0u8; 128];
        let length = encode_dns_query("ntp.aliyun.com", &mut packet).unwrap();

        assert_eq!(&packet[..2], &[0x5a, 0xc3]);
        assert_eq!(&packet[2..6], &[0x01, 0x00, 0x00, 0x01]);
        assert_eq!(
            &packet[12..length],
            &[
                3, b'n', b't', b'p', 6, b'a', b'l', b'i', b'y', b'u', b'n', 3, b'c', b'o', b'm', 0,
                0, 1, 0, 1
            ]
        );
    }

    #[test]
    fn dns_response_parses_compressed_ipv4_answer() {
        let mut packet = [0u8; 128];
        packet[..8].copy_from_slice(&[0x5a, 0xc3, 0x81, 0x80, 0, 1, 0, 1]);

        let question = [
            3, b'n', b't', b'p', 6, b'a', b'l', b'i', b'y', b'u', b'n', 3, b'c', b'o', b'm', 0, 0,
            1, 0, 1,
        ];
        packet[12..12 + question.len()].copy_from_slice(&question);

        let answer_offset = 12 + question.len();
        packet[answer_offset..answer_offset + 16]
            .copy_from_slice(&[0xc0, 0x0c, 0, 1, 0, 1, 0, 0, 0, 1, 0, 4, 203, 107, 6, 88]);

        assert_eq!(
            parse_dns_response(&packet[..answer_offset + 16]),
            Some(IpAddress::v4(203, 107, 6, 88))
        );
    }

    #[test]
    fn ntp_response_converts_unix_timestamp() {
        let mut packet = [0u8; NTP_PACKET_LEN];
        packet[0] = 0x24;
        packet[1] = 1;
        packet[40..44].copy_from_slice(&(NTP_UNIX_OFFSET + 123).to_be_bytes());

        assert_eq!(parse_response(&packet), Some(123));
    }
}
