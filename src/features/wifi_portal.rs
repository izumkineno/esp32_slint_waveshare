//! 板载 SoftAP 配置门户。
//!
//! 设备启动后同时提供配置 AP 和一个可选的 STA 接口：
//! - 手机连接 `ESP32-S3-配置` / `esp32s3-config`；
//! - 浏览器访问 `http://192.168.4.1/`；
//! - 提交 WiFi 凭据后，设备尝试连接目标网络；
//! - 蓝牙名称由 BLE 广播任务读取，下一次广播周期生效。

use alloc::string::String;
use core::{fmt::Write as _, net::Ipv4Addr};
use embassy_executor::Spawner;
use embassy_futures::select::{select, Either};
use embassy_net::{
    tcp::TcpSocket,
    udp::{PacketMetadata, UdpMetadata, UdpSocket},
    IpAddress, IpEndpoint, IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources,
    StaticConfigV4,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    ap::AccessPointConfig, scan::ScanConfig, sta::StationConfig, AuthenticationMethod, Config,
    ControllerConfig, DisconnectReason, Interface, WifiController, WifiError,
};
use static_cell::StaticCell;

use crate::features::config::{self, WifiCredentials};
use crate::features::time_sync;

pub const AP_SSID: &str = "ESP32-S3-配置";
pub const AP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 4, 1);
const WIFI_SCAN_LIMIT: usize = 8;

// Station 认证方式：作为“可接受的最低安全等级”传给底层 threshold.authmode。
// Wpa2Personal 已同时兼容 WPA2 / WPA2-WPA3 过渡 / WPA3（它们的等级都 >= WPA2）。
// 只有当目标路由器是老旧的纯 WPA/WPA1 时，才需要改成 WpaWpa2Personal。
// 底层 PMF 固定为 capable=true, required=false，无需也无法在此配置。
const STATION_AUTH_METHOD: AuthenticationMethod = AuthenticationMethod::Wpa2Personal;

fn ap_config() -> AccessPointConfig {
    AccessPointConfig::default()
        .with_ssid(AP_SSID)
        .with_auth_method(AuthenticationMethod::None)
        .with_channel(1)
}

fn station_config(credentials: WifiCredentials) -> StationConfig {
    let ssid = core::str::from_utf8(&credentials.ssid[..credentials.ssid_len]).unwrap_or("");
    let password =
        core::str::from_utf8(&credentials.password[..credentials.password_len]).unwrap_or("");
    let mut config = StationConfig::default()
        .with_ssid(ssid)
        .with_password(String::from(password));
    if credentials.password_len == 0 {
        // 开放网络：无密码即无认证。
        config = config.with_auth_method(AuthenticationMethod::None);
    } else {
        // 有密码：显式声明可接受的最低认证等级，避免依赖隐式默认值。
        config = config.with_auth_method(STATION_AUTH_METHOD);
    }
    config
}

fn wifi_config(
    ap_enabled: bool,
    station_enabled: bool,
    credentials: Option<WifiCredentials>,
) -> Config {
    let station = credentials
        .map(station_config)
        .unwrap_or_else(StationConfig::default);
    match (ap_enabled, station_enabled) {
        (true, true) => Config::AccessPointStation(station, ap_config()),
        (true, false) => Config::AccessPoint(ap_config()),
        (false, true) | (false, false) => Config::Station(station),
    }
}

fn apply_wifi_config(
    controller: &mut WifiController<'static>,
    ap_enabled: bool,
    station_enabled: bool,
    credentials: Option<WifiCredentials>,
) -> bool {
    match controller.set_config(&wifi_config(ap_enabled, station_enabled, credentials)) {
        Ok(()) => {
            config::set_wifi_mode_state(ap_enabled, station_enabled);
            crate::esp_info!(
                "WIFI: mode applied, ap={}, station={}",
                ap_enabled,
                station_enabled
            );
            if !station_enabled {
                config::set_wifi_connection(config::WIFI_CONNECTION_DISABLED, None);
            } else if !controller.is_connected() {
                config::set_wifi_connection(config::WIFI_CONNECTION_DISCONNECTED, None);
            }
            true
        }
        Err(error) => {
            crate::esp_warn!("WIFI: mode change failed: {:?}", error);
            config::set_wifi_mode_state(false, false);
            config::set_wifi_connection(config::WIFI_CONNECTION_FAILED, None);
            false
        }
    }
}

