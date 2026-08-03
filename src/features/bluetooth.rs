//! ESP32-S3 BLE 广播、扫描和配对服务。

use bt_hci::{cmd::le::LeSetScanParams, controller::ControllerCmdSync};
use embassy_executor::Spawner;
use embassy_futures::{
    join::join,
    select::{select, Either},
};
use embassy_time::{Duration, Timer};
use esp_radio::ble::controller::BleConnector;
use trouble_host::prelude::*;

use crate::features::config;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 3;
const SCAN_DURATION: Duration = Duration::from_secs(5);

#[gatt_server]
struct Server {
    battery_service: BatteryService,
}

#[gatt_service(uuid = service::BATTERY)]
struct BatteryService {
    #[descriptor(uuid = descriptors::VALID_RANGE, read, value = [0, 100])]
    #[descriptor(
        uuid = descriptors::MEASUREMENT_DESCRIPTION,
        name = "battery",
        read,
        value = "ESP32-S3 电池电量",
        type = &'static str
    )]
    #[characteristic(uuid = characteristic::BATTERY_LEVEL, read, notify, value = 100)]
    level: u8,
}

pub fn start(spawner: Spawner, bluetooth: esp_hal::peripherals::BT<'static>) {
    spawner.spawn(ble_task(bluetooth).unwrap());
    crate::esp_info!("BLE: task spawned");
}

#[embassy_executor::task]
async fn ble_task(bluetooth: esp_hal::peripherals::BT<'static>) {
    crate::esp_info!("BLE: initializing controller");
    let connector = match BleConnector::new(bluetooth, Default::default()) {
        Ok(connector) => connector,
        Err(error) => {
            crate::esp_warn!("BLE 初始化失败: {:?}", error);
            config::set_ble_enabled(false);
            config::fail_ble_scan();
            return;
        }
    };
    let controller: ExternalController<_, 1> = ExternalController::new(connector);
    crate::esp_info!("BLE: controller initialized");
    ble_run(controller).await;
}

async fn ble_run<C>(controller: C)
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    crate::esp_info!("BLE address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let mut rng = esp_hal::rng::Trng::try_new().expect("BLE 随机源未初始化");
    let stack = trouble_host::new(controller, &mut resources)
        .set_random_address(address)
        .set_random_generator_seed(&mut rng);
    stack.set_io_capabilities(IoCapabilities::KeyboardOnly);
    let Host {
        central,
        peripheral,
        runner,
        ..
    } = stack.build();
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "ESP32-S3-BLE",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .expect("BLE GATT 服务创建失败");
    crate::esp_info!("BLE: GATT server ready");

    let scanner = Scanner::new(central);
    let scan_handler = BleScanHandler;

    let _ = join(
        ble_runner(runner, &scan_handler),
        ble_roles(peripheral, scanner, &server),
    )
    .await;
}

