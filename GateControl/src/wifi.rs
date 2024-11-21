use esp_idf_hal::{delay::FreeRtos, peripheral::Peripheral};
use esp_idf_svc::{
    eventloop::EspSystemEventLoop,
    nvs::*,
    wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi},
};

use crate::PERIPHERALS;

pub fn connect_wifi(
    wifi_ssid: &str,
    wifi_psk: &str,
) -> anyhow::Result<(Box<EspWifi<'static>>, i8)> {
    use log::info;

    let mut last_rssi: Option<i8> = None;
    let auth_method = if wifi_psk.is_empty() {
        info!("Wifi password is empty");
        AuthMethod::None
    } else {
        AuthMethod::WPA2Personal
    };

    let _nvs_default_partition: EspNvsPartition<NvsDefault> = EspDefaultNvsPartition::take()?;
    let peripherals = PERIPHERALS.clone();
    let mut peripherals = peripherals.lock();
    let modem = unsafe { peripherals.modem.clone_unchecked() };
    let sysloop = EspSystemEventLoop::take()?;
    let mut esp_wifi = EspWifi::new(modem, sysloop.clone(), None)?;
    let mut wifi = BlockingWifi::wrap(&mut esp_wifi, sysloop)?;
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
    wifi.start()?;
    'wifi_loop: loop {
        let ap_infos = wifi.scan()?;
        let ours = ap_infos.into_iter().find(|a| a.ssid == wifi_ssid);

        let channel = if let Some(ours) = ours {
            info!(
                "Found configured access point {} on channel {} with signal strength {}",
                wifi_ssid, ours.channel, ours.signal_strength
            );
            (ours.channel, ours.signal_strength)
        } else {
            info!(
                "Configured access point {} not found during scanning, delay one seconds and retry",
                wifi_ssid
            );
            last_rssi = None;
            FreeRtos::delay_ms(1000);
            continue 'wifi_loop;
        };

        if last_rssi.is_none() {
            last_rssi = Some(channel.1);
        }

        wifi.set_configuration(&Configuration::Client(ClientConfiguration {
            ssid: wifi_ssid
                .try_into()
                .expect("Could not parse the given SSID into WiFi config"),
            password: wifi_psk
                .try_into()
                .expect("Could not parse the given password into WiFi config"),
            channel: Some(channel.0),
            auth_method,
            ..Default::default()
        }))?;

        info!("Connecting wifi...");
        if wifi.connect() != Ok(()) {
            continue 'wifi_loop;
        }

        info!("Waiting for DHCP lease...");
        if wifi.wait_netif_up() != Ok(()) {
            continue 'wifi_loop;
        }
        info!("Get IP info");
        let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
        info!("Wifi DHCP info: {:?}", ip_info);
        break 'wifi_loop Ok((Box::new(esp_wifi), last_rssi.unwrap()));
    }
}