async fn connect_station(
    controller: &mut WifiController<'static>,
    credentials: WifiCredentials,
) -> bool {
    let ssid = core::str::from_utf8(&credentials.ssid[..credentials.ssid_len]).unwrap_or("");
    config::set_wifi_connection(config::WIFI_CONNECTION_CONNECTING, Some(ssid));
    crate::esp_info!("WIFI: connecting to {}", ssid);
    match controller.connect_async().await {
        Ok(connection) => {
            crate::esp_info!("WIFI: connected to {}", connection.ssid.as_str());
            config::set_wifi_connection(
                config::WIFI_CONNECTION_CONNECTED,
                Some(connection.ssid.as_str()),
            );
            true
        }
        Err(error) => {
            crate::esp_warn!("WIFI: connection failed: {:?}", error);
            if let WifiError::Disconnected(info) = error {
                crate::esp_warn!(
                    "WIFI: disconnect reason={:?}, rssi={} -> {}",
                    info.reason,
                    info.rssi,
                    describe_disconnect_reason(info.reason)
                );
            }
            config::set_wifi_connection(config::WIFI_CONNECTION_FAILED, Some(ssid));
            false
        }
    }
}

/// 把底层断连原因翻译成可操作的中文提示，便于从日志直接判断故障根因。
fn describe_disconnect_reason(reason: DisconnectReason) -> &'static str {
    match reason {
        // 关联成功、卡在 WPA2 四次握手 —— PSK 不匹配的典型特征。
        DisconnectReason::FourWayHandshakeTimeout
        | DisconnectReason::HandshakeTimeout
        | DisconnectReason::MicFailure => "密码错误：关联成功但握手失败，请核对 WiFi 密码",
        // 认证阶段被拒。
        DisconnectReason::AuthenticationFailed
        | DisconnectReason::_802_1xAuthenticationFailed => "认证被拒：密码或认证方式不符",
        // 安全能力/阈值不匹配 —— 才是真正的 WPA 版本或 PMF 问题。
        DisconnectReason::NoAccessPointFoundWithCompatibleSecurity
        | DisconnectReason::NoAccessPointFoundInAuthmodeThreshold => {
            "安全能力不匹配：路由器加密方式与本机不符（此时才需调整 STATION_AUTH_METHOD）"
        }
        // 根本没找到 AP。
        DisconnectReason::NoAccessPointFound => "未找到 AP：请确认 SSID 正确且为 2.4GHz 频段",
        DisconnectReason::NoAccessPointFoundInRssiThreshold => "信号过弱：AP 存在但 RSSI 低于阈值",
        DisconnectReason::BeaconTimeout | DisconnectReason::AssociationFailed => {
            "关联失败或信标超时：信号弱或路由器拒绝，可靠近后重试"
        }
        DisconnectReason::AssociationLeave | DisconnectReason::AuthenticationLeave => {
            "被路由器主动断开：可能触发了 MAC 过滤或连接数限制"
        }
        _ => "其他原因：参见上方 reason 字段",
    }
}

async fn disconnect_station(controller: &mut WifiController<'static>) {
    crate::esp_info!("WIFI: disconnect requested by controller");
    if controller.is_connected() {
        if let Err(error) = controller.disconnect_async().await {
            crate::esp_warn!("WIFI: disconnect failed: {:?}", error);
        }
    }
    config::set_wifi_connection(config::WIFI_CONNECTION_DISCONNECTED, None);
    crate::esp_info!("WIFI: station disconnected");
}

