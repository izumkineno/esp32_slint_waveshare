//! 板载 SoftAP 配置门户。
//!
//! 设备启动后同时提供配置 AP 和一个可选的 STA 接口：
//! - 手机连接 `ESP32-S3-配置` / `esp32s3-config`；
//! - 浏览器访问 `http://192.168.4.1/`；
//! - 提交 WiFi 凭据后，设备尝试连接目标网络；
//! - 蓝牙名称由 BLE 广播任务读取，下一次广播周期生效。

use alloc::string::String;
use core::{
    net::Ipv4Addr,
    sync::atomic::{AtomicU8, Ordering},
};

use embassy_executor::Spawner;
use embassy_net::{
    tcp::TcpSocket, IpListenEndpoint, Ipv4Cidr, Runner, Stack, StackResources, StaticConfigV4,
};
use embassy_time::{Duration, Timer};
use embedded_io_async::Write;
use esp_hal::rng::Rng;
use esp_radio::wifi::{
    ap::AccessPointConfig, sta::StationConfig, AuthenticationMethod, Config, ControllerConfig,
    Interface, WifiController,
};
use static_cell::StaticCell;

use crate::features::config::{self, WifiCredentials};

pub const AP_SSID: &str = "ESP32-S3-配置";
pub const AP_PASSWORD: &str = "esp32s3-config";
pub const AP_ADDRESS: Ipv4Addr = Ipv4Addr::new(192, 168, 4, 1);

const WIFI_STATUS_IDLE: u8 = 0;
const WIFI_STATUS_CONNECTING: u8 = 1;
const WIFI_STATUS_CONNECTED: u8 = 2;
const WIFI_STATUS_FAILED: u8 = 3;
static WIFI_STATUS: AtomicU8 = AtomicU8::new(WIFI_STATUS_IDLE);

fn ap_config() -> AccessPointConfig {
    AccessPointConfig::default()
        .with_ssid(AP_SSID)
        .with_auth_method(AuthenticationMethod::Wpa2Personal)
        .with_password(String::from(AP_PASSWORD))
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
        config = config.with_auth_method(AuthenticationMethod::None);
    }
    config
}

macro_rules! mk_static {
    ($t:ty, $value:expr) => {{
        static CELL: StaticCell<$t> = StaticCell::new();
        CELL.uninit().write($value)
    }};
}

pub fn start(spawner: Spawner, wifi: esp_hal::peripherals::WIFI<'static>) {
    let initial_station = StationConfig::default()
        .with_ssid("未配置")
        .with_password(String::from("未配置密码"));
    let initial_config = Config::AccessPointStation(initial_station, ap_config());
    let (controller, interfaces) = esp_radio::wifi::new(
        wifi,
        ControllerConfig::default().with_initial_config(initial_config),
    )
    .expect("WiFi 初始化失败");

    let rng = Rng::new();
    let seed = (rng.random() as u64) << 32 | rng.random() as u64;
    let ap_config = embassy_net::Config::ipv4_static(StaticConfigV4 {
        address: Ipv4Cidr::new(AP_ADDRESS, 24),
        gateway: Some(AP_ADDRESS),
        dns_servers: Default::default(),
    });
    let sta_config = embassy_net::Config::dhcpv4(Default::default());
    let (ap_stack, ap_runner) = embassy_net::new(
        interfaces.access_point,
        ap_config,
        mk_static!(StackResources<3>, StackResources::<3>::new()),
        seed,
    );
    let (_sta_stack, sta_runner) = embassy_net::new(
        interfaces.station,
        sta_config,
        mk_static!(StackResources<4>, StackResources::<4>::new()),
        seed,
    );

    spawner.spawn(wifi_controller(controller).unwrap());
    spawner.spawn(net_task(ap_runner).unwrap());
    spawner.spawn(net_task(sta_runner).unwrap());
    spawner.spawn(run_dhcp(ap_stack).unwrap());
    spawner.spawn(http_server(ap_stack).unwrap());
}

#[embassy_executor::task]
async fn wifi_controller(mut controller: WifiController<'static>) {
    let mut last_credentials = None;
    let mut retry_ticks = 0u8;

    loop {
        if let Some(credentials) = config::take_wifi_command() {
            last_credentials = Some(credentials);
            retry_ticks = 0;
        }

        if let Some(credentials) = last_credentials {
            if !controller.is_connected() && retry_ticks == 0 {
                WIFI_STATUS.store(WIFI_STATUS_CONNECTING, Ordering::Relaxed);
                let config = Config::AccessPointStation(station_config(credentials), ap_config());
                if controller.set_config(&config).is_err() {
                    WIFI_STATUS.store(WIFI_STATUS_FAILED, Ordering::Relaxed);
                    retry_ticks = 20;
                } else {
                    match controller.connect_async().await {
                        Ok(_) => WIFI_STATUS.store(WIFI_STATUS_CONNECTED, Ordering::Relaxed),
                        Err(_) => {
                            WIFI_STATUS.store(WIFI_STATUS_FAILED, Ordering::Relaxed);
                            retry_ticks = 20;
                        }
                    }
                }
            }
            retry_ticks = retry_ticks.saturating_sub(1);
        }

        Timer::after_millis(100).await;
    }
}

