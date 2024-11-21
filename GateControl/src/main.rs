use embedded_svc::{
    http::client::{Client, Method},
    utils::io,
};
use esp_idf_hal::{delay::FreeRtos, gpio::*, peripheral::Peripheral, peripherals::Peripherals};
use esp_idf_svc::http::client::EspHttpConnection;
use lazy_static::lazy_static;
use log::{error, info};
use parking_lot::Mutex;
use rgb_led::{RGB8, WS2812RMT};
use std::sync::Arc;

use crate::wifi::connect_wifi;

pub mod rgb_led;
pub mod wifi;

// Lazy static peripherals initialization
lazy_static! {
    /// Peripherals
    pub static ref PERIPHERALS: Arc<Mutex<Peripherals>> =
        Arc::new(Mutex::new(Peripherals::take().unwrap()));
    /// Gate step-by-step (SBS) pin
    /// When opened - then close, When closed - then open, in porgress - stop
    pub static ref GATE_SBS: Arc<Mutex<PinDriver<'static, Gpio9, Input>>> = {
        let peripherals = PERIPHERALS.clone();
        let mut peripherals = peripherals.lock();
        let gate_sbs =
            PinDriver::input(unsafe { peripherals.pins.gpio9.clone_unchecked() }).unwrap();
        Arc::new(Mutex::new(gate_sbs))
    };
}

// WiFi AP credentials
#[toml_cfg::toml_config]
pub struct Config {
    #[default("")]
    wifi_ssid: &'static str,
    #[default("")]
    wifi_psk: &'static str,
    #[default(-80)]
    max_rssi: i8,
    #[default("http/192.168.0.1/gate_open")]
    gate_open_url: &'static str,
    #[default("http/192.168.0.1/gate_sbs")]
    gate_sbs_url: &'static str,
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let app_config = CONFIG;
    let peripherals = PERIPHERALS.clone();
    let mut peripherals = peripherals.lock();
    let mut led = WS2812RMT::new(
        unsafe { peripherals.pins.gpio8.clone_unchecked() },
        unsafe { peripherals.rmt.channel0.clone_unchecked() },
    )?;
    drop(peripherals);

    loop {
        // Reconnect loop, then WiFi connection lost
        'reconnect_loop: {
            // Yellow
            led.set_pixel(RGB8::new(50, 50, 0))?;
            let mut wifi = connect_wifi(app_config.wifi_ssid, app_config.wifi_psk).unwrap();
            info!("WiFi connected with rssi {}", wifi.1);
            let mut client = Client::wrap(EspHttpConnection::new(&Default::default())?);
            if wifi.1 < app_config.max_rssi {
                info!("Rssi is low. Opening gate");
                // Red
                led.set_pixel(RGB8::new(50, 0, 0))?;
                let _ = get_request(app_config.gate_open_url, &mut client);
                FreeRtos::delay_ms(1000);
            }

            // Green
            led.set_pixel(RGB8::new(0, 50, 0))?;
            let gate_sbs = GATE_SBS.clone();
            let mut gate_sbs = gate_sbs.lock();
            gate_sbs.set_pull(Pull::Up).unwrap();

            // Poll SBS pin loop
            loop {
                let rssi = wifi.0.driver_mut().get_ap_info().unwrap().signal_strength;
                info!("RSSI: {}", rssi);
                if gate_sbs.is_low() {
                    // Blue
                    led.set_pixel(RGB8::new(0, 0, 50))?;
                    let _ = get_request(app_config.gate_sbs_url, &mut client);
                    // Avoid contact bounce and duplicate sensing
                    FreeRtos::delay_ms(100);
                    while gate_sbs.is_low() {
                        FreeRtos::delay_ms(100);
                    }
                    // Green
                    led.set_pixel(RGB8::new(0, 50, 0))?;
                } else {
                    FreeRtos::delay_ms(100);
                }

                if !wifi.0.driver_mut().is_connected().unwrap() {
                    info!("WiFi connection lost. Pause to avoid wrong reconnection");
                    // Violet
                    led.set_pixel(RGB8::new(50, 0, 50))?;
                    FreeRtos::delay_ms(60000);
                    info!("Reconnecting WiFi");
                    break 'reconnect_loop;
                }
            }
        }
    }
}
/// Send an HTTP GET request.
fn get_request(url: &str, client: &mut Client<EspHttpConnection>) -> anyhow::Result<()> {
    let headers = [("accept", "application/json")];

    // Send request
    let request = client.request(Method::Get, url, &headers)?;
    info!("-> GET {}", url);
    let mut response = request.submit()?;

    // Process response
    let status = response.status();
    info!("<- {}", status);
    let mut buf = [0u8; 64];
    let bytes_read = io::try_read_full(&mut response, &mut buf).map_err(|e| e.0)?;
    info!("Read {} bytes", bytes_read);
    match std::str::from_utf8(&buf[0..bytes_read]) {
        Ok(body_string) => info!(
            "Response body (truncated to {} bytes): {:?}",
            buf.len(),
            body_string
        ),
        Err(e) => error!("Error decoding response body: {}", e),
    };
    Ok(())
}