async fn scan_wifi(
    controller: &mut WifiController<'static>,
    station_enabled: bool,
) -> Option<([config::WifiScanEntry; config::MAX_SCAN_RESULTS], usize)> {
    if !station_enabled {
        crate::esp_warn!("WIFI: scan skipped: station interface is disabled");
        return None;
    }

    // esp-radio supports scanning in Station and AP+Station modes. Do not
    // restart WiFi just to scan: set_config() reallocates driver resources
    // and can fail with OutOfMemory while the UI and BLE are active.
    crate::esp_info!("WIFI: scanning without changing AP/STA mode");

    let scan_result = match select(
        controller.scan_async(&ScanConfig::default().with_max(WIFI_SCAN_LIMIT)),
        Timer::after(Duration::from_secs(10)),
    )
    .await
    {
        Either::First(Ok(results)) => {
            let mut entries = [config::WifiScanEntry::empty(); config::MAX_SCAN_RESULTS];
            let mut count = 0;
            for access_point in results {
                if count == WIFI_SCAN_LIMIT {
                    break;
                }
                let entry = &mut entries[count];
                let ssid = access_point.ssid.as_str().as_bytes();
                entry.ssid_len = ssid.len().min(entry.ssid.len());
                entry.ssid[..entry.ssid_len].copy_from_slice(&ssid[..entry.ssid_len]);
                entry.signal_strength = access_point.signal_strength;
                entry.secured =
                    !matches!(access_point.auth_method, Some(AuthenticationMethod::None));
                count += 1;
            }
            Some((entries, count))
        }
        Either::First(Err(error)) => {
            crate::esp_warn!("WIFI: scan failed without changing mode: {:?}", error);
            None
        }
        Either::Second(_) => {
            crate::esp_warn!("WIFI: scan timed out");
            None
        }
    };

    scan_result
}

macro_rules! mk_static {
    ($t:ty, $value:expr) => {{
        static CELL: StaticCell<$t> = StaticCell::new();
        CELL.uninit().write($value)
    }};
}

pub fn start(spawner: Spawner, wifi: esp_hal::peripherals::WIFI<'static>) {
    crate::esp_info!("WIFI: starting controller and network services");
    let boot_credentials = config::boot_wifi_credentials();
    let initial_config = wifi_config(false, true, boot_credentials);
    let (mut controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(initial_config),
    )
    .expect("WiFi 初始化失败");
    controller
        .set_power_saving(esp_radio::wifi::PowerSaveMode::None)
        .expect("WiFi 省电模式配置失败");
    crate::esp_info!("WIFI: controller initialized; SoftAP is disabled by default");

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(AP_ADDRESS, 24),
        gateway: None,
        dns_servers: Default::default(),
    });
    let sta_config = embassy_net::Config::dhcpv4(Default::default());
    let (ap_stack, ap_runner) = embassy_net::new(
        interfaces.access_point,
        ap_config,
        mk_static!(StackResources<4>, StackResources::<4>::new()),
        seed,
    );
    let (sta_stack, sta_runner) = embassy_net::new(
        interfaces.station,
        sta_config,
        mk_static!(StackResources<4>, StackResources::<4>::new()),
        seed,
    );

    spawner.spawn(wifi_controller(controller).unwrap());
    spawner.spawn(net_task(ap_runner).unwrap());
    spawner.spawn(net_task(sta_runner).unwrap());
    spawner.spawn(time_sync::run(sta_stack).unwrap());
    spawner.spawn(run_dhcp(ap_stack).unwrap());
    spawner.spawn(http_server(ap_stack).unwrap());
    crate::esp_info!("WIFI: controller and network tasks spawned");
}