#[embassy_executor::task(pool_size = 2)]
async fn net_task(mut runner: Runner<'static, Interface<'static>>) {
    runner.run().await;
}

#[embassy_executor::task]
async fn run_dhcp(stack: Stack<'static>) {
    use core::net::{Ipv4Addr, SocketAddrV4};

    use edge_dhcp::{
        io::{self, DEFAULT_SERVER_PORT},
        server::{Server, ServerOptions},
    };
    use edge_nal::UdpBind;
    use edge_nal_embassy::{Udp, UdpBuffers};

    let mut buffer = [0u8; 1500];
    let mut gateway_buffer = [Ipv4Addr::UNSPECIFIED];
    let udp_buffers = UdpBuffers::<3, 1024, 1024, 10>::new();
    let unbound_socket = Udp::new(stack, &udp_buffers);
    let mut socket = unbound_socket
        .bind(core::net::SocketAddr::V4(SocketAddrV4::new(
            Ipv4Addr::UNSPECIFIED,
            DEFAULT_SERVER_PORT,
        )))
        .await
        .unwrap();

    loop {
        let _ = io::server::run(
            &mut Server::<_, 64>::new_with_et(AP_ADDRESS),
            &ServerOptions::new(AP_ADDRESS, Some(&mut gateway_buffer)),
            &mut socket,
            &mut buffer,
        )
        .await;
        Timer::after_millis(500).await;
    }
}

#[embassy_executor::task]
async fn http_server(stack: Stack<'static>) {
    let mut rx_buffer = [0u8; 4096];
    let mut tx_buffer = [0u8; 2048];
    let mut socket = TcpSocket::new(stack, &mut rx_buffer, &mut tx_buffer);
    socket.set_timeout(Some(Duration::from_secs(10)));

    loop {
        let accepted = socket
            .accept(IpListenEndpoint {
                addr: None,
                port: 80,
            })
            .await;
        if accepted.is_err() {
            socket.abort();
            continue;
        }

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

        let updated = process_request(&request[..length]);
        let body = if updated {
            CONFIG_SAVED_HTML
        } else {
            PORTAL_HTML
        };
        let header = if updated {
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n" as &[u8]
        } else {
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n"
        };
        let _ = socket.write_all(header).await;
        let _ = socket.write_all(body).await;
        let _ = socket.flush().await;
        socket.close();
        Timer::after_millis(20).await;
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

const PORTAL_HTML: &[u8] = r##"<!doctype html>
<html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1">
<title>ESP32-S3 配置门户</title>
<style>
body{font-family:system-ui,sans-serif;max-width:680px;margin:0 auto;padding:24px;background:#f4f7fb;color:#14253a}
.card{background:white;border:1px solid #d8e1eb;border-radius:18px;padding:20px;margin:14px 0;box-shadow:0 4px 16px #14253a12}
h1{font-size:24px;margin:0 0 8px}.muted{color:#71839a;font-size:14px}label{display:block;margin:12px 0 5px;font-weight:600}
input{box-sizing:border-box;width:100%;padding:11px;border:1px solid #b9cadb;border-radius:10px;font-size:16px}button{margin-top:18px;background:#25415f;color:white;border:0;border-radius:12px;padding:12px 22px;font-size:16px}
</style>
<h1>ESP32-S3 配置门户</h1><p class="muted">Waveshare ESP32-S3-Touch-LCD-1.85C</p>
<div class="card"><b>当前配置网络</b><p>WiFi 名称：<code>ESP32-S3-配置</code><br>密码：<code>esp32s3-config</code><br>设备地址：<code>192.168.4.1</code></p></div>
<form class="card" method="post" action="/config"><h2>WiFi 配置</h2>
<label>目标 WiFi 名称</label><input name="wifi_ssid" maxlength="32" placeholder="例如：家庭 WiFi">
<label>WiFi 密码（开放网络可留空）</label><input name="wifi_password" type="password" maxlength="64">
<h2>蓝牙配置</h2><label>蓝牙广播名称</label><input name="ble_name" maxlength="32" value="ESP32-S3-BLE">
<label><input name="ble_enabled" value="1" type="checkbox" checked style="width:auto;margin-right:8px">启用蓝牙广播</label>
<button type="submit">保存并应用</button></form>
<div class="card"><b>使用说明</b><p>提交 WiFi 后设备会尝试连接目标网络。蓝牙名称在下一次广播周期使用。配置当前保存在运行内存中，重新上电后请再次设置。</p></div>
"##.as_bytes();

const CONFIG_SAVED_HTML: &[u8] = r##"<!doctype html><html lang="zh-CN"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>配置已保存</title><style>body{font-family:system-ui,sans-serif;max-width:600px;margin:60px auto;padding:24px;background:#f4f7fb;color:#14253a}.card{background:white;border-radius:18px;padding:28px;text-align:center}a{color:#25415f}</style><div class="card"><h1>配置已保存</h1><p>设备已经接受配置，并会在后台尝试连接 WiFi。</p><p><a href="/">返回配置页面</a></p></div>"##.as_bytes();
