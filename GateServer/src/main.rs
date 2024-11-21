use embedded_svc::{http::Method, io::Write};
use esp_idf_hal::{delay::FreeRtos, gpio::*, peripheral::Peripheral, peripherals::Peripherals};
use esp_idf_svc::{
    hal::io::EspIOError,
    http::server::{Configuration, EspHttpServer},
};
use lazy_static::lazy_static;
use log::info;
use parking_lot::Mutex;
use std::sync::Arc;

use crate::wifi::connect_wifi;

pub mod wifi;

// Lazy static peripherals initialization
lazy_static! {
    /// Peripherals
    pub static ref PERIPHERALS: Arc<Mutex<Peripherals>> =
        Arc::new(Mutex::new(Peripherals::take().unwrap()));
    /// Gate open pin
    pub static ref GATE_OPEN: Arc<Mutex<PinDriver<'static, Gpio3, Output>>> = {
        let peripherals = PERIPHERALS.clone();
        let mut peripherals = peripherals.lock();
        let gate_open =
            PinDriver::output(unsafe { peripherals.pins.gpio3.clone_unchecked() }).unwrap();
        Arc::new(Mutex::new(gate_open))
    };
    /// Gate step-by-step (SBS) pin
    /// When opened - then close, When closed - then open, in porgress - stop
    pub static ref GATE_SBS: Arc<Mutex<PinDriver<'static, Gpio10, Output>>> = {
        let peripherals = PERIPHERALS.clone();
        let mut peripherals = peripherals.lock();
        let gate_sbs =
            PinDriver::output(unsafe { peripherals.pins.gpio10.clone_unchecked() }).unwrap();
        Arc::new(Mutex::new(gate_sbs))
    };
    /// Gate opened sensor (active low)
    pub static ref GATE_OPENED: Arc<Mutex<PinDriver<'static, Gpio0, Input>>> = {
        let peripherals = PERIPHERALS.clone();
        let mut peripherals = peripherals.lock();
        let gate_opened =
            PinDriver::input(unsafe { peripherals.pins.gpio0.clone_unchecked() }).unwrap();
        Arc::new(Mutex::new(gate_opened))
    };
    // Gate closed sensor (active low)
    pub static ref GATE_CLOSED: Arc<Mutex<PinDriver<'static, Gpio1, Input>>> = {
        let peripherals = PERIPHERALS.clone();
        let mut peripherals = peripherals.lock();
        let gate_closed =
            PinDriver::input(unsafe { peripherals.pins.gpio1.clone_unchecked() }).unwrap();
        Arc::new(Mutex::new(gate_closed))
    };
}

// WiFi AP credentials
#[toml_cfg::toml_config]
pub struct Config {
    #[default("")]
    wifi_ssid: &'static str,
    #[default("")]
    wifi_psk: &'static str,
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();

    let app_config = CONFIG;
    loop {
        // Reconnect loop, then WiFi connection lost
        'reconnect_loop: {
            let mut wifi = connect_wifi(app_config.wifi_ssid, app_config.wifi_psk).unwrap();
            let mut server = EspHttpServer::new(&Configuration::default())?;
            // Main page handler
            server.fn_handler(
                "/",
                Method::Get,
                |request| -> core::result::Result<(), EspIOError> {
                    info!("Gate main page called");
                    let html = gate_page();
                    let mut response = request.into_ok_response()?;
                    response.write_all(html.as_bytes())?;
                    Ok(())
                },
            )?;
            // Gate status JSON handler
            server.fn_handler(
                "/gate_status",
                Method::Get,
                |request| -> core::result::Result<(), EspIOError> {
                    info!("Gate status called");
                    let html = gate_json_status();
                    let mut response = request.into_ok_response()?;
                    response.write_all(html.as_bytes())?;
                    Ok(())
                },
            )?;
            // Gate SBS command handler
            server.fn_handler(
                "/gate_sbs",
                Method::Get,
                |request| -> core::result::Result<(), EspIOError> {
                    info!("Gate SBS called");
                    let html = gate_sbs();
                    let mut response = request.into_ok_response()?;
                    response.write_all(html.as_bytes())?;
                    Ok(())
                },
            )?;
            // Gate open command handler
            server.fn_handler(
                "/gate_open",
                Method::Get,
                |request| -> core::result::Result<(), EspIOError> {
                    info!("Gate open called");
                    let html = gate_open();
                    let mut response = request.into_ok_response()?;
                    response.write_all(html.as_bytes())?;
                    Ok(())
                },
            )?;
            // Prevent program from exiting
            loop {
                info!("Server awaiting connection");
                FreeRtos::delay_ms(60000);
                if !wifi.driver_mut().is_connected().unwrap() {
                    info!("WiFi connection lost, reconnecting");
                    break 'reconnect_loop;
                }
            }
        }
    }
}
// Gate status
// 0 - opened, 1 - closed, 2 - in middle position
fn gate_status() -> u8 {
    let gate_opened = GATE_OPENED.clone();
    let mut gate_opened = gate_opened.lock();
    gate_opened.set_pull(Pull::Floating).unwrap();
    if gate_opened.is_high() {
        info!("Gate opened");
        0u8
    } else {
        let gate_closed = GATE_CLOSED.clone();
        let mut gate_closed = gate_closed.lock();
        gate_closed.set_pull(Pull::Floating).unwrap();
        if gate_closed.is_high() {
            info!("Gate closed");
            1u8
        } else {
            info!("Gate in middle position");
            2u8
        }
    }
}
// Gate status in JSON
fn gate_json_status() -> String {
    format!("{{\"s\":{}}}", gate_status())
}
// Gate step-by-step (SBS) command handler
fn gate_sbs() -> &'static str {
    let gate_sbs = GATE_SBS.clone();
    let mut gate_sbs = gate_sbs.lock();
    gate_sbs.set_high().unwrap();
    FreeRtos::delay_ms(200);
    gate_sbs.set_low().unwrap();
    "{\"s\":2}"
}
// Gate open command handler
fn gate_open() -> &'static str {
    let gate_open = GATE_OPEN.clone();
    let mut gate_open = gate_open.lock();
    gate_open.set_high().unwrap();
    FreeRtos::delay_ms(200);
    gate_open.set_low().unwrap();
    "{\"s\":2}"
}
// Gate main page constructor
fn gate_page() -> &'static str {
    match gate_status() {
        0 => concat!(
            include_str!("index-0.html"),
            "<h2><div id=\"status\">Открыто</div></h2>",
            "<button id=\"sbs_button\" class=\"button\" onclick=\"sbs_gate()\" autofocus>Закрыть</button>",
            include_str!("index-1.html") ),
        1 => concat!(
            include_str!("index-0.html"),
            "<h2><div id=\"status\">Закрыто</div></h2>",
            "<button id=\"sbs_button\" class=\"button\" onclick=\"sbs_gate()\" autofocus>Открыть</button>",
            include_str!("index-1.html") ),
        _ => concat!(
            include_str!("index-0.html"),
            "<h2><div id=\"status\">Промежуточное положение</div></h2>",
            "<button id=\"sbs_button\" class=\"button\" onclick=\"sbs_gate()\" disabled>Открыть/Закрыть/Стоп</button>",
            include_str!("index-1.html") ),
    }
}