#[embassy_executor::task]
async fn wifi_controller(mut controller: WifiController<'static>) {
    let boot_credentials = config::boot_wifi_credentials();
    let mut last_credentials = boot_credentials;
    let mut ap_enabled = false;
    let mut station_enabled = true;
    let mut auto_connect = boot_credentials.is_some();
    let mut retry_ticks = 0u8;

    if let Some(credentials) = boot_credentials {
        crate::esp_info!(
            "WIFI: boot auto-connect enabled, ssid_len={}, password_len={}",
            credentials.ssid_len,
            credentials.password_len
        );
    } else {
        crate::esp_info!("WIFI: boot auto-connect disabled; no SSID configured");
    }

    config::set_wifi_mode_state(ap_enabled, station_enabled);
    config::set_wifi_connection(config::WIFI_CONNECTION_DISCONNECTED, None);
    crate::esp_info!("WIFI: controller task started");

    loop {
        let command = config::take_wifi_control();
        let mut mode_changed = false;

        if let Some(enabled) = command.ap_enabled {
            if enabled != ap_enabled {
                ap_enabled = enabled;
                mode_changed = true;
            }
            crate::esp_info!("WIFI: controller AP change applied locally: {}", enabled);
        }

        if let Some(enabled) = command.station_enabled {
            if enabled != station_enabled {
                station_enabled = enabled;
                mode_changed = true;
            }
            crate::esp_info!(
                "WIFI: controller station change applied locally: {}",
                enabled
            );
            auto_connect = enabled && last_credentials.is_some();
            retry_ticks = 0;
        }

        if command.disconnect {
            crate::esp_info!("WIFI: controller processing disconnect");
            auto_connect = false;
            retry_ticks = 0;
            if station_enabled {
                disconnect_station(&mut controller).await;
            } else {
                config::set_wifi_connection(config::WIFI_CONNECTION_DISABLED, None);
            }
        }

        if let Some(credentials) = config::take_wifi_command() {
            crate::esp_info!(
                "WIFI: controller received credentials, ssid_len={}, password_len={}",
                credentials.ssid_len,
                credentials.password_len
            );
            last_credentials = Some(credentials);
            mode_changed = true;
            auto_connect = true;
            retry_ticks = 0;
            if !station_enabled {
                station_enabled = true;
                mode_changed = true;
            }
        }

        if mode_changed {
            crate::esp_info!(
                "WIFI: applying controller mode, ap={}, station={}",
                ap_enabled,
                station_enabled
            );
            if !apply_wifi_config(
                &mut controller,
                ap_enabled,
                station_enabled,
                last_credentials,
            ) {
                ap_enabled = false;
                station_enabled = false;
                auto_connect = false;
            }
        }

        if config::take_wifi_scan_request() {
            match scan_wifi(&mut controller, station_enabled).await {
                Some((entries, count)) => {
                    crate::esp_info!("WIFI: scan finished with {} result(s)", count);
                    config::finish_wifi_scan(entries, count);
                }
                None => config::fail_wifi_scan(),
            }
            retry_ticks = 0;
            continue;
        }

        if station_enabled && auto_connect && retry_ticks == 0 {
            if let Some(credentials) = last_credentials {
                if !controller.is_connected() {
                    crate::esp_debug!("WIFI: attempting automatic station connection");
                    if !connect_station(&mut controller, credentials).await {
                        retry_ticks = 20;
                    }
                }
            }
        }

        retry_ticks = retry_ticks.saturating_sub(1);
        Timer::after_millis(1000).await;
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    crate::esp_info!("NET: network runner started");
    runner.run().await;
}

#[embassy_executor::task]
async fn run_dhcp(stack: Stack<'static>) {
    use edge_dhcp::{
        server::{Server as DhcpServer, ServerOptions},
        Packet,
    };

    let mut rx_metadata = [PacketMetadata::EMPTY; 8];
    let mut tx_metadata = [PacketMetadata::EMPTY; 8];
    let mut rx_buffer = [0u8; 1024];
    let mut tx_buffer = [0u8; 1024];
    let mut socket = UdpSocket::new(
        stack,
        &mut rx_metadata,
        &mut rx_buffer,
        &mut tx_metadata,
        &mut tx_buffer,
    );
    let dhcp_endpoint = IpEndpoint::new(IpAddress::v4(0, 0, 0, 0), 67);

    crate::esp_info!("DHCP: binding UDP port 67");
    loop {
        match socket.bind(dhcp_endpoint) {
            Ok(()) => {
                crate::esp_info!("DHCP: UDP port 67 is ready");
                break;
            }
            Err(bind_error) => {
                crate::esp_warn!("DHCP: bind failed: {:?}", bind_error);
                Timer::after_millis(100).await;
            }
        }
    }

    fn dhcp_now() -> u64 {
        embassy_time::Instant::now().as_secs()
    }

    let mut gateway_buffer = [AP_ADDRESS];
    let server_options = ServerOptions::new(AP_ADDRESS, Some(&mut gateway_buffer));
    let mut dhcp_server: DhcpServer<fn() -> u64, 16> = DhcpServer::new(dhcp_now, AP_ADDRESS);
    let mut request_buffer = [0u8; 1024];
    let mut response_buffer = [0u8; 1024];

    loop {
        let Ok((length, metadata)) = socket.recv_from(&mut request_buffer).await else {
            continue;
        };
        crate::esp_info!("DHCP: received {} bytes from {:?}", length, metadata);

        let Ok(request) = Packet::decode(&request_buffer[..length]) else {
            crate::esp_warn!("DHCP: packet decode failed");
            continue;
        };
        let mut option_buffer = edge_dhcp::Options::buf();
        let Some(reply) = dhcp_server.handle_request(&mut option_buffer, &server_options, &request)
        else {
            crate::esp_warn!("DHCP: request did not produce a reply");
            continue;
        };
        let Ok(response) = reply.encode(&mut response_buffer) else {
            crate::esp_warn!("DHCP: response encode failed");
            continue;
        };
        let dhcp_reply_endpoint = UdpMetadata {
            endpoint: IpEndpoint::new(IpAddress::v4(255, 255, 255, 255), 68),
            local_address: Some(IpAddress::v4(192, 168, 4, 1)),
            meta: Default::default(),
        };
        if let Err(send_error) = socket.send_to(response, dhcp_reply_endpoint).await {
            crate::esp_warn!("DHCP: response send failed: {:?}", send_error);
        }
    }
}

enum HttpBody {
    Static(&'static [u8]),
    Dynamic(String),
}

impl HttpBody {
    fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Static(body) => body,
            Self::Dynamic(body) => body.as_bytes(),
        }
    }
}

