//! ESP32-S3 BLE 广播与简单 GATT 服务。

use embassy_executor::Spawner;
use embassy_futures::join::join;
use embassy_time::{Duration, Timer};
use esp_radio::ble::controller::BleConnector;
use log::{info, warn};
use trouble_host::prelude::*;

use crate::features::config;

const CONNECTIONS_MAX: usize = 1;
const L2CAP_CHANNELS_MAX: usize = 2;

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
}

#[embassy_executor::task]
async fn ble_task(bluetooth: esp_hal::peripherals::BT<'static>) {
    let connector = BleConnector::new(bluetooth, Default::default()).expect("BLE 初始化失败");
    let controller: ExternalController<_, 1> = ExternalController::new(connector);
    ble_run(controller).await;
}

async fn ble_run<C>(controller: C)
where
    C: Controller,
{
    let address: Address = Address::random([0xff, 0x8f, 0x1a, 0x05, 0xe4, 0xff]);
    info!("BLE address = {:?}", address);

    let mut resources: HostResources<DefaultPacketPool, CONNECTIONS_MAX, L2CAP_CHANNELS_MAX> =
        HostResources::new();
    let stack = trouble_host::new(controller, &mut resources).set_random_address(address);
    let Host {
        mut peripheral,
        runner,
        ..
    } = stack.build();

    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "ESP32-S3-BLE",
        appearance: &appearance::power_device::GENERIC_POWER_DEVICE,
    }))
    .expect("BLE GATT 服务创建失败");

    let _ = join(ble_runner(runner), async {
        loop {
            let mut name_buffer = [0u8; 32];
            let (name_length, enabled) = config::copy_ble_name(&mut name_buffer);
            if !enabled {
                Timer::after(Duration::from_secs(2)).await;
                continue;
            }
            let configured_name =
                core::str::from_utf8(&name_buffer[..name_length]).unwrap_or("ESP32-S3-BLE");
            let name = truncate_name(configured_name);
            match advertise(name, &mut peripheral, &server).await {
                Ok(connection) => {
                    let _ = gatt_events_task(&server, &connection).await;
                }
                Err(error) => {
                    warn!("BLE 广播失败: {:?}", error);
                    Timer::after(Duration::from_secs(1)).await;
                }
            }
        }
    })
    .await;
}

async fn ble_runner<C: Controller, P: PacketPool>(mut runner: Runner<'_, C, P>) {
    loop {
        if let Err(error) = runner.run().await {
            warn!("BLE 协议栈错误: {:?}", error);
        }
    }
}

async fn gatt_events_task<P: PacketPool>(
    server: &Server<'_>,
    connection: &GattConnection<'_, '_, P>,
) -> Result<(), Error> {
    let level = server.battery_service.level;
    loop {
        match connection.next().await {
            GattConnectionEvent::Disconnected { reason } => {
                info!("BLE 连接断开: {:?}", reason);
                return Ok(());
            }
            GattConnectionEvent::Gatt { event } => {
                if let GattEvent::Read(read) = &event {
                    if read.handle() == level.handle {
                        info!("BLE 读取电量: {:?}", server.get(&level));
                    }
                }
                match event.accept() {
                    Ok(reply) => reply.send().await,
                    Err(error) => warn!("BLE GATT 响应失败: {:?}", error),
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
    let advertiser = peripheral
        .advertise(
            &Default::default(),
            Advertisement::ConnectableScannableUndirected {
                adv_data: &advertiser_data[..length],
                scan_data: &[],
            },
        )
        .await?;
    info!("BLE 正在广播: {}", name);
    Ok(advertiser.accept().await?.with_attribute_server(server)?)
}