async fn ble_runner<C, P>(mut runner: Runner<'_, C, P>, scan_handler: &BleScanHandler)
where
    C: Controller,
    P: PacketPool,
{
    loop {
        if let Err(error) = runner.run_with_handler(scan_handler).await {
            crate::esp_warn!("BLE 协议栈错误: {:?}", error);
        }
    }
}

async fn ble_roles<'values, 'server, C>(
    mut peripheral: Peripheral<'values, C, DefaultPacketPool>,
    mut scanner: Scanner<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    loop {
        let mut name_buffer = [0u8; 32];
        let (name_length, enabled) = config::copy_ble_name(&mut name_buffer);
        if !enabled {
            if config::take_ble_scan_request() {
                config::fail_ble_scan();
            }
            if config::take_ble_pair_request().is_some() {
                config::set_ble_pair_state(config::BLE_PAIR_FAILED, 0);
            }
            Timer::after(Duration::from_secs(2)).await;
            continue;
        }

        if config::take_ble_scan_request() {
            scan_devices(&mut scanner).await;
            continue;
        }

        if let Some(request) = config::take_ble_pair_request() {
            let mut central = scanner.into_inner();
            pair_with_device(&mut central, request).await;
            scanner = Scanner::new(central);
            continue;
        }
        let configured_name =
            core::str::from_utf8(&name_buffer[..name_length]).unwrap_or("ESP32-S3-BLE");
        let name = truncate_name(configured_name);
        match advertise(name, &mut peripheral, server).await {
            Ok(connection) => {
                let _ = gatt_events_task(server, &connection).await;
                crate::esp_info!("BLE: GATT connection accepted");
            }
            Err(error) => {
                if config::copy_ble_scan().state != config::BLE_SCAN_REQUESTED {
                    crate::esp_warn!("BLE 广播周期结束: {:?}", error);
                }
                Timer::after_millis(50).await;
            }
        }
    }
}

async fn scan_devices<C>(scanner: &mut Scanner<'_, C, DefaultPacketPool>)
where
    C: Controller + ControllerCmdSync<LeSetScanParams>,
{
    crate::esp_info!("BLE: scan requested");
    let scan_config = ScanConfig {
        // Passive scanning accepts advertisements from devices that do not
        // answer active scan requests.
        active: false,
        filter_accept_list: &[],
        phys: PhySet::M1,
        interval: Duration::from_millis(160),
        window: Duration::from_millis(120),
        timeout: Duration::from_secs(0),
    };

    let session = match scanner.scan(&scan_config).await {
        Ok(session) => session,
        Err(error) => {
            crate::esp_warn!("BLE 扫描启动失败: {:?}", error);
            config::fail_ble_scan();
            return;
        }
    };
    crate::esp_info!(
        "BLE: scan session started for {} seconds",
        SCAN_DURATION.as_secs()
    );

    Timer::after(SCAN_DURATION).await;
    drop(session);
    // Let the runner process the scan-disable command before another request.
    Timer::after_millis(50).await;
    let count = config::copy_ble_scan().count;
    crate::esp_info!("BLE: scan finished with {} result(s)", count);
    config::finish_ble_scan();
}

async fn pair_with_device<C>(
    central: &mut Central<'_, C, DefaultPacketPool>,
    request: config::BlePairRequest,
) where
    C: Controller,
{
    crate::esp_info!("BLE: pairing started");
    config::set_ble_pair_state(config::BLE_PAIR_CONNECTING, 0);
    let targets = [(request.address.kind, &request.address.addr)];
    let connection_config = ConnectConfig {
        connect_params: Default::default(),
        scan_config: ScanConfig {
            filter_accept_list: &targets,
            ..Default::default()
        },
    };

    let connection = match central.connect(&connection_config).await {
        Ok(connection) => connection,
        Err(error) => {
            crate::esp_warn!("BLE 设备连接失败: {:?}", error);
            config::set_ble_pair_state(config::BLE_PAIR_FAILED, 0);
            return;
        }
    };

    if connection.request_security().is_err() {
        crate::esp_warn!("BLE: security request failed");
        config::set_ble_pair_state(config::BLE_PAIR_FAILED, 0);
        return;
    }

    loop {
        match select(connection.next(), wait_for_pair_confirmation()).await {
            Either::First(event) => match event {
                ConnectionEvent::PassKeyInput => {
                    crate::esp_info!("BLE: remote requested passkey input");
                    config::set_ble_pair_state(config::BLE_PAIR_WAITING_INPUT, 0);
                    if connection.pass_key_input(request.passkey).is_err() {
                        crate::esp_warn!("BLE: passkey submission failed");
                        config::set_ble_pair_state(config::BLE_PAIR_FAILED, 0);
                        return;
                    }
                }
                ConnectionEvent::PassKeyDisplay(key) | ConnectionEvent::PassKeyConfirm(key) => {
                    crate::esp_info!("BLE: remote passkey display/confirmation event");
                    config::set_ble_pair_state(config::BLE_PAIR_DISPLAY, key.value());
                }
                ConnectionEvent::PairingComplete { .. } => {
                    config::set_ble_pair_state(config::BLE_PAIR_PAIRED, 0);
                    crate::esp_info!("BLE: pairing completed");
                    return;
                }
                ConnectionEvent::PairingFailed(error) => {
                    crate::esp_warn!("BLE 配对失败: {:?}", error);
                    config::set_ble_pair_state(config::BLE_PAIR_FAILED, 0);
                    return;
                }
                ConnectionEvent::Disconnected { reason } => {
                    crate::esp_warn!("BLE 配对连接断开: {:?}", reason);
                    config::set_ble_pair_state(config::BLE_PAIR_FAILED, 0);
                    return;
                }
                _ => {}
            },
            Either::Second(_) => {
                if connection.pass_key_confirm().is_err() {
                    crate::esp_warn!("BLE: local passkey confirmation failed");
                    config::set_ble_pair_state(config::BLE_PAIR_FAILED, 0);
                    return;
                }
            }
        }
    }
}

async fn wait_for_pair_confirmation() {
    loop {
        if config::take_ble_pair_confirmation() {
            return;
        }
        Timer::after_millis(50).await;
    }
}

struct BleScanHandler;

impl EventHandler for BleScanHandler {
    fn on_adv_reports(&self, mut reports: LeAdvReportsIter<'_>) {
        while let Some(Ok(report)) = reports.next() {
            let address = Address {
                kind: report.addr_kind,
                addr: report.addr,
            };
            let mut name = [0u8; 32];
            let name_length = parse_advertised_name(report.data, &mut name);
            let name = if core::str::from_utf8(&name[..name_length]).is_ok() {
                &name[..name_length]
            } else {
                &[]
            };
            config::store_ble_scan_entry(address, name, report.rssi);
        }
    }
}

fn parse_advertised_name(data: &[u8], output: &mut [u8; 32]) -> usize {
    let mut offset = 0;
    let mut short_name = 0;
    while offset < data.len() {
        let length = data[offset] as usize;
        if length == 0 {
            break;
        }
        let end = offset.saturating_add(1).saturating_add(length);
        if end > data.len() {
            break;
        }
        let kind = data[offset + 1];
        if kind == 0x09 || (kind == 0x08 && short_name == 0) {
            let payload = &data[offset + 2..end];
            let length = payload.len().min(output.len());
            output[..length].copy_from_slice(&payload[..length]);
            if kind == 0x09 {
                return length;
            }
            short_name = length;
        }
        offset = end;
    }
    short_name
}

async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    loop {
        match connection.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                crate::esp_info!("BLE 连接断开: {:?}", reason);
                return Ok(());
            }
            GattConnectionEvent::Gatt { event } => {
                if let GattEvent::Read(read) = &event {
                    if read.handle() == level.handle {
                        crate::esp_info!("BLE 读取电量: {:?}", server.get(&level));
                    }
                }
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(error) => crate::esp_warn!("BLE GATT 响应失败: {:?}", error),
                }
            }
            _ => {}
        }
    }
}

fn truncate_name(name: &str) -> &str {
    let mut length = name.len().min(20);
    while length > 0 && !name.is_char_boundary(length) {
        length -= 1;
    }
    if length == 0 {
        "ESP32-S3-BLE"
    } else {
        &name[..length]
    }
}

async fn advertise<'values, 'server, C: Controller>(
    name: &str,
    peripheral: &mut Peripheral<'values, C, DefaultPacketPool>,
    server: &'server Server<'values>,
) -> Result<GattConnection<'values, 'server, DefaultPacketPool>, BleHostError<C::Error>> {
    let mut advertiser_data = [0; 31];
    let length = AdStructure::encode_slice(
        &[
            AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
            AdStructure::ServiceUuids16(&[[0x0f, 0x18]]),
            AdStructure::CompleteLocalName(name.as_bytes()),
        ],
        &mut advertiser_data,
    )?;
    let parameters = AdvertisementParameters {
        timeout: Some(Duration::from_secs(1)),
        ..Default::default()
    };
    let advertiser = peripheral
        .advertise(
            &parameters,
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..length],
                scan_data: &[],
            },
        )
        .await?;
    crate::esp_info!("BLE 正在广播: {}", name);
    Ok(advertiser.accept().await?.with_attribute_server(server)?)
}