#[embassy_executor::task]
async fn http_server(stack: Stack<'static>) {
    let mut rx_buffer = [0u8; 4096];
    let mut tx_buffer = [0u8; 2048];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(10)));
    crate::esp_info!("HTTP: configuration server started on port 80");

    loop {
        let accepted = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: 80,
            })
            .await;
        if let Err(error) = accepted {
            crate::esp_warn!("HTTP: accept failed: {:?}", error);
            socket.abort();
            continue;
        }
        crate::esp_debug!("HTTP: client connected");

        let mut request = [0u8; 4096];
        let mut length = 0usize;
        loop {
            if length == request.len() {
                break;
            }
            match socket.read(&mut request[length..]).await {
                Ok(0) | Err(_) => break,
                Ok(read) => {
                    length += read;
                    if request_complete(&request[..length]) {
                        break;
                    }
                }
            }
        }

        crate::esp_debug!("HTTP: request received, bytes={}", length);
        let (header, body) = if request.starts_with(b"GET /api/wifi/scan ") {
            crate::esp_info!("HTTP: WiFi scan requested");
            config::request_wifi_scan();
            (
                b"HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                    as &[u8],
                HttpBody::Static(API_SCAN_ACCEPTED),
            )
        } else if request.starts_with(b"GET /api/wifi/results ") {
            (
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                    as &[u8],
                HttpBody::Dynamic(wifi_results_json()),
            )
        } else if request.starts_with(b"GET /api/ble/scan ") {
            crate::esp_info!("HTTP: BLE scan requested");
            config::request_ble_scan();
            (
                b"HTTP/1.1 202 Accepted\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                    as &[u8],
                HttpBody::Static(API_SCAN_ACCEPTED),
            )
        } else if request.starts_with(b"GET /api/ble/results ") {
            (
                b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                    as &[u8],
                HttpBody::Dynamic(ble_results_json()),
            )
        } else {
            let updated = process_request(&request[..length]);
            let body = if updated {
                CONFIG_SAVED_HTML
            } else {
                PORTAL_HTML
            };
            (
                b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n"
                    as &[u8],
                HttpBody::Static(body),
            )
        };

        let _ = socket.write_all(header).await;
        let _ = socket.write_all(body.as_bytes()).await;
        let _ = socket.flush().await;
        socket.close();
        Timer::after_millis(20).await;
        crate::esp_debug!("HTTP: response sent");
        socket.abort();
    }
}

fn request_complete(request: &[u8]) -> bool {
    let Some(header_end) = find_bytes(request, b"\r\n\r\n") else {
        return false;
    };
    let content_length = header_value_number(&request[..header_end], b"Content-Length:");
    request.len() >= header_end + 4 + content_length
}

fn process_request(request: &[u8]) -> bool {
    let Some(header_end) = find_bytes(request, b"\r\n\r\n") else {
        return false;
    };
    if !request.starts_with(b"POST /config ") {
        return false;
    }

    let body = &request[header_end + 4..];
    let mut credentials = WifiCredentials::empty();
    if let Some(length) = form_value(body, b"wifi_ssid", &mut credentials.ssid) {
        credentials.ssid_len = length;
        if length > 0 {
            if let Some(password_length) =
                form_value(body, b"wifi_password", &mut credentials.password)
            {
                credentials.password_len = password_length;
                config::store_wifi_command(credentials);
            }
        }
    }

    let mut ble_name = [0u8; 32];
    if let Some(length) = form_value(body, b"ble_name", &mut ble_name) {
        if length > 0 {
            config::update_ble_name(ble_name, length);
        }
    }

    let mut ble_enabled = [0u8; 4];
    let enabled_length = form_value(body, b"ble_enabled", &mut ble_enabled).unwrap_or(0);
    let enabled = enabled_length > 0 && ble_enabled[0] == b'1';
    config::set_ble_enabled(enabled);
    crate::esp_info!(
        "HTTP: configuration saved, wifi_ssid_len={}, ble_enabled={}",
        credentials.ssid_len,
        enabled
    );
    true
}

fn form_value(body: &[u8], key: &[u8], output: &mut [u8]) -> Option<usize> {
    let mut needle = [0u8; 33];
    if key.len() + 1 > needle.len() {
        return None;
    }
    needle[..key.len()].copy_from_slice(key);
    needle[key.len()] = b'=';
    let start = find_bytes(body, &needle)? + needle.len();
    let end = body[start..]
        .iter()
        .position(|byte| *byte == b'&' || *byte == b' ' || *byte == b'\r' || *byte == b'\n')
        .map_or(body.len(), |offset| start + offset);
    let mut output_length = 0;
    let mut index = start;
    while index < end && output_length < output.len() {
        let byte = body[index];
        if byte == b'+' {
            output[output_length] = b' ';
            output_length += 1;
            index += 1;
        } else if byte == b'%' && index + 2 < end {
            let Some(high) = hex(body[index + 1]) else {
                return None;
            };
            let Some(low) = hex(body[index + 2]) else {
                return None;
            };
            output[output_length] = (high << 4) | low;
            output_length += 1;
            index += 3;
        } else {
            output[output_length] = byte;
            output_length += 1;
            index += 1;
        }
    }
    Some(output_length)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn header_value_number(headers: &[u8], key: &[u8]) -> usize {
    let Some(start) = find_bytes(headers, key) else {
        return 0;
    };
    let mut value = 0usize;
    for byte in headers[start + key.len()..].iter().copied() {
        if byte.is_ascii_digit() {
            value = value
                .saturating_mul(10)
                .saturating_add((byte - b'0') as usize);
        } else if value != 0 {
            break;
        }
    }
    value
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
const API_SCAN_ACCEPTED: &[u8] = br#"{"ok":true}"#;

fn wifi_results_json() -> String {
    let snapshot = config::copy_wifi_scan();
    let mut output = String::new();
    let _ = write!(
        &mut output,
        "{{\"state\":{},\"count\":{},\"items\":[",
        snapshot.state, snapshot.count
    );
    for (index, entry) in snapshot.entries[..snapshot.count.min(snapshot.entries.len())]
        .iter()
        .enumerate()
    {
        if index != 0 {
            output.push(',');
        }
        output.push_str("{\"ssid\":");
        write_json_string(
            &mut output,
            &entry.ssid[..entry.ssid_len.min(entry.ssid.len())],
        );
        let _ = write!(
            &mut output,
            ",\"rssi\":{},\"secured\":{}}}",
            entry.signal_strength, entry.secured
        );
    }
    output.push_str("]}");
    output
}

fn ble_results_json() -> String {
    let snapshot = config::copy_ble_scan();
    let mut output = String::new();
    let _ = write!(
        &mut output,
        "{{\"state\":{},\"count\":{},\"items\":[",
        snapshot.state, snapshot.count
    );
    for (index, entry) in snapshot.entries[..snapshot.count.min(snapshot.entries.len())]
        .iter()
        .enumerate()
    {
        if index != 0 {
            output.push(',');
        }
        output.push_str("{\"name\":");
        write_json_string(
            &mut output,
            &entry.name[..entry.name_len.min(entry.name.len())],
        );
        output.push_str(",\"address\":");
        write_json_string(
            &mut output,
            &entry.address_text[..entry.address_len.min(entry.address_text.len())],
        );
        let _ = write!(&mut output, ",\"rssi\":{}}}", entry.signal_strength);
    }
    output.push_str("]}");
    output
}

fn write_json_string(output: &mut String, bytes: &[u8]) {
    output.push('"');
    if let Ok(value) = core::str::from_utf8(bytes) {
        for character in value.chars() {
            match character {
                '"' => output.push_str("\\\""),
                '\\' => output.push_str("\\\\"),
                '\n' => output.push_str("\\n"),
                '\r' => output.push_str("\\r"),
                '\t' => output.push_str("\\t"),
                '\u{0000}'..='\u{001f}' => {
                    let _ = write!(output, "\\u{:04x}", character as u32);
                }
                _ => output.push(character),
            }
        }
    } else {
        for byte in bytes {
            let _ = write!(output, "\\u00{:02x}", byte);
        }
    }
    output.push('"');
}

const PORTAL_HTML: &[u8] = r##"<!doctype html>
<html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>ESP32-S3 配置门户</title>
<style>
body{font-family:system-ui,sans-serif;max-width:680px;margin:0 auto;padding:24px;background:#f4f7fb;color:#14253a}
.card{background:white;border:1px solid #d8e1eb;border-radius:18px;padding:20px;margin:14px 0;box-shadow:0 4px 16px #14253a12}
h1{font-size:24px;margin:0 0 8px}.muted{color:#71839a;font-size:14px}
label{display:block;margin:12px 0 5px;font-weight:600}
input{box-sizing:border-box;width:100%;padding:11px;border:1px solid #b9cadb;border-radius:10px;font-size:16px}
button{margin-top:12px;background:#25415f;color:white;border:0;border-radius:12px;padding:12px 22px;font-size:16px}
button:active{background:#4d7c9f}button.secondary{background:#4d7c9f}
.status{margin-left:10px;color:#71839a;font-size:14px}
.scan-list{max-height:230px;overflow-y:auto;margin-top:12px;padding:4px;border:1px solid #d8e1eb;border-radius:12px;background:#f8fafc}
.scan-row{display:block;width:100%;margin:4px 0;padding:11px 12px;text-align:left;background:#eef4fa;color:#25415f}
.scan-row:hover,.scan-row:active{background:#dbe8f5}
</style>
<h1>ESP32-S3 配置门户</h1><p class="muted">Waveshare ESP32-S3-Touch-LCD-1.85C</p>
<div class="card"><b>当前配置网络</b><p>WiFi 名称：<code>ESP32-S3-配置</code><br>密码：<code>无密码</code><br>设备地址：<code>192.168.4.1</code></p></div>
<div class="card"><h2>附近 WiFi</h2><button type="button" class="secondary" onclick="scanRadio('wifi')">扫描 WiFi</button><span id="wifi-status" class="status">点击开始扫描</span><div id="wifi-list" class="scan-list"></div></div>
<div class="card"><h2>附近蓝牙</h2><button type="button" class="secondary" onclick="scanRadio('ble')">扫描蓝牙</button><span id="ble-status" class="status">点击开始扫描</span><div id="ble-list" class="scan-list"></div></div>
<form class="card" method="post" action="/config"><h2>WiFi 配置</h2>
<label>目标 WiFi 名称</label><input name="wifi_ssid" maxlength="32" placeholder="例如：家庭 WiFi" autocomplete="off">
<label>WiFi 密码（开放网络可留空）</label><input name="wifi_password" type="password" maxlength="64" autocomplete="off">
<h2>蓝牙配置</h2><label>蓝牙广播名称</label><input name="ble_name" maxlength="32" value="ESP32-S3-BLE">
<label><input name="ble_enabled" value="1" type="checkbox" style="width:auto;margin-right:8px">启用蓝牙广播</label>
<button type="submit">保存并应用</button></form>
<div class="card"><b>使用说明</b><p>选择 WiFi 扫描结果可自动填入名称。扫描结果区域支持上下滚动。提交 WiFi 后设备会尝试连接目标网络，蓝牙名称在下一次广播周期使用。</p></div>
<script>
const elements={wifi:{status:document.getElementById('wifi-status'),list:document.getElementById('wifi-list')},ble:{status:document.getElementById('ble-status'),list:document.getElementById('ble-list')}};
async function scanRadio(type){
  const view=elements[type];view.status.textContent='正在请求扫描…';view.list.replaceChildren();
  try{await fetch('/api/'+type+'/scan',{cache:'no-store'});pollRadio(type);}
  catch(_){view.status.textContent='请求失败，请重试';}
}
async function pollRadio(type){
  const view=elements[type];
  try{
    const response=await fetch('/api/'+type+'/results',{cache:'no-store'});
    const data=await response.json();
    renderResults(type,data);
    if(data.state===1||data.state===2){view.status.textContent='正在扫描…';setTimeout(()=>pollRadio(type),500);}
    else if(data.state===3){view.status.textContent='扫描完成：'+data.count+' 个结果';}
    else if(data.state===4){view.status.textContent='扫描失败，请重试';}
    else{view.status.textContent='点击开始扫描';}
  }catch(_){view.status.textContent='读取结果失败';}
}
function renderResults(type,data){
  const view=elements[type];view.list.replaceChildren();
  data.items.forEach(item=>{
    const row=document.createElement('button');row.type='button';row.className='scan-row';
    if(type==='wifi'){
      row.textContent=item.ssid+'  '+item.rssi+' dBm'+(item.secured?' · 加密':' · 开放');
      row.onclick=()=>{document.forms[0].elements.wifi_ssid.value=item.ssid;};
    }else{
      row.textContent=(item.name||item.address)+'  '+item.rssi+' dBm';
    }
    view.list.append(row);
  });
}
</script>
"##.as_bytes();

const CONFIG_SAVED_HTML: &[u8] = r##"<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>配置已保存</title><style>body{font-family:system-ui,sans-serif;max-width:600px;margin:60px auto;padding:24px;background:#f4f7fb;color:#14253a}.card{background:white;border-radius:18px;padding:28px;text-align:center}a{color:#25415f}</style><div class="card"><h1>配置已保存</h1><p>设备已经接受配置，并会在后台尝试连接 WiFi。</p><p><a href="/">返回配置页面</a></p></div>"##.as_bytes();
